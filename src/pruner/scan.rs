#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use tokio::sync::mpsc;

use crate::client::InstagramClient;
use crate::error::IgError;
use crate::error::IgResult;
use crate::events::{ChannelStats, LogLevel, Phase, PrunerEvent};
use crate::history::{text_preview, AttachmentMeta};
use crate::model::{Comment, DirectMessage, DirectThread, Media};
use crate::util::truncate;

use super::filters::{should_delete_dm, should_delete_media};
use super::resolve::ResolvedTargets;
use super::{ItemKind, Plan, PlannedDelete};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanLimits {
    pub max_pages: u32,
    pub emit_channel_events: bool,
    pub page_size: u32,

    pub concurrency: u32,
}

impl ScanLimits {
    pub fn first_pass(concurrency: u32) -> Self {
        Self {
            max_pages: 500,
            emit_channel_events: true,

            page_size: 33,
            concurrency: concurrency.max(1),
        }
    }

    pub fn verify_pass(concurrency: u32) -> Self {
        Self {
            max_pages: 500,
            emit_channel_events: false,
            page_size: 33,
            concurrency: concurrency.max(1),
        }
    }

    pub fn dm_page_size(self) -> u32 {
        50
    }
}

pub(crate) async fn build_plan(
    client: &InstagramClient,
    scope: &ResolvedTargets,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limit: usize,
    limits: ScanLimits,
    tx: &mpsc::Sender<PrunerEvent>,
    cancel: &Arc<AtomicBool>,
) -> Plan {
    let mut plan = Plan::default();
    let total = super::resolve::target_count(scope);
    let mut index = 0usize;

    if scope.posts {
        if cancel.load(Ordering::Relaxed) {
            return plan;
        }
        if limits.emit_channel_events {
            let _ = tx
                .send(PrunerEvent::ChannelStarted {
                    channel_id: "posts".into(),
                    channel_name: "Your posts".into(),
                    index,
                    total,
                    phase: Phase::Scan,
                })
                .await;
        }
        let (scanned, added, failed, entries) =
            scan_posts(client, before, after, limit, limits, cancel, tx).await;
        plan.entries.extend(entries);
        plan.messages_scanned += scanned;
        plan.messages_eligible += added;
        plan.channels_processed += 1;
        if limits.emit_channel_events {
            let _ = tx
                .send(PrunerEvent::ChannelDone {
                    channel_id: "posts".into(),
                    stats: ChannelStats {
                        scanned,
                        deleted: added,
                        skipped: scanned.saturating_sub(added),
                        failed,
                    },
                    phase: Phase::Scan,
                })
                .await;
        }
        index += 1;
        if limit > 0 && plan.entries.len() >= limit {
            plan.entries.truncate(limit);
            plan.messages_eligible = plan.entries.len() as u64;
            return plan;
        }
    }

    if limit == 0 && scope.threads.len() > 1 {
        scan_threads_parallel(
            client,
            &scope.threads,
            before,
            after,
            limits,
            total,
            index,
            &mut plan,
            cancel,
            tx,
        )
        .await;
    } else {
        for thread in &scope.threads {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if limit > 0 && plan.entries.len() >= limit {
                break;
            }
            let tid = thread.id();
            let tname = thread.display_name();
            if limits.emit_channel_events {
                let _ = tx
                    .send(PrunerEvent::ChannelStarted {
                        channel_id: tid.clone(),
                        channel_name: tname.clone(),
                        index,
                        total,
                        phase: Phase::Scan,
                    })
                    .await;
            }
            let remaining = if limit > 0 {
                limit.saturating_sub(plan.entries.len())
            } else {
                0
            };
            let (scanned, added, failed, entries) =
                scan_thread(client, thread, before, after, remaining, limits, cancel, tx).await;
            plan.entries.extend(entries);
            plan.messages_scanned += scanned;
            plan.messages_eligible += added;
            plan.channels_processed += 1;
            if limits.emit_channel_events {
                let _ = tx
                    .send(PrunerEvent::ChannelDone {
                        channel_id: tid,
                        stats: ChannelStats {
                            scanned,
                            deleted: added,
                            skipped: scanned.saturating_sub(added),
                            failed,
                        },
                        phase: Phase::Scan,
                    })
                    .await;
            }
            index += 1;
        }
    }

    let content = scope.content;
    let mut passes: Vec<ContentPass> = Vec::new();
    if content.likes {
        passes.push(ContentPass::Feed(ActivityFeed::Likes));
    }
    if content.saved {
        passes.push(ContentPass::Feed(ActivityFeed::Saved));
    }
    if content.comments {
        passes.push(ContentPass::Comments);
    }
    if content.archived {
        passes.push(ContentPass::Feed(ActivityFeed::Archived));
    }
    if content.reposts {
        passes.push(ContentPass::Reposts);
    }

    let index_base = index;

    let mut results: Vec<(usize, u64, u64, u64, Vec<PlannedDelete>)> = Vec::new();
    if limit > 0 {
        for (pass_offset, pass) in passes.into_iter().enumerate() {
            if cancel.load(Ordering::Relaxed) || plan.entries.len() >= limit {
                break;
            }
            let pass_index = index_base + pass_offset;
            let remaining = limit.saturating_sub(plan.entries.len());
            results.push(
                run_content_pass(
                    pass, pass_index, remaining, total, limits, client, before, after, cancel, tx,
                )
                .await,
            );
        }
    } else {
        let pass_fanout = passes.len().min(limits.concurrency as usize).max(1);
        results = stream::iter(passes)
            .enumerate()
            .map(|(pass_offset, pass)| {
                let pass_index = index_base + pass_offset;
                run_content_pass(
                    pass, pass_index, 0, total, limits, client, before, after, cancel, tx,
                )
            })
            .buffer_unordered(pass_fanout)
            .collect()
            .await;
    }

    results.sort_by_key(|(pass_index, ..)| *pass_index);
    for (_pass_index, scanned, added, _failed, entries) in results {
        plan.entries.extend(entries);
        plan.messages_scanned += scanned;
        plan.messages_eligible += added;
        plan.channels_processed += 1;
    }

    if limit > 0 && plan.entries.len() > limit {
        plan.entries.truncate(limit);
        plan.messages_eligible = plan.entries.len() as u64;
    }
    plan
}

