use std::collections::BTreeSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::util::truncate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteOutcome {
    Deleted,

    AlreadyGone,

    Forbidden,

    DryRun,
}

#[allow(dead_code)]
impl DeleteOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deleted => "deleted",
            Self::AlreadyGone => "already_gone",
            Self::Forbidden => "forbidden",
            Self::DryRun => "dry_run",
        }
    }

    pub fn log_prefix(self) -> &'static str {
        match self {
            Self::Deleted => "DEL",
            Self::AlreadyGone => "GONE",
            Self::Forbidden => "SKIP",
            Self::DryRun => "WOULD-DEL",
        }
    }

    pub fn bucket(self) -> HistoryBucket {
        match self {
            Self::DryRun => HistoryBucket::DryRun,
            Self::Deleted | Self::AlreadyGone | Self::Forbidden => HistoryBucket::Live,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryBucket {
    Live,
    DryRun,
}

impl HistoryBucket {
    pub const ALL: [Self; 2] = [Self::Live, Self::DryRun];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::DryRun => "dry-run",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryScope {
    #[default]
    All,
    Live,
    DryRun,
}

#[allow(dead_code)]
impl HistoryScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Live => "live",
            Self::DryRun => "dry-run",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Live,
            Self::Live => Self::DryRun,
            Self::DryRun => Self::All,
        }
    }

    fn buckets(self) -> &'static [HistoryBucket] {
        match self {
            Self::All => &HistoryBucket::ALL,
            Self::Live => &[HistoryBucket::Live],
            Self::DryRun => &[HistoryBucket::DryRun],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRef {
    pub id: String,
    pub name: String,

    pub file_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentMeta {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub content_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedRecord {
    pub deleted_at: DateTime<Utc>,
    pub outcome: DeleteOutcome,

    pub target_id: String,
    pub target_name: String,

    pub item_id: String,

    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub author_id: String,
    #[serde(default)]
    pub author_name: String,

    #[serde(default)]
    pub content: String,

    #[serde(default)]
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentMeta>,

    #[serde(default)]
    pub preview: String,
}

#[allow(dead_code)]
impl DeletedRecord {
    pub fn list_line(&self) -> String {
        let time = self.deleted_at.format("%H:%M:%S");
        format!(
            "{time} {} {}  {}",
            self.outcome.log_prefix(),
            truncate(&self.target_name, 18),
            truncate(&self.preview, 48)
        )
    }

    pub fn detail_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("deleted_at: {}\n", self.deleted_at.to_rfc3339()));
        out.push_str(&format!("outcome:    {}\n", self.outcome.as_str()));
        out.push_str(&format!("bucket:     {}\n", self.outcome.bucket().as_str()));
        out.push_str(&format!(
            "target:     {} ({})\n",
            self.target_name, self.target_id
        ));
        out.push_str(&format!("item:       {}\n", self.item_id));
        out.push_str(&format!("kind:       {}\n", self.kind));
        out.push_str(&format!(
            "author:     {} ({})\n",
            self.author_name, self.author_id
        ));
        out.push_str(&format!("timestamp:  {}\n", self.timestamp));
        if let Some(mt) = &self.media_type {
            out.push_str(&format!("media_type: {mt}\n"));
        }
        out.push_str("--- content ---\n");
        out.push_str(&self.content);
        out.push('\n');
        if !self.attachments.is_empty() {
            out.push_str("--- attachments ---\n");
            for a in &self.attachments {
                let url = if a.url.is_empty() {
                    a.proxy_url.as_str()
                } else {
                    a.url.as_str()
                };
                out.push_str(&format!(
                    "- {} ({}) {} bytes  {}\n",
                    a.filename, a.content_type, a.size, url
                ));
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct HistoryStore {
    root: PathBuf,
}

#[allow(dead_code)]
impl HistoryStore {
    pub fn from_env() -> Self {
        let root = match env::var("PRUNER_HISTORY_DIR") {
            Ok(s) if !s.trim().is_empty() => PathBuf::from(s),
            _ => PathBuf::from("history"),
        };
        Self { root }
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn day_rel_path(&self, bucket: HistoryBucket, day: NaiveDate) -> String {
        format!(
            "{}/{}/{:02}/{:02}",
            bucket.as_str(),
            day.year(),
            day.month(),
            day.day()
        )
    }

    fn day_dir(&self, bucket: HistoryBucket, day: NaiveDate) -> PathBuf {
        self.root
            .join(bucket.as_str())
            .join(format!("{:04}", day.year()))
            .join(format!("{:02}", day.month()))
            .join(format!("{:02}", day.day()))
    }

    fn day_messages_path(&self, bucket: HistoryBucket, day: NaiveDate) -> PathBuf {
        self.day_dir(bucket, day).join("messages.jsonl")
    }

    fn target_file_name(target_name: &str, target_id: &str) -> String {
        format!(
            "{}__{}.jsonl",
            sanitize_component(target_name),
            sanitize_component(target_id)
        )
    }

    fn target_path(
        &self,
        bucket: HistoryBucket,
        day: NaiveDate,
        record: &DeletedRecord,
    ) -> PathBuf {
        self.day_dir(bucket, day)
            .join("channels")
            .join(Self::target_file_name(
                &record.target_name,
                &record.target_id,
            ))
    }

    fn legacy_day_path(&self, day: NaiveDate) -> PathBuf {
        self.root.join(format!("{}.jsonl", day.format("%Y-%m-%d")))
    }

    pub fn append(&self, record: &DeletedRecord) -> std::io::Result<()> {
        let bucket = record.outcome.bucket();
        let day = record.deleted_at.date_naive();
        let day_dir = self.day_dir(bucket, day);
        fs::create_dir_all(day_dir.join("channels"))?;

        append_jsonl(&self.day_messages_path(bucket, day), record)?;
        append_jsonl(&self.target_path(bucket, day, record), record)?;
        Ok(())
    }

    pub fn list_days(&self, scope: HistoryScope) -> std::io::Result<Vec<NaiveDate>> {
        let mut days: BTreeSet<NaiveDate> = BTreeSet::new();

        for bucket in scope.buckets() {
            let bucket_root = self.root.join(bucket.as_str());
            if bucket_root.is_dir() {
                collect_days_under(&bucket_root, &mut days)?;
            }
        }

        if matches!(scope, HistoryScope::All | HistoryScope::Live) && self.root.is_dir() {
            for entry in fs::read_dir(&self.root)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Some(stem) = name.strip_suffix(".jsonl") else {
                    continue;
                };
                if let Ok(day) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
                    days.insert(day);
                }
            }
        }

        let mut out: Vec<NaiveDate> = days.into_iter().collect();
        out.sort_unstable_by(|a, b| b.cmp(a));
        Ok(out)
    }

    pub fn list_channels(
        &self,
        day: NaiveDate,
        scope: HistoryScope,
    ) -> std::io::Result<Vec<ChannelRef>> {
        let mut by_id: std::collections::BTreeMap<String, ChannelRef> =
            std::collections::BTreeMap::new();

        for bucket in scope.buckets() {
            let channels_dir = self.day_dir(*bucket, day).join("channels");
            if !channels_dir.is_dir() {
                continue;
            }
            for entry in fs::read_dir(&channels_dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Some(stem) = name.strip_suffix(".jsonl") else {
                    continue;
                };
                if let Some((label, id)) = stem.rsplit_once("__") {
                    by_id.entry(id.to_string()).or_insert(ChannelRef {
                        id: id.to_string(),
                        name: label.replace('_', " "),
                        file_name: name.to_string(),
                    });
                }
            }
        }

        if by_id.is_empty() {
            for rec in self.load_day(day, scope, None)? {
                by_id.entry(rec.target_id.clone()).or_insert(ChannelRef {
                    id: rec.target_id.clone(),
                    name: rec.target_name.clone(),
                    file_name: String::new(),
                });
            }
        }

        Ok(by_id.into_values().collect())
    }

    pub fn load_day(
        &self,
        day: NaiveDate,
        scope: HistoryScope,
        target_id: Option<&str>,
    ) -> std::io::Result<Vec<DeletedRecord>> {
        let mut out = Vec::new();

        for bucket in scope.buckets() {
            if let Some(tid) = target_id {
                let channels_dir = self.day_dir(*bucket, day).join("channels");
                if channels_dir.is_dir() {
                    let mut found = false;
                    for entry in fs::read_dir(&channels_dir)? {
                        let entry = entry?;
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if name.ends_with(&format!("__{tid}.jsonl")) {
                            out.extend(read_jsonl_file(&entry.path())?);
                            found = true;
                            break;
                        }
                    }
                    if found {
                        continue;
                    }
                }
            }

            let path = self.day_messages_path(*bucket, day);
            if path.exists() {
                let mut recs = read_jsonl_file(&path)?;
                if let Some(tid) = target_id {
                    recs.retain(|r| r.target_id == tid);
                }
                out.extend(recs);
            }
        }

        if matches!(scope, HistoryScope::All | HistoryScope::Live) {
            let legacy = self.legacy_day_path(day);
            if legacy.exists() {
                let mut recs = read_jsonl_file(&legacy)?;
                if matches!(scope, HistoryScope::Live) {
                    recs.retain(|r| r.outcome != DeleteOutcome::DryRun);
                }
                if let Some(tid) = target_id {
                    recs.retain(|r| r.target_id == tid);
                }
                out.extend(recs);
            }
        }

        out.sort_by(|a, b| {
            a.deleted_at
                .cmp(&b.deleted_at)
                .then_with(|| a.item_id.cmp(&b.item_id))
        });
        out.dedup_by(|a, b| {
            a.item_id == b.item_id
                && a.target_id == b.target_id
                && a.outcome == b.outcome
                && a.deleted_at == b.deleted_at
        });
        Ok(out)
    }

    pub fn load_recent(
        &self,
        limit: usize,
        scope: HistoryScope,
    ) -> std::io::Result<Vec<DeletedRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut collected = Vec::new();
        for day in self.list_days(scope)? {
            let mut day_recs = self.load_day(day, scope, None)?;
            day_recs.reverse();
            for r in day_recs {
                collected.push(r);
                if collected.len() >= limit {
                    return Ok(collected);
                }
            }
        }
        Ok(collected)
    }
}

fn append_jsonl(path: &Path, record: &DeletedRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, record).map_err(json_io_err)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_jsonl_file(path: &Path) -> std::io::Result<Vec<DeletedRecord>> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<DeletedRecord>(line) {
            Ok(r) => out.push(r),
            Err(e) => tracing::warn!("skipping corrupt history line in {}: {e}", path.display()),
        }
    }
    Ok(out)
}

fn collect_days_under(bucket_root: &Path, days: &mut BTreeSet<NaiveDate>) -> std::io::Result<()> {
    for year_ent in fs::read_dir(bucket_root)? {
        let year_ent = year_ent?;
        if !year_ent.file_type()?.is_dir() {
            continue;
        }
        let year_name = year_ent.file_name();
        let year_str = year_name.to_string_lossy();
        let Ok(year) = year_str.parse::<i32>() else {
            continue;
        };
        for month_ent in fs::read_dir(year_ent.path())? {
            let month_ent = month_ent?;
            if !month_ent.file_type()?.is_dir() {
                continue;
            }
            let month_name = month_ent.file_name();
            let month_str = month_name.to_string_lossy();
            let Ok(month) = month_str.parse::<u32>() else {
                continue;
            };
            for day_ent in fs::read_dir(month_ent.path())? {
                let day_ent = day_ent?;
                if !day_ent.file_type()?.is_dir() {
                    continue;
                }
                let day_name = day_ent.file_name();
                let day_str = day_name.to_string_lossy();
                let Ok(day_n) = day_str.parse::<u32>() else {
                    continue;
                };
                let Some(day) = NaiveDate::from_ymd_opt(year, month, day_n) else {
                    continue;
                };
                let path = day_ent.path();
                let has_data =
                    path.join("messages.jsonl").exists() || path.join("channels").is_dir();
                if has_data {
                    days.insert(day);
                }
            }
        }
    }
    Ok(())
}

fn sanitize_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(48));
    for c in s.chars().take(48) {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if c == '-' || c == '.' {
            out.push(c);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        } else if out.is_empty() && c.is_whitespace() {
        } else if out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "target".into()
    } else {
        out
    }
}

