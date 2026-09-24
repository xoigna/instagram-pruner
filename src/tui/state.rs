use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use chrono::NaiveDate;

use crate::config::{Config, ContentFlags, DmKind, ScopeForm, ScopeKind};
use crate::events::{ChannelStats, LogEntry, LogLevel, Phase, PrunerEvent, RunStats};
use crate::history::{ChannelRef, DeletedRecord, HistoryScope, HistoryStore};
use crate::model::{DirectThread, Media};
use crate::pruner::{dates, PrunerConfig, TargetResolution};

use super::action::{self, Action, Effect, Field, Overlay, Screen, TargetsTab};
use super::keys::InputMode;

const LOG_CAP: usize = 1200;
const SPARKLINE_CAP: usize = 60;
pub const TOAST_TTL: Duration = Duration::from_secs(6);

fn split_ids(raw: &str) -> Vec<String> {
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
pub enum Connection {
    #[default]
    Idle,
    Connecting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user_id: String,
    pub username: String,
    pub full_name: String,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub level: LogLevel,
    pub expires_at: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>, level: LogLevel) -> Self {
        Self {
            text: text.into(),
            level,
            expires_at: Instant::now() + TOAST_TTL,
        }
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

#[derive(Debug, Clone)]
pub struct Form {
    pub sessionid: String,
    pub scope: ScopeForm,
    pub before: String,
    pub after: String,
    pub limit: String,
    pub throttle_ms: String,
    pub scan_throttle_ms: String,

    pub concurrency: String,
    pub dm_kind: DmKind,
    pub dry_run: bool,
    pub hide_threads: bool,

    pub content: ContentFlags,
    pub protect_users: String,
}

impl Form {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            sessionid: cfg.sessionid.clone(),
            scope: cfg.scope_form.clone(),
            before: cfg.before.clone(),
            after: cfg.after.clone(),
            limit: cfg.limit.clone(),
            throttle_ms: cfg.throttle_ms.clone(),
            scan_throttle_ms: cfg.scan_throttle_ms.clone(),
            concurrency: cfg.concurrency.clone(),
            dm_kind: cfg.dm_kind,
            dry_run: cfg.dry_run,
            hide_threads: cfg.hide_threads,
            content: cfg.content,
            protect_users: cfg.protect_users.join(", "),
        }
    }

    pub fn get_text(&self, field: Field) -> String {
        match field {
            Field::SessionId => self.sessionid.clone(),
            Field::Payload => self.scope.threads.clone(),
            Field::Before => self.before.clone(),
            Field::After => self.after.clone(),
            Field::Limit => self.limit.clone(),
            Field::Throttle => self.throttle_ms.clone(),
            Field::ScanThrottle => self.scan_throttle_ms.clone(),
            Field::Concurrency => self.concurrency.clone(),
            Field::Protect => self.protect_users.clone(),
            _ => String::new(),
        }
    }

    pub fn set_text(&mut self, field: Field, value: String) {
        match field {
            Field::SessionId => self.sessionid = value.trim().to_string(),
            Field::Payload => self.scope.threads = value,
            Field::Before => self.before = value.trim().to_string(),
            Field::After => self.after = value.trim().to_string(),
            Field::Limit => self.limit = value.trim().to_string(),
            Field::Throttle => self.throttle_ms = value.trim().to_string(),
            Field::ScanThrottle => self.scan_throttle_ms = value.trim().to_string(),
            Field::Concurrency => self.concurrency = value.trim().to_string(),
            Field::Protect => self.protect_users = value,
            _ => {}
        }
    }

    pub fn toggle(&mut self, field: Field) {
        match field {
            Field::DryRun => self.dry_run = !self.dry_run,
            Field::HideThreads => self.hide_threads = !self.hide_threads,
            Field::ContentLikes => self.content.likes = !self.content.likes,
            Field::ContentSaved => self.content.saved = !self.content.saved,
            Field::ContentComments => self.content.comments = !self.content.comments,
            Field::ContentArchived => self.content.archived = !self.content.archived,
            Field::ContentReposts => self.content.reposts = !self.content.reposts,
            _ => {}
        }
    }

    pub fn cycle_next(&mut self, field: Field) {
        match field {
            Field::Scope => self.scope.next(),
            Field::DmKind => self.dm_kind = self.dm_kind.next(),
            Field::DryRun => self.dry_run = !self.dry_run,
            Field::HideThreads => self.hide_threads = !self.hide_threads,
            Field::ContentLikes => self.content.likes = !self.content.likes,
            Field::ContentSaved => self.content.saved = !self.content.saved,
            Field::ContentComments => self.content.comments = !self.content.comments,
            Field::ContentArchived => self.content.archived = !self.content.archived,
            Field::ContentReposts => self.content.reposts = !self.content.reposts,
            _ => {}
        }
    }

    pub fn cycle_prev(&mut self, field: Field) {
        match field {
            Field::Scope => self.scope.prev(),
            Field::DmKind => self.dm_kind = self.dm_kind.prev(),
            Field::DryRun => self.dry_run = !self.dry_run,
            Field::HideThreads => self.hide_threads = !self.hide_threads,
            Field::ContentLikes => self.content.likes = !self.content.likes,
            Field::ContentSaved => self.content.saved = !self.content.saved,
            Field::ContentComments => self.content.comments = !self.content.comments,
            Field::ContentArchived => self.content.archived = !self.content.archived,
            Field::ContentReposts => self.content.reposts = !self.content.reposts,
            _ => {}
        }
    }

    pub fn validate(&self) -> HashMap<Field, String> {
        let mut errs = HashMap::new();
        if self.sessionid.trim().is_empty() {
            errs.insert(Field::SessionId, "sessionid cookie is required".to_string());
        }
        if self.scope.kind == ScopeKind::Threads && split_ids(&self.scope.threads).is_empty() {
            errs.insert(
                Field::Payload,
                "at least one thread ID is required".to_string(),
            );
        }
        if self.scope.kind == ScopeKind::None && !self.content.any() {
            errs.insert(
                Field::Scope,
                "scope 'none' needs at least one content class enabled".to_string(),
            );
        }
        if !self.before.trim().is_empty() && dates::validate_date_input(&self.before).is_err() {
            errs.insert(
                Field::Before,
                "invalid date: expected YYYY-MM-DD or RFC3339".to_string(),
            );
        }
        if !self.after.trim().is_empty() && dates::validate_date_input(&self.after).is_err() {
            errs.insert(
                Field::After,
                "invalid date: expected YYYY-MM-DD or RFC3339".to_string(),
            );
        }
        if self.limit.trim().parse::<u64>().is_err() {
            errs.insert(Field::Limit, "must be a non-negative integer".to_string());
        }
        if let Ok(throttle) = self.throttle_ms.trim().parse::<u64>() {
            if throttle < 200 {
                errs.insert(Field::Throttle, "must be at least 200ms".to_string());
            }
        } else {
            errs.insert(Field::Throttle, "must be a positive integer".to_string());
        }
        if let Ok(scan) = self.scan_throttle_ms.trim().parse::<u64>() {
            if scan < 100 {
                errs.insert(Field::ScanThrottle, "must be at least 100ms".to_string());
            }
        } else {
            errs.insert(
                Field::ScanThrottle,
                "must be a positive integer".to_string(),
            );
        }
        match self.concurrency.trim().parse::<u64>() {
            Ok(n) if (1..=u64::from(crate::config::MAX_SCAN_CONCURRENCY)).contains(&n) => {}
            Ok(_) => {
                errs.insert(
                    Field::Concurrency,
                    format!("must be 1-{}", crate::config::MAX_SCAN_CONCURRENCY),
                );
            }
            Err(_) => {
                errs.insert(Field::Concurrency, "must be a positive integer".to_string());
            }
        }
        errs
    }

    pub fn to_pruner_config(&self) -> Result<PrunerConfig, String> {
        let targets = match self.scope.kind {
            ScopeKind::None => Vec::new(),
            ScopeKind::Posts => vec![TargetResolution::Posts],
            ScopeKind::Dms => vec![TargetResolution::AllDms],
            ScopeKind::All => vec![TargetResolution::Posts, TargetResolution::AllDms],
            ScopeKind::Threads => {
                let ids = split_ids(&self.scope.threads);
                if ids.is_empty() {
                    return Err("threads scope requires at least one thread ID".to_string());
                }
                vec![TargetResolution::Threads(ids)]
            }
        };

        let before = if self.before.trim().is_empty() {
            None
        } else {
            Some(self.before.trim().to_string())
        };

        let after = if self.after.trim().is_empty() {
            None
        } else {
            Some(self.after.trim().to_string())
        };

        let limit = self
            .limit
            .trim()
            .parse::<usize>()
            .map_err(|_| "invalid limit".to_string())?;
        let throttle_ms = self
            .throttle_ms
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid throttle".to_string())?;
        let scan_throttle_ms = self
            .scan_throttle_ms
            .trim()
            .parse::<u64>()
            .map_err(|_| "invalid scan throttle".to_string())?;
        let concurrency = crate::config::clamp_scan_concurrency(
            self.concurrency
                .trim()
                .parse::<u64>()
                .map_err(|_| "invalid scan concurrency".to_string())?,
        );

        let protect_users = split_ids(&self.protect_users);

        Ok(PrunerConfig {
            targets,
            content: self.content,
            before,
            after,
            limit,
            dry_run: self.dry_run,
            throttle_ms,
            scan_throttle_ms,
            concurrency,
            hide_threads: self.hide_threads,
            protect_users,
            dm_kind: self.dm_kind,
        })
    }
}