#[allow(clippy::too_many_arguments)]
async fn run_content_pass(
    pass: ContentPass,
    pass_index: usize,
    limit: usize,
    total: usize,
    limits: ScanLimits,
    client: &InstagramClient,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) -> (usize, u64, u64, u64, Vec<PlannedDelete>) {
    if cancel.load(Ordering::Relaxed) {
        return (pass_index, 0, 0, 0, Vec::new());
    }
    if limits.emit_channel_events {
        let _ = tx
            .send(PrunerEvent::ChannelStarted {
                channel_id: pass.channel_id().into(),
                channel_name: pass.channel_name().into(),
                index: pass_index,
                total,
                phase: Phase::Scan,
            })
            .await;
    }
    let (scanned, added, failed, entries) = pass
        .scan(client, before, after, limit, limits, cancel, tx)
        .await;
    if limits.emit_channel_events {
        let _ = tx
            .send(PrunerEvent::ChannelDone {
                channel_id: pass.channel_id().into(),
                stats: ChannelStats {
                    scanned,
                    deleted: added,
                    skipped: scanned.saturating_sub(added),
                    failed,
                },
                phase: Phase::Scan,
            })
            .await;
    }
    (pass_index, scanned, added, failed, entries)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentPass {
    Feed(ActivityFeed),

    Comments,

    Reposts,
}

impl ContentPass {
    fn channel_id(self) -> &'static str {
        match self {
            Self::Feed(feed) => feed.channel_id(),
            Self::Comments => "comments",
            Self::Reposts => "reposts",
        }
    }

    fn channel_name(self) -> &'static str {
        match self {
            Self::Feed(feed) => feed.channel_name(),
            Self::Comments => "Own comments",
            Self::Reposts => "Reposts",
        }
    }

    async fn scan(
        self,
        client: &InstagramClient,
        before: Option<DateTime<Utc>>,
        after: Option<DateTime<Utc>>,
        limit: usize,
        limits: ScanLimits,
        cancel: &Arc<AtomicBool>,
        tx: &mpsc::Sender<PrunerEvent>,
    ) -> (u64, u64, u64, Vec<PlannedDelete>) {
        match self {
            Self::Feed(feed) => {
                scan_activity_feed(client, feed, before, after, limit, limits, cancel, tx).await
            }
            Self::Comments => scan_comments(client, before, after, limit, limits, cancel, tx).await,
            Self::Reposts => scan_reposts(client, before, after, limit, limits, cancel, tx).await,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivityFeed {
    Likes,
    Saved,
    Archived,
}

impl ActivityFeed {
    fn channel_id(self) -> &'static str {
        match self {
            Self::Likes => "likes",
            Self::Saved => "saved",
            Self::Archived => "archived",
        }
    }

    fn channel_name(self) -> &'static str {
        match self {
            Self::Likes => "Liked posts",
            Self::Saved => "Saved posts",
            Self::Archived => "Archived posts",
        }
    }

    fn item_kind(self) -> ItemKind {
        match self {
            Self::Likes => ItemKind::Like,
            Self::Saved => ItemKind::Saved,
            Self::Archived => ItemKind::Archived,
        }
    }

    async fn page(
        self,
        client: &InstagramClient,
        cursor: Option<&str>,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        match self {
            Self::Likes => client.liked_medias_page(cursor).await,
            Self::Saved => client.saved_medias_page(cursor).await,
            Self::Archived => client.archived_medias_page(cursor).await,
        }
    }
}