fn json_io_err(e: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}

pub fn text_preview(text: &str, fallback: &str) -> String {
    let content = text.trim();
    if !content.is_empty() {
        return truncate(content, 60);
    }
    truncate(fallback, 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch() -> (Tmp, HistoryStore) {
        let dir = std::env::temp_dir().join(format!("pruner-hist-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let store = HistoryStore::with_root(&dir);
        (Tmp(dir), store)
    }

    fn sample_record(id: &str, outcome: DeleteOutcome, target: &str) -> DeletedRecord {
        DeletedRecord {
            deleted_at: Utc::now(),
            outcome,
            target_id: target.into(),
            target_name: format!("name-{target}"),
            item_id: id.into(),
            kind: "dm".into(),
            author_id: "me".into(),
            author_name: "me".into(),
            content: format!("hello {id}"),
            timestamp: "2024-01-01T00:00:00+00:00".into(),
            media_type: None,
            attachments: vec![],
            preview: format!("hello {id}"),
        }
    }

    #[test]
    fn append_writes_bucket_day_and_target_files() {
        let (tmp, store) = scratch();
        let r = sample_record("m1", DeleteOutcome::Deleted, "c1");
        store.append(&r).unwrap();
        let day = r.deleted_at.date_naive();
        let day_dir = tmp.0.join(format!(
            "live/{}/{:02}/{:02}",
            day.year(),
            day.month(),
            day.day()
        ));
        assert!(day_dir.join("messages.jsonl").is_file());
        let channels: Vec<_> = fs::read_dir(day_dir.join("channels"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(channels.len(), 1);
        assert!(channels[0].ends_with("__c1.jsonl"));
    }

    #[test]
    fn dry_run_goes_under_dry_run_bucket() {
        let (tmp, store) = scratch();
        let r = sample_record("m1", DeleteOutcome::DryRun, "c1");
        store.append(&r).unwrap();
        let day = r.deleted_at.date_naive();
        assert!(tmp
            .0
            .join(format!(
                "dry-run/{}/{:02}/{:02}/messages.jsonl",
                day.year(),
                day.month(),
                day.day()
            ))
            .is_file());
    }

    #[test]
    fn append_and_load_roundtrip() {
        let (_tmp, store) = scratch();
        let r1 = sample_record("m1", DeleteOutcome::Deleted, "c1");
        let r2 = sample_record("m2", DeleteOutcome::AlreadyGone, "c1");
        store.append(&r1).unwrap();
        store.append(&r2).unwrap();
        let day = r1.deleted_at.date_naive();
        let loaded = store.load_day(day, HistoryScope::Live, None).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].item_id, "m1");
        assert_eq!(loaded[1].item_id, "m2");
    }

    #[test]
    fn load_day_filters_by_target() {
        let (_tmp, store) = scratch();
        store
            .append(&sample_record("a", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("b", DeleteOutcome::Deleted, "c2"))
            .unwrap();
        let day = Utc::now().date_naive();
        let only_c1 = store.load_day(day, HistoryScope::All, Some("c1")).unwrap();
        assert_eq!(only_c1.len(), 1);
        assert_eq!(only_c1[0].item_id, "a");
    }

    #[test]
    fn list_days_and_scope_separation() {
        let (_tmp, store) = scratch();
        store
            .append(&sample_record("a", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("b", DeleteOutcome::DryRun, "c1"))
            .unwrap();
        assert_eq!(store.list_days(HistoryScope::Live).unwrap().len(), 1);
        assert_eq!(store.list_days(HistoryScope::DryRun).unwrap().len(), 1);
        assert_eq!(store.list_days(HistoryScope::All).unwrap().len(), 1);
    }

    #[test]
    fn load_recent_newest_first() {
        let (_tmp, store) = scratch();
        store
            .append(&sample_record("a", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("b", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("c", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        let recent = store.load_recent(2, HistoryScope::All).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].item_id, "c");
        assert_eq!(recent[1].item_id, "b");
    }

    #[test]
    fn list_channels_from_shards() {
        let (_tmp, store) = scratch();
        store
            .append(&sample_record("a", DeleteOutcome::Deleted, "c1"))
            .unwrap();
        store
            .append(&sample_record("b", DeleteOutcome::Deleted, "c2"))
            .unwrap();
        let day = Utc::now().date_naive();
        let ch = store.list_channels(day, HistoryScope::Live).unwrap();
        assert_eq!(ch.len(), 2);
        assert!(ch.iter().any(|c| c.id == "c1"));
        assert!(ch.iter().any(|c| c.id == "c2"));
    }

    #[test]
    fn sanitize_component_strips_junk() {
        assert_eq!(sanitize_component("Alice / DM"), "alice_dm");
        assert_eq!(sanitize_component("!!!"), "target");
        assert_eq!(sanitize_component("General"), "general");
    }

    #[test]
    fn text_preview_truncates() {
        let p = text_preview(&"a".repeat(100), "fallback");
        assert!(p.chars().count() <= 61);
        assert!(p.ends_with('\u{2026}'));
    }

    #[test]
    fn text_preview_uses_fallback() {
        assert_eq!(text_preview("  ", "photo"), "photo");
    }
}
