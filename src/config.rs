use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub sessionid: String,

    pub base_url: String,

    pub session_path: String,
    pub scope: ScopeMode,
    pub scope_form: ScopeForm,
    pub before: String,
    pub after: String,
    pub limit: String,
    pub throttle_ms: String,

    pub scan_throttle_ms: String,

    pub concurrency: String,
    pub dry_run: bool,

    pub hide_threads: bool,

    pub content: ContentFlags,

    pub protect_users: Vec<String>,

    pub dm_kind: DmKind,

    pub history_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeMode {
    None,

    Posts,

    Dms,

    All,

    Threads(String),
}

#[allow(dead_code)]
impl ScopeMode {
    pub fn kind(&self) -> ScopeKind {
        match self {
            Self::None => ScopeKind::None,
            Self::Posts => ScopeKind::Posts,
            Self::Dms => ScopeKind::Dms,
            Self::All => ScopeKind::All,
            Self::Threads(_) => ScopeKind::Threads,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeKind {
    None,
    #[default]
    Posts,
    Dms,
    All,
    Threads,
}

#[allow(dead_code)]
impl ScopeKind {
    pub const ALL: [ScopeKind; 5] = [
        ScopeKind::None,
        ScopeKind::Posts,
        ScopeKind::Dms,
        ScopeKind::All,
        ScopeKind::Threads,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ScopeKind::None => "none (content only)",
            ScopeKind::Posts => "posts",
            ScopeKind::Dms => "all DMs",
            ScopeKind::All => "all (posts + DMs)",
            ScopeKind::Threads => "threads by ID",
        }
    }

    pub fn has_payload(self) -> bool {
        matches!(self, ScopeKind::Threads)
    }

    pub fn next(self) -> Self {
        match self {
            ScopeKind::None => ScopeKind::Posts,
            ScopeKind::Posts => ScopeKind::Dms,
            ScopeKind::Dms => ScopeKind::All,
            ScopeKind::All => ScopeKind::Threads,
            ScopeKind::Threads => ScopeKind::None,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            ScopeKind::None => ScopeKind::Threads,
            ScopeKind::Posts => ScopeKind::None,
            ScopeKind::Dms => ScopeKind::Posts,
            ScopeKind::All => ScopeKind::Dms,
            ScopeKind::Threads => ScopeKind::All,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeForm {
    pub kind: ScopeKind,
    pub threads: String,
}

#[allow(dead_code)]
impl ScopeForm {
    pub fn new(kind: ScopeKind, threads: String) -> Self {
        Self { kind, threads }
    }

    pub fn from_scope_mode(mode: &ScopeMode) -> Self {
        match mode {
            ScopeMode::None => Self {
                kind: ScopeKind::None,
                threads: String::new(),
            },
            ScopeMode::Posts => Self {
                kind: ScopeKind::Posts,
                threads: String::new(),
            },
            ScopeMode::Dms => Self {
                kind: ScopeKind::Dms,
                threads: String::new(),
            },
            ScopeMode::All => Self {
                kind: ScopeKind::All,
                threads: String::new(),
            },
            ScopeMode::Threads(s) => Self {
                kind: ScopeKind::Threads,
                threads: s.clone(),
            },
        }
    }

    pub fn resolution(&self) -> Result<ScopeMode, String> {
        match self.kind {
            ScopeKind::None => Ok(ScopeMode::None),
            ScopeKind::Posts => Ok(ScopeMode::Posts),
            ScopeKind::Dms => Ok(ScopeMode::Dms),
            ScopeKind::All => Ok(ScopeMode::All),
            ScopeKind::Threads => {
                let ids = thread_id_list(&self.threads);
                if ids.is_empty() {
                    Err("threads scope requires at least one thread ID".to_string())
                } else {
                    Ok(ScopeMode::Threads(ids.join(",")))
                }
            }
        }
    }

    pub fn summary(&self) -> String {
        match self.kind {
            ScopeKind::None => "content only".to_string(),
            ScopeKind::Posts => "posts".to_string(),
            ScopeKind::Dms => "DMs".to_string(),
            ScopeKind::All => "all".to_string(),
            ScopeKind::Threads => {
                let trimmed = self.threads.trim();
                if trimmed.is_empty() {
                    "threads: (none)".to_string()
                } else {
                    format!("threads: {trimmed}")
                }
            }
        }
    }

    pub fn next(&mut self) {
        self.kind = self.kind.next();
    }

    pub fn prev(&mut self) {
        self.kind = self.kind.prev();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentFlags {
    pub likes: bool,

    pub saved: bool,

    pub comments: bool,

    pub archived: bool,

    pub reposts: bool,
}

#[allow(dead_code)]
impl ContentFlags {
    pub const fn likes() -> Self {
        Self {
            likes: true,
            saved: false,
            comments: false,
            archived: false,
            reposts: false,
        }
    }

    pub const fn saved() -> Self {
        Self {
            likes: false,
            saved: true,
            comments: false,
            archived: false,
            reposts: false,
        }
    }

    pub const fn comments() -> Self {
        Self {
            likes: false,
            saved: false,
            comments: true,
            archived: false,
            reposts: false,
        }
    }

    pub const fn archived() -> Self {
        Self {
            likes: false,
            saved: false,
            comments: false,
            archived: true,
            reposts: false,
        }
    }

    pub const fn reposts() -> Self {
        Self {
            likes: false,
            saved: false,
            comments: false,
            archived: false,
            reposts: true,
        }
    }

    pub fn any(self) -> bool {
        self.likes || self.saved || self.comments || self.archived || self.reposts
    }

    pub fn count(self) -> usize {
        usize::from(self.likes)
            + usize::from(self.saved)
            + usize::from(self.comments)
            + usize::from(self.archived)
            + usize::from(self.reposts)
    }

    pub fn label(self) -> String {
        let mut on: Vec<&str> = Vec::new();
        if self.likes {
            on.push("likes");
        }
        if self.saved {
            on.push("saved");
        }
        if self.comments {
            on.push("comments");
        }
        if self.archived {
            on.push("archived");
        }
        if self.reposts {
            on.push("reposts");
        }
        if on.is_empty() {
            "none".to_string()
        } else {
            on.join("+")
        }
    }
}

pub fn thread_id_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split([',', ' ', '\t', '\n']) {
        let trimmed = part.trim();
        if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DmKind {
    #[default]
    All,
    OneOnOne,
    Group,
}

impl DmKind {
    pub fn matches(&self, is_group: bool) -> bool {
        match self {
            Self::All => true,
            Self::OneOnOne => !is_group,
            Self::Group => is_group,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::OneOnOne => "1on1",
            Self::Group => "group",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::OneOnOne,
            Self::OneOnOne => Self::Group,
            Self::Group => Self::All,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::All => Self::Group,
            Self::OneOnOne => Self::All,
            Self::Group => Self::OneOnOne,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let scope = parse_scope_mode("INSTAGRAM_SCOPE");
        let scope_form = ScopeForm::from_scope_mode(&scope);
        Self {
            sessionid: env::var("INSTAGRAM_SESSIONID").unwrap_or_default(),
            session_path: {
                let raw = env::var("INSTAGRAM_SESSION_PATH")
                    .unwrap_or_else(|_| crate::device::DEFAULT_SESSION_PATH.to_string());
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    crate::device::DEFAULT_SESSION_PATH.to_string()
                } else {
                    trimmed.to_string()
                }
            },
            base_url: {
                let raw = env::var("INSTAGRAM_BASE_URL")
                    .unwrap_or_else(|_| "https://i.instagram.com/api/v1".to_string());
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    "https://i.instagram.com/api/v1".to_string()
                } else {
                    trimmed.trim_end_matches('/').to_string()
                }
            },
            scope,
            scope_form,
            before: env::var("INSTAGRAM_BEFORE")
                .unwrap_or_default()
                .trim()
                .to_string(),
            after: env::var("INSTAGRAM_AFTER")
                .unwrap_or_default()
                .trim()
                .to_string(),
            limit: parse_string_int("INSTAGRAM_LIMIT", "0"),
            throttle_ms: parse_string_int("INSTAGRAM_THROTTLE_MS", "2000"),
            scan_throttle_ms: parse_string_int("INSTAGRAM_SCAN_THROTTLE_MS", "350"),
            concurrency: parse_string_int("INSTAGRAM_SCAN_CONCURRENCY", "4"),
            dry_run: parse_bool("INSTAGRAM_DRY_RUN", true),
            hide_threads: parse_bool("INSTAGRAM_HIDE_THREADS", false),
            content: parse_content_flags(),
            protect_users: Vec::new(),
            dm_kind: parse_dm_kind(),
            history_dir: env::var("PRUNER_HISTORY_DIR").unwrap_or_else(|_| "./history".to_string()),
        }
    }
}

pub(crate) fn parse_content_flags() -> ContentFlags {
    let raw = match env::var("INSTAGRAM_CONTENT") {
        Ok(v) => v,
        Err(_) => return ContentFlags::default(),
    };
    let mut flags = ContentFlags::default();
    for token in raw.split([',', ' ', '\t', '\n']) {
        let t = token.trim().to_ascii_lowercase();
        if t.is_empty() {
            continue;
        }
        match t.as_str() {
            "likes" | "like" => flags.likes = true,
            "saved" | "save" => flags.saved = true,
            "comments" | "comment" => flags.comments = true,
            "archived" | "archive" => flags.archived = true,
            "reposts" | "repost" => flags.reposts = true,
            "all" => {
                flags = ContentFlags {
                    likes: true,
                    saved: true,
                    comments: true,
                    archived: true,
                    reposts: true,
                }
            }
            "none" => flags = ContentFlags::default(),
            other => warn(
                "INSTAGRAM_CONTENT",
                other,
                "expected likes|saved|comments|archived|reposts|all|none; ignoring",
            ),
        }
    }
    flags
}

pub(crate) fn parse_content_value(raw: &str) -> Result<ContentFlags, String> {
    let mut flags = ContentFlags::default();
    for token in raw.split([',', ' ', '\t', '\n']) {
        let t = token.trim().to_ascii_lowercase();
        if t.is_empty() {
            continue;
        }
        match t.as_str() {
            "likes" | "like" => flags.likes = true,
            "saved" | "save" => flags.saved = true,
            "comments" | "comment" => flags.comments = true,
            "archived" | "archive" => flags.archived = true,
            "reposts" | "repost" => flags.reposts = true,
            "all" => {
                flags = ContentFlags {
                    likes: true,
                    saved: true,
                    comments: true,
                    archived: true,
                    reposts: true,
                }
            }
            "none" => flags = ContentFlags::default(),
            other => {
                return Err(format!(
                    "invalid --content '{other}': expected likes, saved, comments, archived, \
                     reposts, all, or none"
                ))
            }
        }
    }
    Ok(flags)
}

pub(crate) fn parse_dm_kind() -> DmKind {
    match env::var("INSTAGRAM_DM_KIND")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("all") | None => DmKind::All,
        Some("1on1") | Some("1-on-1") | Some("dm") => DmKind::OneOnOne,
        Some("group") | Some("grp") => DmKind::Group,
        Some(other) => {
            warn(
                "INSTAGRAM_DM_KIND",
                other,
                "expected all|1on1|group; using all",
            );
            DmKind::All
        }
    }
}

pub(crate) fn parse_dm_kind_value(raw: &str) -> Result<DmKind, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "all" => Ok(DmKind::All),
        "1on1" | "1-on-1" | "dm" => Ok(DmKind::OneOnOne),
        "group" | "grp" => Ok(DmKind::Group),
        other => Err(format!(
            "invalid --dm-kind '{other}': expected 'all', '1on1', or 'group'"
        )),
    }
}

pub(crate) fn parse_scope(raw: &str) -> Result<ScopeMode, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("posts") {
        return Ok(ScopeMode::Posts);
    }
    if trimmed.eq_ignore_ascii_case("none") || trimmed.eq_ignore_ascii_case("content") {
        return Ok(ScopeMode::None);
    }
    if trimmed.eq_ignore_ascii_case("dms") || trimmed.eq_ignore_ascii_case("all_dms") {
        return Ok(ScopeMode::Dms);
    }
    if trimmed.eq_ignore_ascii_case("all") {
        return Ok(ScopeMode::All);
    }
    if let Some(rest) = stripped_prefix_ci(trimmed, "threads:") {
        let ids = thread_id_list(rest);
        if ids.is_empty() {
            return Err("scope 'threads:' requires at least one thread id".to_string());
        }
        return Ok(ScopeMode::Threads(ids.join(",")));
    }
    if let Some(rest) = stripped_prefix_ci(trimmed, "threads ") {
        let ids = thread_id_list(rest);
        if ids.is_empty() {
            return Err("scope 'threads ' requires at least one thread id".to_string());
        }
        return Ok(ScopeMode::Threads(ids.join(",")));
    }

    let ids = thread_id_list(trimmed);
    if !ids.is_empty() && ids.iter().all(|id| id.chars().all(|c| c.is_ascii_digit())) {
        return Ok(ScopeMode::Threads(ids.join(",")));
    }
    Err(format!(
        "unrecognised scope '{raw}': expected posts, dms, all, or threads:<id>,<id>"
    ))
}

fn parse_scope_mode(key: &str) -> ScopeMode {
    let raw = match env::var(key) {
        Ok(v) => v,
        Err(_) => return ScopeMode::Posts,
    };
    match parse_scope(&raw) {
        Ok(mode) => mode,
        Err(_) => {
            warn(key, &raw, "unrecognised scope; using 'posts'");
            ScopeMode::Posts
        }
    }
}

fn stripped_prefix_ci<'a>(haystack: &'a str, prefix: &str) -> Option<&'a str> {
    if haystack.len() < prefix.len() {
        return None;
    }
    haystack
        .get(..prefix.len())
        .filter(|p| p.eq_ignore_ascii_case(prefix))
        .and(haystack.get(prefix.len()..))
}

pub(crate) fn parse_bool(key: &str, default: bool) -> bool {
    let raw = match env::var(key) {
        Ok(v) => v,
        Err(_) => return default,
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" | "" => false,
        _ => {
            warn(
                key,
                &raw,
                &format!("expected bool (true/false); using {default}"),
            );
            default
        }
    }
}

pub const MAX_SCAN_CONCURRENCY: u32 = 8;

pub fn clamp_scan_concurrency(raw: u64) -> u32 {
    raw.clamp(1, MAX_SCAN_CONCURRENCY as u64) as u32
}

pub(crate) fn parse_string_int(key: &str, default: &str) -> String {
    let raw = match env::var(key) {
        Ok(v) => v,
        Err(_) => return default.to_string(),
    };
    let trimmed = raw.trim();
    if trimmed.parse::<u64>().is_ok() {
        return trimmed.to_string();
    }
    warn(
        key,
        &raw,
        &format!("expected non-negative integer; using {default}"),
    );
    default.to_string()
}

fn warn(key: &str, value: &str, hint: &str) {
    eprintln!("[instagram-pruner] {key}={value:?} — {hint}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_key(prefix: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!("PRUNER_TEST_{prefix}_{}", N.fetch_add(1, Ordering::Relaxed))
    }

    #[test]
    fn scope_posts_aliases() {
        for v in ["", "posts", "POSTS"] {
            let k = unique_key("scope");
            env::set_var(&k, v);
            assert!(matches!(parse_scope_mode(&k), ScopeMode::Posts));
            env::remove_var(&k);
        }
    }

    #[test]
    fn scope_dms_and_all() {
        let k = unique_key("scope");
        env::set_var(&k, "dms");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::Dms));
        env::set_var(&k, "all_dms");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::Dms));
        env::set_var(&k, "all");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::All));
        env::remove_var(&k);
    }

    #[test]
    fn scope_threads_prefix_and_bare() {
        let k = unique_key("scope");
        env::set_var(&k, "threads:111,222");
        assert!(matches!(&parse_scope_mode(&k), ScopeMode::Threads(s) if s == "111,222"));
        env::set_var(&k, "1,2,3");
        assert!(matches!(&parse_scope_mode(&k), ScopeMode::Threads(s) if s == "1,2,3"));
        env::set_var(&k, "987654321");
        assert!(matches!(&parse_scope_mode(&k), ScopeMode::Threads(s) if s == "987654321"));
        env::remove_var(&k);
    }

    #[test]
    fn scope_none_is_content_only() {
        let k = unique_key("scope");
        env::set_var(&k, "none");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::None));
        env::set_var(&k, "content");
        assert!(matches!(parse_scope_mode(&k), ScopeMode::None));
        env::remove_var(&k);
    }

    #[test]
    fn content_label_names_every_enabled_class() {
        assert_eq!(ContentFlags::default().label(), "none");
        assert_eq!(ContentFlags::likes().label(), "likes");
        let mut flags = ContentFlags::likes();
        flags.saved = true;
        assert_eq!(flags.label(), "likes+saved");
        flags.comments = true;
        flags.archived = true;
        flags.reposts = true;
        assert_eq!(flags.label(), "likes+saved+comments+archived+reposts");
        assert_eq!(flags.count(), 5);
    }

    #[test]
    fn content_flags_parse_and_reject_typos() {
        assert_eq!(parse_content_value("").unwrap(), ContentFlags::default());
        assert_eq!(
            parse_content_value("none").unwrap(),
            ContentFlags::default()
        );

        let likes = parse_content_value("likes").unwrap();
        assert!(likes.likes && !likes.saved && likes.count() == 1);

        let both = parse_content_value("likes, saved").unwrap();
        assert!(both.likes && both.saved && both.count() == 2);

        let all = parse_content_value("all").unwrap();
        assert!(all.any() && all.count() == 5);

        assert!(parse_content_value("likez").is_err());
    }

    #[test]
    fn scan_concurrency_clamps_into_supported_range() {
        assert_eq!(clamp_scan_concurrency(0), 1);
        assert_eq!(clamp_scan_concurrency(1), 1);
        assert_eq!(clamp_scan_concurrency(4), 4);
        assert_eq!(
            clamp_scan_concurrency(u64::from(MAX_SCAN_CONCURRENCY)),
            MAX_SCAN_CONCURRENCY
        );

        assert_eq!(clamp_scan_concurrency(999), MAX_SCAN_CONCURRENCY);
    }

    #[test]
    fn dry_run_defaults_to_enabled() {
        let key = unique_key("dry_run");
        env::remove_var(&key);
        assert!(parse_bool(&key, true));
    }

    #[test]
    fn scope_form_cycling() {
        let mut form = ScopeForm::default();
        assert_eq!(form.kind, ScopeKind::Posts);
        form.next();
        assert_eq!(form.kind, ScopeKind::Dms);
        form.next();
        assert_eq!(form.kind, ScopeKind::All);
        form.next();
        assert_eq!(form.kind, ScopeKind::Threads);
        form.next();
        assert_eq!(form.kind, ScopeKind::None);
        form.next();
        assert_eq!(form.kind, ScopeKind::Posts);
        form.prev();
        assert_eq!(form.kind, ScopeKind::None);
        form.prev();
        assert_eq!(form.kind, ScopeKind::Threads);
    }

    #[test]
    fn scope_form_resolution() {
        let form = ScopeForm {
            kind: ScopeKind::Posts,
            threads: String::new(),
        };
        assert_eq!(form.resolution().unwrap(), ScopeMode::Posts);

        let form = ScopeForm {
            kind: ScopeKind::Threads,
            threads: "123, 456".to_string(),
        };
        assert_eq!(
            form.resolution().unwrap(),
            ScopeMode::Threads("123,456".to_string())
        );

        let form = ScopeForm {
            kind: ScopeKind::Threads,
            threads: "  ".to_string(),
        };
        assert!(form.resolution().is_err());
    }

    #[test]
    fn parse_dm_kind_variants() {
        assert_eq!(parse_dm_kind_value("all").unwrap(), DmKind::All);
        assert_eq!(parse_dm_kind_value("1on1").unwrap(), DmKind::OneOnOne);
        assert_eq!(parse_dm_kind_value("group").unwrap(), DmKind::Group);
        assert!(parse_dm_kind_value("invalid").is_err());
    }

    #[test]
    fn none_scope_resolves_without_targets() {
        let form = ScopeForm {
            kind: ScopeKind::None,
            threads: String::new(),
        };
        assert_eq!(form.resolution().unwrap(), ScopeMode::None);
        assert_eq!(form.summary(), "content only");
        assert!(!form.kind.has_payload());
    }
}