fn undo_item_id(feed: ActivityFeed, media: &Media, self_uid: &str) -> String {
    match feed {
        ActivityFeed::Likes => strip_mccr(&media.media_id()),
        ActivityFeed::Saved => media.pk.clone(),
        ActivityFeed::Archived => {
            let full = media.media_id();
            if full.contains('_') || self_uid.is_empty() {
                full
            } else {
                format!("{full}_{self_uid}")
            }
        }
    }
}

fn strip_mccr(id: &str) -> String {
    match id.rsplit_once("_mccr") {
        Some((head, marker))
            if !head.is_empty()
                && !marker.is_empty()
                && marker.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            head.to_string()
        }
        _ => id.to_string(),
    }
}

async fn scan_activity_feed(
    client: &InstagramClient,
    feed: ActivityFeed,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limit: usize,
    limits: ScanLimits,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) -> (u64, u64, u64, Vec<PlannedDelete>) {
    let mut scanned = 0u64;
    let mut added = 0u64;
    let mut failed = 0u64;
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0u32;
    let uid = client.user_id();
    let uname = client.username();

    while pages < limits.max_pages {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if limit > 0 && entries.len() >= limit {
            break;
        }
        pages += 1;
        let (items, next) = match feed.page(client, cursor.as_deref()).await {
            Ok(p) => p,
            Err(IgError::Cancelled) => break,
            Err(e) => {
                failed += 1;
                let _ = tx
                    .send(PrunerEvent::Log {
                        level: LogLevel::Warning,
                        message: format!("{} page failed: {e}", feed.channel_id()),
                    })
                    .await;
                break;
            }
        };
        if items.is_empty() {
            break;
        }
        for media in &items {
            scanned += 1;

            if !super::filters::in_date_window(
                super::dates::media_taken_at(media.taken_at),
                before,
                after,
            ) {
                continue;
            }
            let mut entry = PlannedDelete::from_media(media, &uname);
            entry.kind = feed.item_kind();
            entry.target_id = feed.channel_id().to_string();
            entry.target_name = feed.channel_name().to_string();
            entry.item_id = undo_item_id(feed, media, &uid);
            if entry.item_id.is_empty() {
                continue;
            }
            entry.author_name = media.author_username();
            entry.author_id = media.author_id();
            entries.push(entry);
            added += 1;
            if limit > 0 && entries.len() >= limit {
                break;
            }
        }
        if limits.emit_channel_events {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Info,
                    message: format!(
                        "···  {}  p{pages}  {scanned} seen · {added} to undo",
                        feed.channel_id()
                    ),
                })
                .await;
            let _ = tx
                .send(PrunerEvent::ScanProgress {
                    channel_id: feed.channel_id().to_string(),
                    scanned,
                    eligible: added,
                })
                .await;
        }
        match next {
            Some(n) if !n.is_empty() && Some(n.clone()) != cursor => cursor = Some(n),
            _ => break,
        }
    }
    if pages >= limits.max_pages {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: format!(
                    "{}: hit page cap ({pages}/{}) — older items may remain",
                    feed.channel_id(),
                    limits.max_pages
                ),
            })
            .await;
    }
    (scanned, added, failed, entries)
}

