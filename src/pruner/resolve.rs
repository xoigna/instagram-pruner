use std::collections::HashSet;

use tokio::sync::mpsc;

use crate::client::{self, InstagramClient};
use crate::config::DmKind;
use crate::error::IgResult;
use crate::events::{LogLevel, PrunerEvent};
use crate::model::DirectThread;

use super::filters::{first_protected_user, thread_matches_kind};
use super::TargetResolution;

const MAX_INBOX_THREADS: usize = 2_000;

#[derive(Debug, Default)]
pub(crate) struct ResolvedTargets {
    pub posts: bool,
    pub threads: Vec<DirectThread>,

    pub content: super::ContentFlags,
}

pub(crate) async fn resolve_targets(
    client: &InstagramClient,
    targets: &[TargetResolution],
    content: super::ContentFlags,
    protect_users: &[String],
    dm_kind: DmKind,
    tx: &mpsc::Sender<PrunerEvent>,
) -> IgResult<ResolvedTargets> {
    let protect: HashSet<&str> = protect_users.iter().map(|s| s.as_str()).collect();
    let mut out = ResolvedTargets {
        content,
        ..ResolvedTargets::default()
    };
    let mut seen_thread_ids: HashSet<String> = HashSet::new();
    let mut skipped_protected = 0usize;
    let mut skipped_protected_names: Vec<String> = Vec::new();
    let mut skipped_kind = 0usize;

    for target in targets {
        match target {
            TargetResolution::Posts => {
                out.posts = true;
            }
            TargetResolution::AllDms => {
                let threads = client::list_all_threads(client, MAX_INBOX_THREADS).await?;
                if threads.len() >= MAX_INBOX_THREADS {
                    let _ = tx
                        .send(PrunerEvent::Log {
                            level: LogLevel::Warning,
                            message: format!(
                                "inbox walk capped at {MAX_INBOX_THREADS} threads — remaining not scanned"
                            ),
                        })
                        .await;
                }
                for t in threads {
                    if !thread_matches_kind(&t, dm_kind) {
                        skipped_kind += 1;
                        continue;
                    }
                    if let Some(name) = first_protected_user(&t, &protect) {
                        skipped_protected += 1;
                        skipped_protected_names.push(name);
                        continue;
                    }
                    let id = t.id();
                    if id.is_empty() || !seen_thread_ids.insert(id) {
                        continue;
                    }
                    out.threads.push(t);
                }
            }
            TargetResolution::Threads(ids) => {
                for id in ids {
                    let id = id.trim();
                    if id.is_empty() || seen_thread_ids.contains(id) {
                        continue;
                    }
                    let _ = tx
                        .send(PrunerEvent::Log {
                            level: LogLevel::Info,
                            message: format!("loading thread {id}…"),
                        })
                        .await;
                    let thread = match client.direct_thread(id, 50, None).await {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = tx
                                .send(PrunerEvent::Log {
                                    level: LogLevel::Warning,
                                    message: format!("thread {id}: {e}"),
                                })
                                .await;
                            continue;
                        }
                    };
                    if let Some(name) = first_protected_user(&thread, &protect) {
                        skipped_protected += 1;
                        skipped_protected_names.push(name);
                        continue;
                    }
                    seen_thread_ids.insert(thread.id());
                    out.threads.push(thread);
                }
            }
            TargetResolution::ThreadObjects(threads) => {
                for thread in threads {
                    let id = thread.id();
                    if id.is_empty() || !seen_thread_ids.insert(id) {
                        continue;
                    }
                    if let Some(name) = first_protected_user(thread, &protect) {
                        skipped_protected += 1;
                        skipped_protected_names.push(name);
                        continue;
                    }
                    out.threads.push(thread.clone());
                }
            }
        }
    }

    if skipped_kind > 0 {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Info,
                message: format!(
                    "skipped {skipped_kind} thread(s) not matching dm_kind={}",
                    dm_kind.label()
                ),
            })
            .await;
    }

    if skipped_protected > 0 {
        skipped_protected_names.sort();
        skipped_protected_names.dedup();
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: format!(
                    "skipped {skipped_protected} protected thread(s): {}",
                    skipped_protected_names.join(", ")
                ),
            })
            .await;
    }

    Ok(out)
}

pub(crate) fn target_count(resolved: &ResolvedTargets) -> usize {
    resolved.threads.len() + usize::from(resolved.posts) + resolved.content.count()
}
