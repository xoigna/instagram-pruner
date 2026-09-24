use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;

use crate::client::InstagramClient;
use crate::config::DmKind;
use crate::events::{LogLevel, PrunerEvent, RunStats};
use crate::ratelimit::RateLimiter;

pub use crate::config::ContentFlags;

pub mod dates;
mod execute;
mod filters;
mod resolve;
mod scan;

#[cfg(test)]
mod tests;

const MAX_VERIFY_PASSES: u8 = 3;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TargetResolution {
    Posts,
    AllDms,

    Threads(Vec<String>),

    ThreadObjects(Vec<crate::model::DirectThread>),
}

#[derive(Debug, Clone)]
pub struct PrunerConfig {
    pub targets: Vec<TargetResolution>,

    pub content: ContentFlags,
    pub before: Option<String>,
    pub after: Option<String>,
    pub limit: usize,
    pub dry_run: bool,

    pub throttle_ms: u64,

    pub scan_throttle_ms: u64,

    pub concurrency: u32,

    pub hide_threads: bool,

    pub protect_users: Vec<String>,
    pub dm_kind: DmKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemKind {
    #[default]
    Media,
    Dm,

    Like,

    Saved,

    Comment,

    Archived,

    Repost,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Dm => "dm",
            Self::Like => "like",
            Self::Saved => "saved",
            Self::Comment => "comment",
            Self::Archived => "archived",
            Self::Repost => "repost",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlannedDelete {
    pub target_id: String,
    pub target_name: String,
    pub item_id: String,
    pub kind: ItemKind,
    pub preview: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: String,
    pub media_type: Option<String>,
    pub attachments: Vec<crate::history::AttachmentMeta>,
}

impl PlannedDelete {
    pub fn to_record(
        &self,
        outcome: crate::history::DeleteOutcome,
    ) -> crate::history::DeletedRecord {
        crate::history::DeletedRecord {
            deleted_at: chrono::Utc::now(),
            outcome,
            target_id: self.target_id.clone(),
            target_name: self.target_name.clone(),
            item_id: self.item_id.clone(),
            kind: self.kind.as_str().to_string(),
            author_id: self.author_id.clone(),
            author_name: self.author_name.clone(),
            content: self.content.clone(),
            timestamp: self.timestamp.clone(),
            media_type: self.media_type.clone(),
            attachments: self.attachments.clone(),
            preview: self.preview.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Plan {
    pub entries: Vec<PlannedDelete>,
    pub targets_processed: usize,
    pub items_scanned: u64,
    pub items_eligible: u64,

    pub channels_processed: usize,
    pub messages_scanned: u64,
    pub messages_eligible: u64,
}

pub async fn run(
    client: InstagramClient,
    config: PrunerConfig,
    rate_limiter: Arc<RateLimiter>,
    tx: mpsc::Sender<PrunerEvent>,
    cancel: Arc<AtomicBool>,
) -> RunStats {
    rate_limiter.set_user_throttle(std::time::Duration::from_millis(config.scan_throttle_ms));
    rate_limiter.set_parallelism(config.concurrency);

    let before = config
        .before
        .as_deref()
        .and_then(|s| dates::parse_to_datetime(s, dates::DateBound::Before));
    let after = config
        .after
        .as_deref()
        .and_then(|s| dates::parse_to_datetime(s, dates::DateBound::After));

    let scope = match resolve_scope(&client, &config, &tx).await {
        Some(s) => s,
        None => return RunStats::default(),
    };

    let target_count = resolve::target_count(&scope);
    let _ = tx
        .send(PrunerEvent::Started {
            channel_count: target_count,
        })
        .await;
    emit_dry_run_banner(config.dry_run, &tx).await;
    let _ = tx
        .send(PrunerEvent::Log {
            level: LogLevel::Info,
            message: format!(
                "CFG   scan {}ms ×{} parallel · delete {}ms serial · own-only full history · content {}",
                config.scan_throttle_ms,
                config.concurrency,
                config.throttle_ms,
                config.content.label()
            ),
        })
        .await;

    let mut plan = scan_scope(
        &client,
        &scope,
        before,
        after,
        &config,
        scan::ScanLimits::first_pass(config.concurrency),
        &tx,
        &cancel,
    )
    .await;
    finalize_plan(&mut plan);

    rate_limiter.set_user_throttle(std::time::Duration::from_millis(config.throttle_ms));

    rate_limiter.set_parallelism(1);

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut changed_items: HashSet<(String, String)> = HashSet::new();

    let mut stats = if plan.entries.is_empty() {
        let was_cancelled = cancel.load(Ordering::Relaxed);
        if was_cancelled {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Info,
                    message: "scan cancelled — nothing to delete".into(),
                })
                .await;
            let _ = tx.send(PrunerEvent::Cancelled).await;
        }
        RunStats {
            channel_count: target_count,
            channels_processed: plan.channels_processed,
            messages_scanned: plan.messages_scanned,
            messages_deleted: 0,
            dm_messages_deleted: 0,
            messages_skipped: 0,
            messages_failed: 0,
            rate_limit_hits: 0,
            plan_total: 0,
            was_cancelled,
        }
    } else {
        let _ = tx
            .send(PrunerEvent::PlanReady {
                total: plan.entries.len(),
            })
            .await;
        let (s, succeeded) = execute::execute_plan(
            &client,
            plan,
            config.dry_run,
            rate_limiter.clone(),
            &tx,
            &cancel,
        )
        .await;
        changed_items.extend(succeeded.iter().cloned());
        seen.extend(succeeded);
        s
    };

    if cancel.load(Ordering::Relaxed) && !stats.was_cancelled {
        stats.was_cancelled = true;
        let _ = tx.send(PrunerEvent::Cancelled).await;
    }

    let mut verify_clean = true;
    if should_verify_leftovers(&config, &stats) {
        let outcome = verify_leftovers(
            &client,
            &scope,
            &config,
            before,
            after,
            rate_limiter,
            &tx,
            &cancel,
            stats,
            &mut seen,
            &mut changed_items,
        )
        .await;
        stats = outcome.stats;
        verify_clean = outcome.clean;
    } else if stats.plan_total == 0 && !stats.was_cancelled {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Info,
                message: "plan empty — nothing to delete".into(),
            })
            .await;
    }

    let changed_threads: Vec<_> = scope
        .threads
        .iter()
        .filter(|thread| {
            let thread_id = thread.id();
            changed_items
                .iter()
                .any(|(target_id, _)| target_id == &thread_id)
        })
        .cloned()
        .collect();
    hide_threads_if_requested(
        &client,
        &config,
        &changed_threads,
        stats.was_cancelled || !verify_clean,
        stats.dm_messages_deleted,
        &tx,
    )
    .await;

    stats
}

async fn resolve_scope(
    client: &InstagramClient,
    config: &PrunerConfig,
    tx: &mpsc::Sender<PrunerEvent>,
) -> Option<resolve::ResolvedTargets> {
    match resolve::resolve_targets(
        client,
        &config.targets,
        config.content,
        &config.protect_users,
        config.dm_kind,
        tx,
    )
    .await
    {
        Ok(s) => Some(s),
        Err(e) => {
            let _ = tx
                .send(PrunerEvent::Error {
                    message: format!("target resolution failed: {e}"),
                })
                .await;
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn scan_scope(
    client: &InstagramClient,
    scope: &resolve::ResolvedTargets,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    config: &PrunerConfig,
    limits: scan::ScanLimits,
    tx: &mpsc::Sender<PrunerEvent>,
    cancel: &Arc<AtomicBool>,
) -> Plan {
    let mut plan = scan::build_plan(
        client,
        scope,
        before,
        after,
        config.limit,
        limits,
        tx,
        cancel,
    )
    .await;
    plan.targets_processed = plan.channels_processed;
    plan.items_scanned = plan.messages_scanned;
    plan.items_eligible = plan.messages_eligible;
    plan
}

fn should_verify_leftovers(config: &PrunerConfig, stats: &RunStats) -> bool {
    !config.dry_run && config.limit == 0 && !stats.was_cancelled && stats.plan_total > 0
}

struct VerifyOutcome {
    stats: RunStats,
    clean: bool,
}

async fn emit_cancelled_once(stats: &mut RunStats, tx: &mpsc::Sender<PrunerEvent>) {
    if stats.was_cancelled {
        return;
    }
    stats.was_cancelled = true;
    let _ = tx.send(PrunerEvent::Cancelled).await;
}

#[allow(clippy::too_many_arguments)]
async fn verify_leftovers(
    client: &InstagramClient,
    scope: &resolve::ResolvedTargets,
    config: &PrunerConfig,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    rate_limiter: Arc<RateLimiter>,
    tx: &mpsc::Sender<PrunerEvent>,
    cancel: &Arc<AtomicBool>,
    mut stats: RunStats,
    seen: &mut HashSet<(String, String)>,
    changed_items: &mut HashSet<(String, String)>,
) -> VerifyOutcome {
    let mut verify_executes = 0u8;
    loop {
        if cancel.load(Ordering::Relaxed) {
            emit_cancelled_once(&mut stats, tx).await;
            return VerifyOutcome {
                stats,
                clean: false,
            };
        }

        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Info,
                message: format!(
                    "verify: re-scanning for leftovers (pass {}/{MAX_VERIFY_PASSES})",
                    verify_executes + 1
                ),
            })
            .await;

        rate_limiter.set_user_throttle(std::time::Duration::from_millis(config.scan_throttle_ms));
        rate_limiter.set_parallelism(config.concurrency);
        let mut leftover = scan_scope(
            client,
            scope,
            before,
            after,
            config,
            scan::ScanLimits::verify_pass(config.concurrency),
            tx,
            cancel,
        )
        .await;
        finalize_plan(&mut leftover);
        drop_already_seen(&mut leftover, seen);
        stats.messages_scanned += leftover.messages_scanned;
        rate_limiter.set_user_throttle(std::time::Duration::from_millis(config.throttle_ms));
        rate_limiter.set_parallelism(1);

        if cancel.load(Ordering::Relaxed) {
            emit_cancelled_once(&mut stats, tx).await;
            return VerifyOutcome {
                stats,
                clean: false,
            };
        }

        if leftover.entries.is_empty() {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Success,
                    message: if verify_executes == 0 && stats.plan_total == 0 {
                        "plan empty — nothing to delete".into()
                    } else {
                        "verify: no leftovers — deletions confirmed".into()
                    },
                })
                .await;
            return VerifyOutcome { stats, clean: true };
        }

        if verify_executes >= MAX_VERIFY_PASSES {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Warning,
                    message: format!(
                        "verify: still found {} leftover(s) after {MAX_VERIFY_PASSES} passes — stopping",
                        leftover.entries.len()
                    ),
                })
                .await;
            return VerifyOutcome {
                stats,
                clean: false,
            };
        }

        verify_executes += 1;
        stats.plan_total += leftover.entries.len();
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: format!(
                    "verify pass {verify_executes}: {} leftover item(s) still present — deleting",
                    leftover.entries.len()
                ),
            })
            .await;
        let _ = tx
            .send(PrunerEvent::PlanReady {
                total: stats.plan_total,
            })
            .await;

        let (extra, succeeded) = execute::execute_plan(
            client,
            leftover,
            config.dry_run,
            rate_limiter.clone(),
            tx,
            cancel,
        )
        .await;
        changed_items.extend(succeeded.iter().cloned());
        seen.extend(succeeded);
        merge_execute_stats(&mut stats, extra);
        if stats.was_cancelled {
            return VerifyOutcome {
                stats,
                clean: false,
            };
        }
    }
}