async fn scan_comments(
    client: &InstagramClient,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limit: usize,
    limits: ScanLimits,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) -> (u64, u64, u64, Vec<PlannedDelete>) {
    let mut scanned = 0u64;
    let mut added = 0u64;
    let mut failed = 0u64;
    let mut entries = Vec::new();
    let uid = client.user_id();
    let mut feed_cursor: Option<String> = None;
    let mut feed_pages = 0u32;

    let fanout = if limit > 0 {
        1
    } else {
        limits.concurrency.max(1) as usize
    };

    'posts: while feed_pages < limits.max_pages {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if limit > 0 && entries.len() >= limit {
            break;
        }
        feed_pages += 1;
        let (posts, next) = match client
            .user_medias_page(&uid, feed_cursor.as_deref(), limits.page_size)
            .await
        {
            Ok(p) => p,
            Err(IgError::Cancelled) => break,
            Err(e) => {
                failed += 1;
                let _ = tx
                    .send(PrunerEvent::Log {
                        level: LogLevel::Warning,
                        message: format!("comments: own feed page failed: {e}"),
                    })
                    .await;
                break;
            }
        };
        if posts.is_empty() {
            break;
        }

        if fanout > 1 {
            let results: Vec<(u64, u64, u64, Vec<PlannedDelete>)> = stream::iter(posts)
                .map(|media| {
                    let tx = tx.clone();
                    let cancel = cancel.clone();
                    async move {
                        if cancel.load(Ordering::Relaxed) {
                            return (0, 0, 0, Vec::new());
                        }
                        scan_post_comments(
                            client, &media, limit, before, after, limits, &cancel, &tx,
                        )
                        .await
                    }
                })
                .buffer_unordered(fanout)
                .collect()
                .await;
            for (s, a, f, batch) in results {
                scanned += s;
                added += a;
                failed += f;
                entries.extend(batch);
            }
            if cancel.load(Ordering::Relaxed) {
                break;
            }
        } else {
            for media in &posts {
                if cancel.load(Ordering::Relaxed) {
                    break 'posts;
                }
                if limit > 0 && entries.len() >= limit {
                    break 'posts;
                }
                let (s, a, f, batch) =
                    scan_post_comments(client, media, limit, before, after, limits, cancel, tx)
                        .await;
                scanned += s;
                added += a;
                failed += f;
                entries.extend(batch);
                if limit > 0 && entries.len() >= limit {
                    break 'posts;
                }
            }
        }

        if limits.emit_channel_events {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Info,
                    message: format!(
                        "···  comments  feed p{feed_pages}  {scanned} seen · {added} own"
                    ),
                })
                .await;
            let _ = tx
                .send(PrunerEvent::ScanProgress {
                    channel_id: "comments".into(),
                    scanned,
                    eligible: added,
                })
                .await;
        }
        match next {
            Some(n) if !n.is_empty() && Some(n.clone()) != feed_cursor => feed_cursor = Some(n),
            _ => break,
        }
    }
    (scanned, added, failed, entries)
}

async fn scan_post_comments(
    client: &InstagramClient,
    media: &Media,
    limit: usize,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limits: ScanLimits,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) -> (u64, u64, u64, Vec<PlannedDelete>) {
    let mut scanned = 0u64;
    let mut added = 0u64;
    let mut failed = 0u64;
    let mut entries = Vec::new();
    let uid = client.user_id();
    let uname = client.username();
    let media_id = media.media_id();
    let mut cursor: Option<String> = None;
    let mut pages = 0u32;
    loop {
        if cancel.load(Ordering::Relaxed) || pages >= limits.max_pages {
            break;
        }
        pages += 1;
        let (comments, next) = match client
            .media_comments_page(&media_id, cursor.as_deref())
            .await
        {
            Ok(p) => p,
            Err(IgError::Cancelled) => break,
            Err(e) => {
                failed += 1;
                let _ = tx
                    .send(PrunerEvent::Log {
                        level: LogLevel::Warning,
                        message: format!("comments on {media_id}: {e}"),
                    })
                    .await;
                break;
            }
        };
        if comments.is_empty() {
            break;
        }
        for comment in &comments {
            scanned += 1;
            if !super::filters::should_delete_comment(comment, &uid, &media_id, before, after) {
                continue;
            }
            entries.push(PlannedDelete::from_comment(comment, media, &uname));
            added += 1;
            if limit > 0 && entries.len() >= limit {
                break;
            }
        }
        if limit > 0 && entries.len() >= limit {
            break;
        }
        match next {
            Some(n) if !n.is_empty() && Some(n.clone()) != cursor => cursor = Some(n),
            _ => break,
        }
    }
    (scanned, added, failed, entries)
}