pub fn mask_sessionid(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return "(empty)".to_string();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 6 {
        return "••••••".to_string();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("••••••••••••{tail}")
}

#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub cursor: usize,
    pub editing: bool,
    pub buffer: String,
    pub buffer_cursor: usize,
    pub errors: HashMap<Field, String>,
}

impl SettingsState {
    pub fn field(&self) -> Field {
        Field::ALL[self.cursor.min(Field::ALL.len() - 1)]
    }

    fn move_visible(&mut self, scope: ScopeKind, forward: bool) {
        let visible: Vec<usize> = Field::ALL
            .iter()
            .enumerate()
            .filter(|(_, field)| **field != Field::Payload || scope.has_payload())
            .map(|(index, _)| index)
            .collect();
        let current = visible
            .iter()
            .position(|index| *index == self.cursor)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % visible.len()
        } else {
            (current + visible.len() - 1) % visible.len()
        };
        self.cursor = visible[next];
    }

    pub fn start_edit(&mut self, current_value: &str) {
        self.editing = true;
        self.buffer = current_value.to_string();
        self.buffer_cursor = self.buffer.chars().count();
    }

    pub fn commit_edit(&mut self) -> String {
        self.editing = false;
        std::mem::take(&mut self.buffer)
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.buffer.clear();
        self.buffer_cursor = 0;
    }

    pub fn input(&mut self, c: char) {
        let mut chars: Vec<char> = self.buffer.chars().collect();
        let idx = self.buffer_cursor.min(chars.len());
        chars.insert(idx, c);
        self.buffer = chars.into_iter().collect();
        self.buffer_cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.buffer_cursor > 0 {
            let mut chars: Vec<char> = self.buffer.chars().collect();
            chars.remove(self.buffer_cursor - 1);
            self.buffer = chars.into_iter().collect();
            self.buffer_cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        let mut chars: Vec<char> = self.buffer.chars().collect();
        if self.buffer_cursor < chars.len() {
            chars.remove(self.buffer_cursor);
            self.buffer = chars.into_iter().collect();
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelProgress {
    pub channel_id: String,
    pub channel_name: String,
    pub scanned: u64,
    pub eligible: u64,
    pub deleted: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimiterView {
    pub last_wait_ms: u64,
    pub hits_429: u64,
    pub cooldown_remaining_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunStatus {
    #[default]
    Idle,
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct RunState {
    pub status: RunStatus,
    pub phase: Phase,
    pub started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
    pub channels: Vec<(String, String, ChannelStats)>,
    pub active_channel: Option<String>,
    pub current_progress: Option<ChannelProgress>,
    pub stats: RunStats,
    pub plan_total: usize,
    pub plan_done: usize,
    pub log: VecDeque<LogEntry>,
    pub log_scroll: usize,
    pub rate_limiter: RateLimiterView,
    pub sparkline_data: VecDeque<u64>,
}

impl Default for RunState {
    fn default() -> Self {
        Self {
            status: RunStatus::Idle,
            phase: Phase::Scan,
            started_at: None,
            finished_at: None,
            channels: Vec::new(),
            active_channel: None,
            current_progress: None,
            stats: RunStats::default(),
            plan_total: 0,
            plan_done: 0,
            log: VecDeque::new(),
            log_scroll: 0,
            rate_limiter: RateLimiterView::default(),
            sparkline_data: VecDeque::new(),
        }
    }
}

impl RunState {
    pub fn is_running(&self) -> bool {
        self.status == RunStatus::Running
    }

    pub fn elapsed(&self) -> Duration {
        match (self.started_at, self.finished_at) {
            (Some(s), Some(f)) => f.duration_since(s),
            (Some(s), None) => Instant::now().duration_since(s),
            _ => Duration::ZERO,
        }
    }

    pub fn push_log(&mut self, level: LogLevel, message: impl Into<String>) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            level,
            message: message.into(),
            at: std::time::SystemTime::now(),
        });
    }

    pub fn push_rate(&mut self, ms: u64) {
        if self.sparkline_data.len() >= SPARKLINE_CAP {
            self.sparkline_data.pop_front();
        }
        self.sparkline_data.push_back(ms);
    }
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct TargetsState {
    pub tab: TargetsTab,
    pub threads: Vec<DirectThread>,
    pub posts: Vec<Media>,
    pub selected_threads: HashSet<String>,
    pub protected_users: HashSet<String>,
    pub thread_cursor: usize,
    pub post_cursor: usize,
    pub loading: bool,
}

impl TargetsState {
    pub fn filtered_threads(&self, kind: DmKind) -> Vec<&DirectThread> {
        self.threads
            .iter()
            .filter(|t| kind.matches(t.is_group))
            .collect()
    }

    pub fn is_protected(&self, thread: &DirectThread) -> bool {
        if self.protected_users.contains(&thread.thread_id) {
            return true;
        }
        for u in &thread.users {
            if self.protected_users.contains(&u.pk.to_string())
                || self.protected_users.contains(&u.username)
            {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchiveFocus {
    #[default]
    Days,
    Channels,
    Records,
}

impl ArchiveFocus {
    pub fn next(self) -> Self {
        match self {
            Self::Days => Self::Channels,
            Self::Channels => Self::Records,
            Self::Records => Self::Days,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Days => Self::Records,
            Self::Channels => Self::Days,
            Self::Records => Self::Channels,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ArchiveState {
    pub store: HistoryStore,
    pub scope: HistoryScope,
    pub focus: ArchiveFocus,
    pub days: Vec<NaiveDate>,
    pub day_cursor: usize,
    pub channels: Vec<ChannelRef>,
    pub channel_cursor: usize,
    pub records: Vec<DeletedRecord>,
    pub record_cursor: usize,
    pub show_detail: bool,
    pub detail_scroll: usize,
}

impl Default for ArchiveState {
    fn default() -> Self {
        let store = HistoryStore::with_root("./history");
        let days = store.list_days(HistoryScope::All).unwrap_or_default();
        let channels = days
            .first()
            .and_then(|d| store.list_channels(*d, HistoryScope::All).ok())
            .unwrap_or_default();
        let records = match (days.first(), channels.first()) {
            (Some(d), Some(c)) => store
                .load_day(*d, HistoryScope::All, Some(&c.id))
                .unwrap_or_default(),
            (Some(d), None) => store
                .load_day(*d, HistoryScope::All, None)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Self {
            store,
            scope: HistoryScope::All,
            focus: ArchiveFocus::Days,
            days,
            day_cursor: 0,
            channels,
            channel_cursor: 0,
            records,
            record_cursor: 0,
            show_detail: false,
            detail_scroll: 0,
        }
    }
}

impl ArchiveState {
    pub fn reload(&mut self) {
        self.days = self.store.list_days(self.scope).unwrap_or_default();
        self.day_cursor = self.day_cursor.min(self.days.len().saturating_sub(1));
        self.reload_channels();
    }

    pub fn reload_channels(&mut self) {
        if let Some(&day) = self.days.get(self.day_cursor) {
            self.channels = self
                .store
                .list_channels(day, self.scope)
                .unwrap_or_default();
        } else {
            self.channels.clear();
        }
        self.channel_cursor = self
            .channel_cursor
            .min(self.channels.len().saturating_sub(1));
        self.reload_records();
    }

    pub fn reload_records(&mut self) {
        if let Some(&day) = self.days.get(self.day_cursor) {
            if let Some(ch) = self.channels.get(self.channel_cursor) {
                self.records = self
                    .store
                    .load_day(day, self.scope, Some(&ch.id))
                    .unwrap_or_default();
            } else {
                self.records = self
                    .store
                    .load_day(day, self.scope, None)
                    .unwrap_or_default();
            }
        } else {
            self.records.clear();
        }
        self.record_cursor = self.record_cursor.min(self.records.len().saturating_sub(1));
    }

    pub fn cycle_scope(&mut self) {
        self.scope = self.scope.next();
        self.reload();
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub matches: Vec<usize>,
    pub cursor: usize,
}

impl Default for PaletteState {
    fn default() -> Self {
        let count = action::commands().len();
        Self {
            open: false,
            query: String::new(),
            matches: (0..count).collect(),
            cursor: 0,
        }
    }
}

impl PaletteState {
    pub fn update_matches(&mut self) {
        let q = self.query.trim().to_ascii_lowercase();
        self.matches.clear();
        for (i, cmd) in action::commands().iter().enumerate() {
            if q.is_empty()
                || cmd.label.to_ascii_lowercase().contains(&q)
                || cmd.hint.to_ascii_lowercase().contains(&q)
                || cmd.id.to_ascii_lowercase().contains(&q)
            {
                self.matches.push(i);
            }
        }
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }
}

#[derive(Debug)]
pub struct AppState {
    pub screen: Screen,
    pub overlay: Overlay,
    pub connection: Connection,
    pub identity: Option<Identity>,
    pub toast: Option<Toast>,
    pub form: Form,
    pub settings: SettingsState,
    pub targets: TargetsState,
    pub run: RunState,
    pub archive: ArchiveState,
    pub palette: PaletteState,
    pub config: Config,
}

impl AppState {
    pub fn new(cfg: Config) -> Self {
        let form = Form::from_config(&cfg);
        let mut archive = ArchiveState {
            store: HistoryStore::with_root(&cfg.history_dir),
            ..ArchiveState::default()
        };
        archive.reload();

        let mut targets = TargetsState::default();
        for u in &cfg.protect_users {
            targets.protected_users.insert(u.clone());
        }

        Self {
            screen: Screen::Settings,
            overlay: Overlay::None,
            connection: Connection::Idle,
            identity: None,
            toast: None,
            form,
            settings: SettingsState::default(),
            targets,
            run: RunState::default(),
            archive,
            palette: PaletteState::default(),
            config: cfg,
        }
    }

    pub fn input_mode(&self) -> InputMode {
        match self.overlay {
            Overlay::Help => InputMode::Help,
            Overlay::Palette => InputMode::Palette,
            Overlay::Confirm => InputMode::Confirm,
            Overlay::None => {
                if self.settings.editing {
                    InputMode::Text
                } else {
                    InputMode::Normal
                }
            }
        }
    }

    pub fn set_toast(&mut self, text: impl Into<String>, level: LogLevel) {
        self.toast = Some(Toast::new(text, level));
    }

    pub fn apply_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Quit => vec![Effect::Quit],
            Action::Help => {
                self.overlay = if self.overlay == Overlay::Help {
                    Overlay::None
                } else {
                    Overlay::Help
                };
                Vec::new()
            }
            Action::Palette => {
                self.overlay = if self.overlay == Overlay::Palette {
                    Overlay::None
                } else {
                    self.palette.query.clear();
                    self.palette.update_matches();
                    self.palette.cursor = 0;
                    Overlay::Palette
                };
                Vec::new()
            }
            Action::PaletteDown => {
                if !self.palette.matches.is_empty() {
                    self.palette.cursor = (self.palette.cursor + 1) % self.palette.matches.len();
                }
                Vec::new()
            }
            Action::PaletteUp => {
                if !self.palette.matches.is_empty() {
                    self.palette.cursor = (self.palette.cursor + self.palette.matches.len() - 1)
                        % self.palette.matches.len();
                }
                Vec::new()
            }
            Action::PaletteSelect => {
                if let Some(&cmd_idx) = self.palette.matches.get(self.palette.cursor) {
                    let cmd = action::commands().remove(cmd_idx);
                    self.overlay = Overlay::None;
                    self.apply_action(cmd.action)
                } else {
                    self.overlay = Overlay::None;
                    Vec::new()
                }
            }
            Action::Back => {
                if self.overlay != Overlay::None {
                    self.overlay = Overlay::None;
                    Vec::new()
                } else if self.settings.editing {
                    self.settings.cancel_edit();
                    Vec::new()
                } else if self.run.is_running() {
                    vec![Effect::CancelRun]
                } else {
                    Vec::new()
                }
            }
            Action::PrevScreen => {
                self.screen = self.screen.prev();
                Vec::new()
            }
            Action::NextScreen => {
                self.screen = self.screen.next();
                Vec::new()
            }
            Action::Goto(screen) => {
                self.screen = screen;
                Vec::new()
            }
            Action::NextField => {
                if self.screen == Screen::Settings {
                    self.settings.move_visible(self.form.scope.kind, true);
                } else if self.screen == Screen::Archive {
                    self.archive.focus = self.archive.focus.next();
                } else if self.screen == Screen::Targets {
                    self.targets.tab = self.targets.tab.next();
                }
                Vec::new()
            }
            Action::PrevField => {
                if self.screen == Screen::Settings {
                    self.settings.move_visible(self.form.scope.kind, false);
                } else if self.screen == Screen::Archive {
                    self.archive.focus = self.archive.focus.prev();
                } else if self.screen == Screen::Targets {
                    self.targets.tab = self.targets.tab.prev();
                }
                Vec::new()
            }
            Action::Down => {
                match self.screen {
                    Screen::Settings => {
                        self.settings.move_visible(self.form.scope.kind, true);
                    }
                    Screen::Targets => {
                        let total = self.targets.filtered_threads(self.form.dm_kind).len();
                        if total > 0 {
                            self.targets.thread_cursor =
                                (self.targets.thread_cursor + 1).min(total - 1);
                        }
                    }
                    Screen::Run => {
                        self.run.log_scroll = self.run.log_scroll.saturating_sub(1);
                    }
                    Screen::Archive => match self.archive.focus {
                        ArchiveFocus::Days => {
                            if !self.archive.days.is_empty() {
                                self.archive.day_cursor =
                                    (self.archive.day_cursor + 1).min(self.archive.days.len() - 1);
                                self.archive.reload_channels();
                            }
                        }
                        ArchiveFocus::Channels => {
                            if !self.archive.channels.is_empty() {
                                self.archive.channel_cursor = (self.archive.channel_cursor + 1)
                                    .min(self.archive.channels.len() - 1);
                                self.archive.reload_records();
                            }
                        }
                        ArchiveFocus::Records => {
                            if !self.archive.records.is_empty() {
                                self.archive.record_cursor = (self.archive.record_cursor + 1)
                                    .min(self.archive.records.len() - 1);
                            }
                        }
                    },
                }
                Vec::new()
            }
            Action::Up => {
                match self.screen {
                    Screen::Settings => {
                        self.settings.move_visible(self.form.scope.kind, false);
                    }
                    Screen::Targets => {
                        self.targets.thread_cursor = self.targets.thread_cursor.saturating_sub(1);
                    }
                    Screen::Run => {
                        self.run.log_scroll = self.run.log_scroll.saturating_add(1);
                    }
                    Screen::Archive => match self.archive.focus {
                        ArchiveFocus::Days => {
                            self.archive.day_cursor = self.archive.day_cursor.saturating_sub(1);
                            self.archive.reload_channels();
                        }
                        ArchiveFocus::Channels => {
                            self.archive.channel_cursor =
                                self.archive.channel_cursor.saturating_sub(1);
                            self.archive.reload_records();
                        }
                        ArchiveFocus::Records => {
                            self.archive.record_cursor =
                                self.archive.record_cursor.saturating_sub(1);
                        }
                    },
                }
                Vec::new()
            }
            Action::CycleNext => {
                if self.screen == Screen::Settings {
                    let field = self.settings.field();
                    self.form.cycle_next(field);
                    if field == Field::Scope
                        && !self.form.scope.kind.has_payload()
                        && self.settings.field() == Field::Payload
                    {
                        self.settings.cursor = Field::ALL
                            .iter()
                            .position(|field| *field == Field::Scope)
                            .unwrap_or(0);
                    }
                } else if self.screen == Screen::Archive {
                    self.archive.cycle_scope();
                }
                Vec::new()
            }
            Action::CyclePrev => {
                if self.screen == Screen::Settings {
                    let field = self.settings.field();
                    self.form.cycle_prev(field);
                    if field == Field::Scope
                        && !self.form.scope.kind.has_payload()
                        && self.settings.field() == Field::Payload
                    {
                        self.settings.cursor = Field::ALL
                            .iter()
                            .position(|field| *field == Field::Scope)
                            .unwrap_or(0);
                    }
                } else if self.screen == Screen::Archive {
                    self.archive.cycle_scope();
                }
                Vec::new()
            }
            Action::Toggle => {
                if self.screen == Screen::Settings {
                    let field = self.settings.field();
                    self.form.toggle(field);
                } else if self.screen == Screen::Targets {
                    let threads = self.targets.filtered_threads(self.form.dm_kind);
                    if let Some(thread) = threads.get(self.targets.thread_cursor) {
                        let id = thread.thread_id.clone();
                        if self.targets.selected_threads.contains(&id) {
                            self.targets.selected_threads.remove(&id);
                        } else {
                            self.targets.selected_threads.insert(id);
                        }
                    }
                }
                Vec::new()
            }
            Action::ToggleProtect => {
                if self.screen == Screen::Targets {
                    let threads = self.targets.filtered_threads(self.form.dm_kind);
                    if let Some(thread) = threads.get(self.targets.thread_cursor) {
                        let id = thread.thread_id.clone();
                        if self.targets.protected_users.contains(&id) {
                            self.targets.protected_users.remove(&id);
                            self.set_toast("unprotected thread", LogLevel::Info);
                        } else {
                            self.targets.protected_users.insert(id);
                            self.set_toast("protected thread", LogLevel::Success);
                        }
                        self.form.protect_users = self
                            .targets
                            .protected_users
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                    }
                }
                Vec::new()
            }
            Action::ToggleDryRun => {
                self.form.dry_run = !self.form.dry_run;
                let state = if self.form.dry_run { "ON" } else { "OFF" };
                self.set_toast(format!("dry run {state}"), LogLevel::Info);
                Vec::new()
            }
            Action::StartEdit => {
                if self.screen == Screen::Settings {
                    let field = self.settings.field();
                    if field.is_text() {
                        let val = self.form.get_text(field);
                        self.settings.start_edit(&val);
                    }
                }
                Vec::new()
            }
            Action::CommitEdit => {
                if self.settings.editing {
                    let field = self.settings.field();
                    let val = self.settings.commit_edit();
                    self.form.set_text(field, val);
                    self.settings.errors = self.form.validate();
                }
                Vec::new()
            }
            Action::CancelEdit => {
                self.settings.cancel_edit();
                Vec::new()
            }
            Action::Input(c) => {
                if self.overlay == Overlay::Palette {
                    self.palette.query.push(c);
                    self.palette.update_matches();
                } else if self.settings.editing {
                    self.settings.input(c);
                }
                Vec::new()
            }
            Action::InputBackspace => {
                if self.overlay == Overlay::Palette {
                    self.palette.query.pop();
                    self.palette.update_matches();
                } else if self.settings.editing {
                    self.settings.backspace();
                }
                Vec::new()
            }
            Action::InputDelete => {
                if self.settings.editing {
                    self.settings.delete();
                }
                Vec::new()
            }
            Action::InputLeft => {
                if self.settings.editing && self.settings.buffer_cursor > 0 {
                    self.settings.buffer_cursor -= 1;
                }
                Vec::new()
            }
            Action::InputRight => {
                if self.settings.editing
                    && self.settings.buffer_cursor < self.settings.buffer.chars().count()
                {
                    self.settings.buffer_cursor += 1;
                }
                Vec::new()
            }
            Action::InputHome => {
                if self.settings.editing {
                    self.settings.buffer_cursor = 0;
                }
                Vec::new()
            }
            Action::InputEnd => {
                if self.settings.editing {
                    self.settings.buffer_cursor = self.settings.buffer.chars().count();
                }
                Vec::new()
            }
            Action::InputClear => {
                if self.overlay == Overlay::Palette {
                    self.palette.query.clear();
                    self.palette.update_matches();
                } else if self.settings.editing {
                    self.settings.buffer.clear();
                    self.settings.buffer_cursor = 0;
                }
                Vec::new()
            }
            Action::SelectAll => {
                if self.screen == Screen::Targets {
                    let ids: Vec<String> = self
                        .targets
                        .filtered_threads(self.form.dm_kind)
                        .iter()
                        .map(|t| t.thread_id.clone())
                        .collect();
                    for id in ids {
                        self.targets.selected_threads.insert(id);
                    }
                }
                Vec::new()
            }
            Action::SelectNone => {
                if self.screen == Screen::Targets {
                    self.targets.selected_threads.clear();
                }
                Vec::new()
            }
            Action::SelectInvert => {
                if self.screen == Screen::Targets {
                    let all_ids: Vec<String> = self
                        .targets
                        .filtered_threads(self.form.dm_kind)
                        .iter()
                        .map(|t| t.thread_id.clone())
                        .collect();
                    let mut inverted = HashSet::new();
                    for id in all_ids {
                        if !self.targets.selected_threads.contains(&id) {
                            inverted.insert(id);
                        }
                    }
                    self.targets.selected_threads = inverted;
                }
                Vec::new()
            }
            Action::Activate => {
                if self.screen == Screen::Targets {
                    let picked = self.picked_threads();
                    if !picked.is_empty() {
                        let count = picked.len();
                        let ids: Vec<String> = picked.iter().map(|t| t.thread_id.clone()).collect();
                        self.form.scope.kind = ScopeKind::Threads;
                        self.form.scope.threads = ids.join(", ");
                        self.set_toast(
                            format!(
                                "thread-ids scope set to {count} picked thread(s) — run uses them either way"
                            ),
                            LogLevel::Success,
                        );
                        self.screen = Screen::Settings;
                    }
                    Vec::new()
                } else if self.screen == Screen::Settings || self.screen == Screen::Run {
                    if self.run.is_running() {
                        self.set_toast("a prune run is already active", LogLevel::Warning);
                        return Vec::new();
                    }
                    self.settings.errors = self.form.validate();
                    if !self.settings.errors.is_empty() {
                        self.set_toast("fix validation errors before running", LogLevel::Error);
                        self.screen = Screen::Settings;
                        return Vec::new();
                    }
                    if !self.form.dry_run && self.overlay != Overlay::Confirm {
                        self.overlay = Overlay::Confirm;
                        return Vec::new();
                    }
                    self.start_run_effects()
                } else {
                    Vec::new()
                }
            }
            Action::Confirm => {
                self.overlay = Overlay::None;
                self.start_run_effects()
            }
            Action::Decline => {
                self.overlay = Overlay::None;
                Vec::new()
            }
            Action::Probe => {
                self.connection = Connection::Connecting;
                self.targets.loading = true;
                vec![Effect::Probe]
            }
            Action::Refresh => {
                if self.screen == Screen::Archive {
                    self.archive.reload();
                    self.set_toast("archive reloaded", LogLevel::Info);
                    Vec::new()
                } else {
                    self.targets.loading = true;
                    vec![Effect::LoadTargets]
                }
            }
            Action::ToggleDetail => {
                if self.screen == Screen::Archive {
                    self.archive.show_detail = !self.archive.show_detail;
                }
                Vec::new()
            }
            Action::Yank => {
                if self.screen == Screen::Archive {
                    if let Some(record) = self.archive.records.get(self.archive.record_cursor) {
                        let json = serde_json::to_string_pretty(record).unwrap_or_default();
                        self.set_toast("copied record to clipboard", LogLevel::Success);
                        return vec![Effect::Yank(json)];
                    }
                }
                Vec::new()
            }
            Action::ClearLog => {
                self.run.log.clear();
                self.run.log_scroll = 0;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn picked_threads(&self) -> Vec<DirectThread> {
        self.targets
            .threads
            .iter()
            .filter(|t| self.targets.selected_threads.contains(&t.thread_id))
            .cloned()
            .collect()
    }

    pub fn picker_override(&self) -> Option<Vec<DirectThread>> {
        if !matches!(self.form.scope.kind, ScopeKind::Dms | ScopeKind::All) {
            return None;
        }
        let picked = self.picked_threads();
        (!picked.is_empty()).then_some(picked)
    }

    fn start_run_effects(&mut self) -> Vec<Effect> {
        match self.form.to_pruner_config() {
            Ok(mut pruner_cfg) => {
                self.run.log.clear();
                self.run.log_scroll = 0;
                if let Some(picked) = self.picker_override() {
                    let count = picked.len();

                    pruner_cfg.targets = vec![TargetResolution::ThreadObjects(picked)];
                    self.run.push_log(
                        LogLevel::Info,
                        format!(
                            "only {count} picked thread(s) — scope {} ignored",
                            self.form.scope.kind.label()
                        ),
                    );
                }
                self.run.status = RunStatus::Running;
                self.run.phase = Phase::Scan;
                self.run.started_at = Some(Instant::now());
                self.run.finished_at = None;
                self.run.plan_total = 0;
                self.run.plan_done = 0;
                self.run.stats = RunStats::default();
                self.run.channels.clear();
                self.run.current_progress = None;
                self.run.log_scroll = 0;

                if pruner_cfg.content.any() {
                    self.run.push_log(
                        LogLevel::Info,
                        format!("content classes: {}", pruner_cfg.content.label()),
                    );
                }
                self.screen = Screen::Run;
                vec![Effect::StartRun(pruner_cfg)]
            }
            Err(e) => {
                self.set_toast(format!("config error: {e}"), LogLevel::Error);
                Vec::new()
            }
        }
    }

    pub fn apply_event(&mut self, event: PrunerEvent) {
        match event {
            PrunerEvent::Started { channel_count } => {
                self.run.stats.channel_count = channel_count;
                self.run.push_log(
                    LogLevel::Info,
                    format!("started scan of {channel_count} target(s)"),
                );
            }
            PrunerEvent::ChannelStarted {
                channel_id,
                channel_name,
                index,
                total,
                phase,
            } => {
                self.run.phase = phase;
                self.run.active_channel = Some(channel_id.clone());
                self.run.current_progress = Some(ChannelProgress {
                    channel_id: channel_id.clone(),
                    channel_name: channel_name.clone(),
                    scanned: 0,
                    eligible: 0,
                    deleted: 0,
                });
                self.run.push_log(
                    LogLevel::Info,
                    format!("[{index}/{total}] starting {channel_name}"),
                );
            }
            PrunerEvent::ScanProgress {
                channel_id,
                scanned,
                eligible,
            } => {
                if let Some(p) = &mut self.run.current_progress {
                    if p.channel_id == channel_id {
                        p.scanned = scanned;
                        p.eligible = eligible;
                    }
                }
            }
            PrunerEvent::MessageDeleted {
                preview, dry_run, ..
            } => {
                self.run.plan_done += 1;
                self.run.stats.messages_deleted += 1;
                if let Some(p) = &mut self.run.current_progress {
                    p.deleted += 1;
                }
                let tag = if dry_run { "plan" } else { "deleted" };
                self.run
                    .push_log(LogLevel::Success, format!("{tag}: {preview}"));
            }
            PrunerEvent::MessageFailed {
                channel_id: _,
                message_id,
                error,
            } => {
                self.run.stats.messages_failed += 1;
                self.run
                    .push_log(LogLevel::Error, format!("failed {message_id}: {error}"));
            }
            PrunerEvent::ChannelDone {
                channel_id,
                stats,
                phase,
            } => {
                self.run.stats.aggregate(stats);
                let name = self
                    .run
                    .current_progress
                    .as_ref()
                    .map(|p| p.channel_name.clone())
                    .unwrap_or_else(|| channel_id.clone());
                self.run.channels.push((channel_id, name.clone(), stats));
                self.run.push_log(
                    LogLevel::Info,
                    format!("finished {name} ({phase:?}): deleted {}", stats.deleted),
                );
            }
            PrunerEvent::PlanReady { total } => {
                self.run.plan_total = total;
                self.run.phase = Phase::Delete;
                self.run.push_log(
                    LogLevel::Success,
                    format!("plan ready: {total} items to prune"),
                );
            }
            PrunerEvent::RateLimited {
                retry_after_ms,
                global: _,
                scope: _,
            } => {
                self.run.stats.rate_limit_hits += 1;
                self.run.rate_limiter.hits_429 += 1;
                self.run.rate_limiter.cooldown_remaining_ms = retry_after_ms;
                self.run.push_rate(retry_after_ms);
                self.run.push_log(
                    LogLevel::Warning,
                    format!("rate limited: backing off for {retry_after_ms}ms"),
                );
            }
            PrunerEvent::Log { level, message } => {
                self.run.push_log(level, message);
            }
            PrunerEvent::Done { stats } => {
                self.run.status = RunStatus::Completed;
                self.run.finished_at = Some(Instant::now());
                self.run.stats = stats;
                self.run.current_progress = None;
                self.run.active_channel = None;
                self.run.push_log(
                    LogLevel::Success,
                    format!(
                        "run complete: {} deleted, {} failed in {:?}",
                        stats.messages_deleted,
                        stats.messages_failed,
                        self.run.elapsed()
                    ),
                );
                self.archive.reload();
            }
            PrunerEvent::Cancelled => {
                self.run.status = RunStatus::Cancelled;
                self.run.finished_at = Some(Instant::now());
                self.run.current_progress = None;
                self.run.push_log(LogLevel::Warning, "run cancelled");
            }
            PrunerEvent::Error { message } => {
                self.run.status = RunStatus::Failed;
                self.run.finished_at = Some(Instant::now());
                self.run
                    .push_log(LogLevel::Error, format!("error: {message}"));
            }
            PrunerEvent::Identity {
                user_id,
                username,
                full_name,
            } => {
                self.connection = Connection::Ready;
                self.identity = Some(Identity {
                    user_id,
                    username,
                    full_name,
                });
            }
            PrunerEvent::DMsLoaded { channels, .. } => {
                self.targets.loading = false;

                let before = self.targets.selected_threads.len();
                self.targets
                    .selected_threads
                    .retain(|id| channels.iter().any(|t| &t.thread_id == id));
                let dropped = before - self.targets.selected_threads.len();
                self.targets.threads = channels;
                if dropped > 0 {
                    self.set_toast(
                        format!("{dropped} picked thread(s) no longer in the inbox — deselected"),
                        LogLevel::Warning,
                    );
                }
            }
            PrunerEvent::DMsLoadFailed { message, .. } => {
                self.targets.loading = false;
                self.set_toast(
                    format!("failed to load threads: {message}"),
                    LogLevel::Error,
                );
            }
            PrunerEvent::ProbeFailed { message } => {
                self.connection = Connection::Failed;
                self.targets.loading = false;
                self.set_toast(format!("probe failed: {message}"), LogLevel::Error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_validation_checks_sessionid_and_dates() {
        let mut form = Form::from_config(&Config::from_env());
        form.sessionid = String::new();
        let errs = form.validate();
        assert!(errs.contains_key(&Field::SessionId));

        form.sessionid = "fake_sessionid".to_string();
        form.before = "invalid_date".to_string();
        let errs = form.validate();
        assert!(errs.contains_key(&Field::Before));

        form.before = "2026-01-01".to_string();
        let errs = form.validate();
        assert!(!errs.contains_key(&Field::Before));
    }

    #[test]
    fn settings_navigation_skips_hidden_thread_payload() {
        let mut app = AppState::new(Config::from_env());
        app.screen = Screen::Settings;
        app.settings.cursor = Field::ALL
            .iter()
            .position(|field| *field == Field::Scope)
            .unwrap();

        app.apply_action(Action::Down);
        assert_eq!(app.settings.field(), Field::Before);

        app.form.scope.kind = ScopeKind::Threads;
        app.settings.cursor = Field::ALL
            .iter()
            .position(|field| *field == Field::Scope)
            .unwrap();
        app.apply_action(Action::Down);
        assert_eq!(app.settings.field(), Field::Payload);
    }

    #[test]
    fn cycling_away_from_thread_scope_moves_focus_off_hidden_payload() {
        let mut app = AppState::new(Config::from_env());
        app.screen = Screen::Settings;
        app.form.scope.kind = ScopeKind::Threads;
        app.settings.cursor = Field::ALL
            .iter()
            .position(|field| *field == Field::Scope)
            .unwrap();
        app.apply_action(Action::CycleNext);
        assert_eq!(app.form.scope.kind, ScopeKind::None);
        assert_eq!(app.settings.field(), Field::Scope);
    }

    #[test]
    fn target_promotion_sets_threads_scope() {
        let mut app = AppState::new(Config::from_env());
        app.screen = Screen::Targets;
        app.targets.threads = vec![thread("thread_456"), thread("thread_123")];
        app.targets
            .selected_threads
            .insert("thread_123".to_string());
        app.targets
            .selected_threads
            .insert("thread_456".to_string());

        app.apply_action(Action::Activate);

        assert_eq!(app.form.scope.kind, ScopeKind::Threads);
        assert_eq!(app.form.scope.threads, "thread_456, thread_123");
        assert_eq!(app.screen, Screen::Settings);
    }

    #[test]
    fn selecting_quit_from_command_palette_returns_quit_effect() {
        let mut app = AppState::new(Config::from_env());
        app.overlay = Overlay::Palette;
        app.palette.matches = action::commands()
            .iter()
            .enumerate()
            .filter(|(_, command)| command.id == "quit")
            .map(|(index, _)| index)
            .collect();
        app.palette.cursor = 0;

        assert!(matches!(
            app.apply_action(Action::PaletteSelect).as_slice(),
            [Effect::Quit]
        ));
    }

    #[test]
    fn active_run_cannot_be_started_again() {
        let mut app = AppState::new(Config::from_env());
        app.form.sessionid = "fake_sessionid".into();
        app.form.dry_run = true;
        app.run.status = RunStatus::Running;

        assert!(app.apply_action(Action::Activate).is_empty());
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|toast| toast.text.contains("already active")));
    }

    fn thread(id: &str) -> DirectThread {
        DirectThread {
            thread_id: id.to_string(),
            pk: None,
            thread_title: None,
            is_group: false,
            users: Vec::new(),
            items: Vec::new(),
            has_older: false,
            prev_cursor: None,
            oldest_cursor: None,
        }
    }

    fn run_targets(app: &mut AppState) -> Vec<TargetResolution> {
        app.form.sessionid = "fake_sessionid".to_string();
        app.form.dry_run = true;
        app.form.limit = "0".to_string();
        app.form.throttle_ms = "2000".to_string();
        app.form.scan_throttle_ms = "350".to_string();
        app.screen = Screen::Settings;
        let effects = app.apply_action(Action::Activate);
        match effects.into_iter().next() {
            Some(Effect::StartRun(cfg)) => cfg.targets,
            other => panic!("expected StartRun, got {other:?}"),
        }
    }

    #[test]
    fn picked_threads_override_blanket_scope() {
        let mut app = AppState::new(Config::from_env());
        app.form.scope.kind = ScopeKind::All;
        app.targets.threads = vec![thread("t1"), thread("t2"), thread("t3")];
        app.targets.selected_threads.insert("t3".to_string());
        app.targets.selected_threads.insert("t1".to_string());

        match run_targets(&mut app).as_slice() {
            [TargetResolution::ThreadObjects(picked)] => {
                let ids: Vec<&str> = picked.iter().map(|t| t.thread_id.as_str()).collect();
                assert_eq!(ids, ["t1", "t3"]);
            }
            other => panic!("expected only picked threads, got {other:?}"),
        }
    }

    #[test]
    fn empty_picker_keeps_scope_targets() {
        let mut app = AppState::new(Config::from_env());
        app.form.scope.kind = ScopeKind::All;
        app.targets.threads = vec![thread("t1")];

        let targets = run_targets(&mut app);
        assert!(matches!(
            targets.as_slice(),
            [TargetResolution::Posts, TargetResolution::AllDms]
        ));
    }

    #[test]
    fn dms_scope_is_narrowed_by_picks() {
        let mut app = AppState::new(Config::from_env());
        app.form.scope.kind = ScopeKind::Dms;
        app.targets.threads = vec![thread("t1"), thread("t2")];
        app.targets.selected_threads.insert("t2".to_string());

        match run_targets(&mut app).as_slice() {
            [TargetResolution::ThreadObjects(picked)] => {
                assert_eq!(picked.len(), 1);
                assert_eq!(picked[0].thread_id, "t2");
            }
            other => panic!("expected only picked threads, got {other:?}"),
        }
    }

    #[test]
    fn typed_thread_scope_beats_stale_picks() {
        let mut app = AppState::new(Config::from_env());
        app.form.scope.kind = ScopeKind::Threads;
        app.form.scope.threads = "typed_1, typed_2".to_string();
        app.targets.threads = vec![thread("t1")];
        app.targets.selected_threads.insert("t1".to_string());

        match run_targets(&mut app).as_slice() {
            [TargetResolution::Threads(ids)] => assert_eq!(ids, &["typed_1", "typed_2"]),
            other => panic!("expected the typed ids to win, got {other:?}"),
        }
    }

    #[test]
    fn posts_scope_ignores_picks() {
        let mut app = AppState::new(Config::from_env());
        app.form.scope.kind = ScopeKind::Posts;
        app.targets.threads = vec![thread("t1")];
        app.targets.selected_threads.insert("t1".to_string());

        assert!(matches!(
            run_targets(&mut app).as_slice(),
            [TargetResolution::Posts]
        ));
    }

    #[test]
    fn refreshed_inbox_drops_unresolvable_picks() {
        let mut app = AppState::new(Config::from_env());
        app.form.scope.kind = ScopeKind::All;
        app.targets.threads = vec![thread("t1"), thread("t2")];
        app.targets.selected_threads.insert("t1".to_string());
        app.targets.selected_threads.insert("t2".to_string());

        app.apply_event(PrunerEvent::DMsLoaded {
            channels: vec![thread("t2")],
            gen: 0,
            truncated: false,
        });

        assert_eq!(
            app.targets.selected_threads.iter().collect::<Vec<_>>(),
            vec!["t2"]
        );

        match run_targets(&mut app).as_slice() {
            [TargetResolution::ThreadObjects(picked)] => assert_eq!(picked.len(), 1),
            other => panic!("expected the surviving pick, got {other:?}"),
        }
    }

    #[test]
    fn mask_sessionid_hides_secrets() {
        assert_eq!(mask_sessionid(""), "(empty)");
        assert_eq!(mask_sessionid("123"), "••••••");
        assert_eq!(mask_sessionid("1234567890abcdef"), "••••••••••••cdef");
        assert_eq!(mask_sessionid("1234567890🔐"), "••••••••••••890🔐");
    }

    #[test]
    fn space_toggles_the_focused_content_class_into_the_run() {
        let mut app = AppState::new(Config::from_env());
        app.form.sessionid = "fake_sessionid".to_string();
        app.form.dry_run = true;
        app.form.scope.kind = ScopeKind::None;
        app.screen = Screen::Settings;

        assert!(!app.form.content.any());

        app.settings.cursor = Field::ALL
            .iter()
            .position(|f| *f == Field::ContentLikes)
            .expect("content field present");
        app.apply_action(Action::Toggle);

        assert!(app.form.content.likes);
        assert_eq!(app.form.content.label(), "likes");

        let cfg = app.form.to_pruner_config().expect("valid config");
        assert!(cfg.targets.is_empty());
        assert!(cfg.content.likes && cfg.content.count() == 1);

        app.apply_action(Action::Toggle);
        assert!(!app.form.content.any());
        assert!(app.form.validate().contains_key(&Field::Scope));
    }
}