fn finalize_plan(plan: &mut Plan) {
    plan.entries.sort_by(|a, b| {
        a.target_id
            .cmp(&b.target_id)
            .then(a.item_id.cmp(&b.item_id))
    });
    plan.entries
        .dedup_by(|a, b| a.target_id == b.target_id && a.item_id == b.item_id);
    plan.targets_processed = plan.channels_processed;
    plan.items_scanned = plan.messages_scanned;
    plan.items_eligible = plan.entries.len() as u64;
    plan.messages_eligible = plan.items_eligible;
}

pub(crate) fn drop_already_seen(plan: &mut Plan, seen: &HashSet<(String, String)>) {
    plan.entries
        .retain(|e| !seen.contains(&(e.target_id.clone(), e.item_id.clone())));
}

fn merge_execute_stats(into: &mut RunStats, extra: RunStats) {
    into.messages_deleted += extra.messages_deleted;
    into.dm_messages_deleted += extra.dm_messages_deleted;
    into.messages_failed += extra.messages_failed;
    into.messages_skipped += extra.messages_skipped;
    into.rate_limit_hits += extra.rate_limit_hits;
    into.channels_processed = into
        .channels_processed
        .saturating_add(extra.channels_processed);
    if extra.was_cancelled {
        into.was_cancelled = true;
    }
}