async fn scan_reposts(
    client: &InstagramClient,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limit: usize,
    limits: ScanLimits,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) -> (u64, u64, u64, Vec<PlannedDelete>) {
    let collections = match client.collections().await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Warning,
                    message: format!("reposts: collections/list/ failed: {e}"),
                })
                .await;
            return (0, 0, 1, Vec::new());
        }
    };

    let repost_cols: Vec<_> = collections
        .iter()
        .filter(|c| {
            let name = c.collection_name.to_ascii_lowercase();
            let ty = c
                .collection_type
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            name.contains("repost") || name.contains("reshare") || ty.contains("repost")
        })
        .collect();

    if repost_cols.is_empty() {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: format!(
                    "reposts: no repost collection on this account ({} collection(s) seen: {}) — \
                     Instagram exposes no repost endpoint, so nothing can be undone",
                    collections.len(),
                    collections
                        .iter()
                        .map(|c| c.collection_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
            .await;
        return (0, 0, 0, Vec::new());
    }

    let mut scanned = 0u64;
    let mut added = 0u64;
    let mut failed = 0u64;
    let mut entries = Vec::new();
    let uname = client.username();

    for collection in repost_cols {
        let mut cursor: Option<String> = None;
        let mut pages = 0u32;
        while pages < limits.max_pages {
            if cancel.load(Ordering::Relaxed) || (limit > 0 && entries.len() >= limit) {
                break;
            }
            pages += 1;
            let (items, next) = match client
                .collection_medias_page(&collection.collection_id, cursor.as_deref())
                .await
            {
                Ok(p) => p,
                Err(IgError::Cancelled) => break,
                Err(e) => {
                    failed += 1;
                    let _ = tx
                        .send(PrunerEvent::Log {
                            level: LogLevel::Warning,
                            message: format!(
                                "reposts: collection {} failed: {e}",
                                collection.collection_id
                            ),
                        })
                        .await;
                    break;
                }
            };
            if items.is_empty() {
                break;
            }
            for media in &items {
                scanned += 1;
                if !super::filters::in_date_window(
                    super::dates::media_taken_at(media.taken_at),
                    before,
                    after,
                ) {
                    continue;
                }
                let mut entry = PlannedDelete::from_media(media, &uname);
                entry.kind = ItemKind::Repost;

                entry.target_id = collection.collection_id.clone();
                entry.target_name = format!("Reposts ({})", collection.collection_name);
                entry.item_id = media.pk.clone();
                if entry.item_id.is_empty() {
                    continue;
                }
                entries.push(entry);
                added += 1;
                if limit > 0 && entries.len() >= limit {
                    break;
                }
            }
            match next {
                Some(n) if !n.is_empty() && Some(n.clone()) != cursor => cursor = Some(n),
                _ => break,
            }
        }
    }
    (scanned, added, failed, entries)
}

async fn scan_threads_parallel(
    client: &InstagramClient,
    threads: &[DirectThread],
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limits: ScanLimits,
    total: usize,
    index_base: usize,
    plan: &mut Plan,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) {
    let _ = tx
        .send(PrunerEvent::Log {
            level: LogLevel::Info,
            message: format!(
                "SCAN  {} DM thread(s) ×{} parallel · full history · own only",
                threads.len(),
                limits.concurrency
            ),
        })
        .await;

    let jobs: Vec<(usize, DirectThread)> = threads
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, t)| (index_base + i, t))
        .collect();

    let results: Vec<(usize, String, String, u64, u64, u64, Vec<PlannedDelete>)> =
        stream::iter(jobs)
            .map(|(index, thread)| {
                let tx = tx.clone();
                let cancel = cancel.clone();
                async move {
                    let tid = thread.id();
                    let tname = thread.display_name();
                    if limits.emit_channel_events {
                        let _ = tx
                            .send(PrunerEvent::ChannelStarted {
                                channel_id: tid.clone(),
                                channel_name: tname.clone(),
                                index,
                                total,
                                phase: Phase::Scan,
                            })
                            .await;
                    }
                    let (scanned, added, failed, entries) =
                        scan_thread(client, &thread, before, after, 0, limits, &cancel, &tx).await;
                    if limits.emit_channel_events {
                        let _ = tx
                            .send(PrunerEvent::ChannelDone {
                                channel_id: tid.clone(),
                                stats: ChannelStats {
                                    scanned,
                                    deleted: added,
                                    skipped: scanned.saturating_sub(added),
                                    failed,
                                },
                                phase: Phase::Scan,
                            })
                            .await;
                    }
                    (index, tid, tname, scanned, added, failed, entries)
                }
            })
            .buffer_unordered(limits.concurrency as usize)
            .collect()
            .await;

    let mut results = results;
    results.sort_by_key(|(index, ..)| *index);
    for (_index, _tid, _tname, scanned, added, _failed, entries) in results {
        plan.entries.extend(entries);
        plan.messages_scanned += scanned;
        plan.messages_eligible += added;
        plan.channels_processed += 1;
    }
}

async fn scan_posts(
    client: &InstagramClient,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limit: usize,
    limits: ScanLimits,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) -> (u64, u64, u64, Vec<PlannedDelete>) {
    let mut scanned = 0u64;
    let mut added = 0u64;
    let mut failed = 0u64;
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0u32;
    let mut past_after = false;
    let uid = client.user_id();
    let uname = client.username();

    while pages < limits.max_pages {
        if cancel.load(Ordering::Relaxed) || past_after {
            break;
        }
        if limit > 0 && entries.len() >= limit {
            break;
        }
        pages += 1;
        let page = match client
            .user_medias_page(&uid, cursor.as_deref(), limits.page_size)
            .await
        {
            Ok(p) => p,
            Err(IgError::Cancelled) => break,
            Err(e) => {
                failed += 1;
                let _ = tx
                    .send(PrunerEvent::Log {
                        level: LogLevel::Warning,
                        message: format!("posts page failed: {e}"),
                    })
                    .await;
                break;
            }
        };
        let (items, next) = page;
        if items.is_empty() {
            break;
        }

        let mut page_all_old = after.is_some();
        let mut saw_timestamp = false;
        for media in &items {
            scanned += 1;
            if let Some(t) = super::dates::media_taken_at(media.taken_at) {
                saw_timestamp = true;
                if let Some(a) = after {
                    if t > a {
                        page_all_old = false;
                    }
                }
            } else {
                page_all_old = false;
            }
            if !should_delete_media(media, &uid, before, after) {
                continue;
            }
            entries.push(PlannedDelete::from_media(media, &uname));
            added += 1;
            if limit > 0 && entries.len() >= limit {
                break;
            }
        }
        if after.is_some() && page_all_old && saw_timestamp {
            past_after = true;
        }
        if limits.emit_channel_events {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Info,
                    message: format!("···  posts  p{pages}  {scanned} seen · {added} own"),
                })
                .await;
            let _ = tx
                .send(PrunerEvent::ScanProgress {
                    channel_id: "posts".into(),
                    scanned,
                    eligible: added,
                })
                .await;
        }
        match next {
            Some(n) if !n.is_empty() && n != cursor.clone().unwrap_or_default() => {
                cursor = Some(n);
            }
            _ => break,
        }
    }
    if pages >= limits.max_pages {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: format!(
                    "posts: hit page cap ({pages}/{}) — older media may remain unscanned",
                    limits.max_pages
                ),
            })
            .await;
    }
    (scanned, added, failed, entries)
}