async fn emit_dry_run_banner(dry_run: bool, tx: &mpsc::Sender<PrunerEvent>) {
    if dry_run {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: "DRY RUN — no content will be deleted".into(),
            })
            .await;
    }
}

async fn hide_threads_if_requested(
    client: &InstagramClient,
    config: &PrunerConfig,
    threads: &[crate::model::DirectThread],
    was_cancelled: bool,
    dm_messages_deleted: u64,
    tx: &mpsc::Sender<PrunerEvent>,
) {
    if !should_hide_threads(config) || was_cancelled {
        return;
    }
    if dm_messages_deleted == 0 {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Info,
                message: "hide_threads: skipped — no DM messages were deleted".into(),
            })
            .await;
        return;
    }
    if threads.is_empty() {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Info,
                message: "hide_threads: no threads changed — nothing to hide".into(),
            })
            .await;
        return;
    }
    execute::hide_resolved_threads(client, threads, config.dry_run, tx).await;
}

fn should_hide_threads(config: &PrunerConfig) -> bool {
    config.hide_threads
        && config.targets.iter().any(|target| {
            matches!(
                target,
                TargetResolution::AllDms
                    | TargetResolution::Threads(_)
                    | TargetResolution::ThreadObjects(_)
            )
        })
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn only_dm_scopes_can_hide_threads() {
        let mut config = PrunerConfig {
            targets: vec![TargetResolution::Posts],
            content: ContentFlags::default(),
            before: None,
            after: None,
            limit: 0,
            dry_run: false,
            throttle_ms: 2_000,
            scan_throttle_ms: 350,
            concurrency: 4,
            hide_threads: true,
            protect_users: Vec::new(),
            dm_kind: DmKind::All,
        };
        assert!(!should_hide_threads(&config));

        config.targets = vec![TargetResolution::AllDms];
        assert!(should_hide_threads(&config));

        config.hide_threads = false;
        assert!(!should_hide_threads(&config));
    }
}