async fn scan_thread(
    client: &InstagramClient,
    thread: &DirectThread,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    remaining_limit: usize,
    limits: ScanLimits,
    cancel: &Arc<AtomicBool>,
    tx: &mpsc::Sender<PrunerEvent>,
) -> (u64, u64, u64, Vec<PlannedDelete>) {
    let mut scanned = 0u64;
    let mut added = 0u64;
    let mut failed = 0u64;
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0u32;
    let tid = thread.id();
    let tname = thread.display_name();
    let mut past_after = false;
    let mut seen_ids: HashSet<String> = HashSet::new();

    if !thread.items.is_empty() {
        let (s, a, stop_after, batch) = absorb_dm_page(
            client,
            &tid,
            &tname,
            &thread.items,
            before,
            after,
            remaining_limit,
            &mut seen_ids,
        );
        scanned += s;
        added += a;
        entries.extend(batch);
        if stop_after {
            past_after = true;
        }
        if limits.emit_channel_events && scanned > 0 {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Info,
                    message: format!(
                        "···  {}  preview  {scanned} seen · {added} own",
                        crate::util::truncate(&tname, 28)
                    ),
                })
                .await;
            let _ = tx
                .send(PrunerEvent::ScanProgress {
                    channel_id: tid.clone(),
                    scanned,
                    eligible: added,
                })
                .await;
        }
        if remaining_limit > 0 && added as usize >= remaining_limit {
            return (scanned, added, failed, entries);
        }
        if !thread.has_older || past_after {
            return (scanned, added, failed, entries);
        }
        cursor = thread
            .oldest_cursor
            .clone()
            .or_else(|| thread.prev_cursor.clone())
            .filter(|s| !s.is_empty());
        if cursor.is_none() {}
    }

    while pages < limits.max_pages {
        if cancel.load(Ordering::Relaxed) || past_after {
            break;
        }
        if remaining_limit > 0 && added as usize >= remaining_limit {
            break;
        }
        pages += 1;

        let page_thread = match client
            .direct_thread(&tid, limits.dm_page_size(), cursor.as_deref())
            .await
        {
            Ok(t) => t,
            Err(IgError::Cancelled) => break,
            Err(e) => {
                failed += 1;
                let _ = tx
                    .send(PrunerEvent::Log {
                        level: LogLevel::Warning,
                        message: format!("thread {tid} page failed: {e}"),
                    })
                    .await;
                break;
            }
        };

        if page_thread.items.is_empty() {
            break;
        }

        let (s, a, stop_after, batch) = absorb_dm_page(
            client,
            &tid,
            &tname,
            &page_thread.items,
            before,
            after,
            remaining_limit.saturating_sub(added as usize),
            &mut seen_ids,
        );
        scanned += s;
        added += a;
        entries.extend(batch);
        if stop_after {
            past_after = true;
        }
        if limits.emit_channel_events {
            let _ = tx
                .send(PrunerEvent::Log {
                    level: LogLevel::Info,
                    message: format!(
                        "···  {}  p{pages}  {scanned} seen · {added} own",
                        crate::util::truncate(&tname, 28)
                    ),
                })
                .await;
            let _ = tx
                .send(PrunerEvent::ScanProgress {
                    channel_id: tid.clone(),
                    scanned,
                    eligible: added,
                })
                .await;
        }
        if remaining_limit > 0 && added as usize >= remaining_limit {
            break;
        }
        if !page_thread.has_older {
            break;
        }
        let next = page_thread
            .oldest_cursor
            .or(page_thread.prev_cursor)
            .filter(|s| !s.is_empty());
        if next.is_none() || next == cursor {
            break;
        }
        cursor = next;
    }
    if pages >= limits.max_pages {
        let _ = tx
            .send(PrunerEvent::Log {
                level: LogLevel::Warning,
                message: format!(
                    "thread {tid}: hit page cap ({pages}/{}) — older own messages may remain",
                    limits.max_pages
                ),
            })
            .await;
    }
    (scanned, added, failed, entries)
}

fn absorb_dm_page(
    client: &InstagramClient,
    tid: &str,
    tname: &str,
    items: &[DirectMessage],
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    remaining_limit: usize,
    seen_ids: &mut HashSet<String>,
) -> (u64, u64, bool, Vec<PlannedDelete>) {
    let mut scanned = 0u64;
    let mut added = 0u64;
    let mut out = Vec::new();
    let mut page_all_old = after.is_some();
    let mut saw_timestamp = false;
    let uid = client.user_id();
    let uname = client.username();

    for msg in items {
        scanned += 1;
        if let Some(t) = super::dates::dm_timestamp_us(msg.timestamp) {
            saw_timestamp = true;
            if let Some(a) = after {
                if t > a {
                    page_all_old = false;
                }
            }
        } else {
            page_all_old = false;
        }

        if !should_delete_dm(msg, &uid, before, after) {
            continue;
        }
        let mid = msg.message_id();
        if !seen_ids.insert(mid.clone()) {
            continue;
        }
        out.push(PlannedDelete::from_dm(msg, tid, tname, &uid, &uname));
        added += 1;
        if remaining_limit > 0 && added as usize >= remaining_limit {
            break;
        }
    }
    let stop_after = after.is_some() && page_all_old && saw_timestamp;
    (scanned, added, stop_after, out)
}

impl PlannedDelete {
    pub fn from_media(media: &Media, self_username: &str) -> Self {
        let caption = media.caption_text();
        let kind_label = media.kind_label();
        let preview = text_preview(&caption, kind_label);
        let ts = super::dates::media_taken_at(media.taken_at)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        let mut attachments = Vec::new();
        if let Some(url) = media.preview_url() {
            attachments.push(AttachmentMeta {
                id: media.pk.clone(),
                filename: format!(
                    "{}.{}",
                    media.code.clone().unwrap_or_else(|| media.pk.clone()),
                    ext_for(kind_label)
                ),
                url: url.clone(),
                proxy_url: String::new(),
                size: 0,
                content_type: kind_label.to_string(),
            });
        }
        Self {
            target_id: "posts".into(),
            target_name: "Your posts".into(),
            item_id: media.media_id(),
            kind: ItemKind::Media,
            preview,
            author_id: media.author_id(),
            author_name: if self_username.is_empty() {
                media.author_username()
            } else {
                self_username.to_string()
            },
            content: caption,
            timestamp: ts,
            media_type: Some(kind_label.to_string()),
            attachments,
        }
    }

    pub fn from_dm(
        msg: &DirectMessage,
        thread_id: &str,
        thread_name: &str,
        self_id: &str,
        self_username: &str,
    ) -> Self {
        let content = msg.text.clone().unwrap_or_default();
        let kind_label = humanize_item_type(msg.item_type.as_deref());
        let preview = text_preview(&content, &kind_label);
        Self {
            target_id: thread_id.to_string(),
            target_name: thread_name.to_string(),
            item_id: msg.message_id(),
            kind: ItemKind::Dm,
            preview: truncate(&preview, 80),
            author_id: self_id.to_string(),
            author_name: self_username.to_string(),
            content,
            timestamp: msg.timestamp_rfc3339(),
            media_type: Some(kind_label),
            attachments: Vec::new(),
        }
    }

    pub fn from_comment(comment: &Comment, media: &Media, self_username: &str) -> Self {
        let content = comment.text.clone();
        let preview = text_preview(&content, "comment");
        let ts = super::dates::media_taken_at(comment.created_at)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        Self {
            target_id: media.media_id(),
            target_name: format!("Comments on {}", media.kind_label()),
            item_id: comment.pk.clone(),
            kind: ItemKind::Comment,
            preview: truncate(&preview, 80),
            author_id: comment.author_id(),
            author_name: if self_username.is_empty() {
                comment.author_username()
            } else {
                self_username.to_string()
            },
            content,
            timestamp: ts,
            media_type: Some("comment".into()),
            attachments: Vec::new(),
        }
    }
}

fn ext_for(kind: &str) -> &'static str {
    match kind {
        "video" | "reel" | "igtv" => "mp4",
        _ => "jpg",
    }
}

fn humanize_item_type(item_type: Option<&str>) -> String {
    match item_type.map(str::trim).filter(|s| !s.is_empty()) {
        None => "message".into(),
        Some("text") => "text".into(),
        Some("media") => "media".into(),
        Some("raven_media") => "view-once".into(),
        Some("voice_media") => "voice note".into(),
        Some("animated_media") => "gif".into(),
        Some("like") => "heart".into(),
        Some("link") => "link".into(),
        Some("reel_share" | "clip" | "xma_clip") => "reel".into(),
        Some("story_share" | "xma_story_share") => "story share".into(),
        Some("media_share" | "xma_media_share" | "felix_share") => "shared post".into(),
        Some("profile" | "xma_profile") => "profile share".into(),
        Some("location") => "location".into(),
        Some("action_log") => "system activity".into(),
        Some("video_call_event" | "voice_call_event" | "call" | "video_call") => {
            "call event".into()
        }
        Some("placeholder") => "placeholder".into(),
        Some(other) if other.ends_with("_log") => "system activity".into(),
        Some(other) => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_mccr_normalises_cache_marked_like_ids() {
        assert_eq!(
            strip_mccr("3474265637340066486_5515149943_mccr40091ae1"),
            "3474265637340066486_5515149943"
        );

        assert_eq!(
            strip_mccr("3992123904827341000_331150405"),
            "3992123904827341000_331150405"
        );
    }

    #[test]
    fn strip_mccr_leaves_non_marker_ids_untouched() {
        assert_eq!(strip_mccr("111_222"), "111_222");
        assert_eq!(strip_mccr(""), "");

        assert_eq!(strip_mccr("111_222_mccr"), "111_222_mccr");

        assert_eq!(strip_mccr("111_222_mccrZZZ"), "111_222_mccrZZZ");

        assert_eq!(strip_mccr("3954879423574379060"), "3954879423574379060");
    }
}
