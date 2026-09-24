#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub action: Action,
}

impl Command {
    pub const fn new(
        id: &'static str,
        label: &'static str,
        hint: &'static str,
        action: Action,
    ) -> Self {
        Self {
            id,
            label,
            hint,
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,

    Help,

    Palette,

    PaletteDown,

    PaletteUp,

    PaletteSelect,

    Back,

    PrevScreen,
    NextScreen,

    Goto(Screen),

    Down,
    Up,

    PageDown,
    PageUp,

    Top,
    Bottom,

    CycleNext,
    CyclePrev,

    NextField,
    PrevField,

    Activate,

    Toggle,

    ToggleProtect,

    ToggleDryRun,

    StartEdit,

    CommitEdit,

    CancelEdit,

    Input(char),
    InputBackspace,
    InputDelete,
    InputLeft,
    InputRight,
    InputHome,
    InputEnd,
    InputClear,

    SelectAll,
    SelectNone,
    SelectInvert,

    Probe,

    Refresh,

    Confirm,

    Decline,

    ToggleDetail,

    Yank,

    ClearLog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Settings,
    Targets,
    Run,
    Archive,
}

impl Screen {
    pub const ALL: [Screen; 4] = [
        Screen::Settings,
        Screen::Targets,
        Screen::Run,
        Screen::Archive,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Settings => "settings",
            Screen::Targets => "targets",
            Screen::Run => "run",
            Screen::Archive => "archive",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Palette,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    SessionId,
    Scope,
    Payload,
    Before,
    After,
    Limit,
    Throttle,
    ScanThrottle,

    Concurrency,
    DmKind,
    DryRun,
    HideThreads,
    Protect,

    ContentLikes,
    ContentSaved,
    ContentComments,
    ContentArchived,
    ContentReposts,
}

impl Field {
    pub const ALL: [Field; 18] = [
        Field::SessionId,
        Field::Scope,
        Field::Payload,
        Field::Before,
        Field::After,
        Field::Limit,
        Field::Throttle,
        Field::ScanThrottle,
        Field::Concurrency,
        Field::DmKind,
        Field::DryRun,
        Field::HideThreads,
        Field::Protect,
        Field::ContentLikes,
        Field::ContentSaved,
        Field::ContentComments,
        Field::ContentArchived,
        Field::ContentReposts,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::SessionId => "sessionid",
            Field::Scope => "scope",
            Field::Payload => "thread ids",
            Field::Before => "before",
            Field::After => "after",
            Field::Limit => "limit",
            Field::Throttle => "throttle (ms)",
            Field::ScanThrottle => "scan throttle",
            Field::Concurrency => "scan parallel",
            Field::DmKind => "dm kind",
            Field::DryRun => "dry run",
            Field::HideThreads => "hide threads",
            Field::Protect => "protect users",
            Field::ContentLikes => "unlike all",
            Field::ContentSaved => "unsave all",
            Field::ContentComments => "delete comments",
            Field::ContentArchived => "delete archived",
            Field::ContentReposts => "undo reposts",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Field::SessionId => "browser sessionid cookie (masked); env wins",
            Field::Scope => {
                "posts / all DMs / all / threads (left/right to cycle); picks narrow dms/all"
            }
            Field::Payload => "comma-separated thread IDs for threads scope",
            Field::Before => "only items older than this (RFC3339 or YYYY-MM-DD)",
            Field::After => "only items newer than this",
            Field::Limit => "maximum planned items across the run; 0 = unlimited",
            Field::Throttle => "minimum ms between DELETE requests (safe: 2000)",
            Field::ScanThrottle => "minimum ms between GET/scan requests (safe: 350)",
            Field::Concurrency => {
                "concurrent scan requests 1-8; divides scan throttle, so reads run N× faster"
            }
            Field::DmKind => "DM filter: all / 1on1 / group",
            Field::DryRun => "plan + archive only, never call DELETE",
            Field::HideThreads => "inbox-hide emptied threads after a live DM run",
            Field::Protect => "never prune DMs with these user IDs/usernames (comma-separated)",
            Field::ContentLikes => "unlike every post in your liked feed",
            Field::ContentSaved => "remove every post from your saved items",
            Field::ContentComments => {
                "delete your own comments from your own posts (walks your feed)"
            }
            Field::ContentArchived => "permanently delete media in your archive",
            Field::ContentReposts => {
                "unsave reposted media; needs a repost collection (Instagram has no repost API)"
            }
        }
    }

    pub fn is_text(self) -> bool {
        matches!(
            self,
            Field::SessionId
                | Field::Payload
                | Field::Before
                | Field::After
                | Field::Limit
                | Field::Throttle
                | Field::ScanThrottle
                | Field::Concurrency
                | Field::Protect
        )
    }

    #[allow(dead_code)]
    pub fn is_toggle(self) -> bool {
        matches!(
            self,
            Field::DryRun
                | Field::HideThreads
                | Field::ContentLikes
                | Field::ContentSaved
                | Field::ContentComments
                | Field::ContentArchived
                | Field::ContentReposts
        )
    }

    #[allow(dead_code)]
    pub fn is_choice(self) -> bool {
        matches!(self, Field::Scope | Field::DmKind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetsTab {
    #[default]
    Threads,
    Posts,
}

#[allow(dead_code)]
impl TargetsTab {
    pub fn label(self) -> &'static str {
        match self {
            TargetsTab::Threads => "Direct Threads",
            TargetsTab::Posts => "Posts / Feed",
        }
    }

    pub fn next(self) -> Self {
        match self {
            TargetsTab::Threads => TargetsTab::Posts,
            TargetsTab::Posts => TargetsTab::Threads,
        }
    }

    pub fn prev(self) -> Self {
        self.next()
    }
}

#[derive(Debug)]
pub enum Effect {
    Quit,

    Probe,

    LoadTargets,

    StartRun(crate::pruner::PrunerConfig),
    CancelRun,

    Yank(String),
}

pub fn commands() -> Vec<Command> {
    vec![
        Command::new(
            "settings",
            "Open settings",
            "edit scope, filters and behaviour",
            Action::Goto(Screen::Settings),
        ),
        Command::new(
            "targets",
            "Open targets",
            "browse and select threads / posts to prune",
            Action::Goto(Screen::Targets),
        ),
        Command::new(
            "probe",
            "Connect / refresh account",
            "fetch identity, DM threads and posts from Instagram",
            Action::Probe,
        ),
        Command::new(
            "start",
            "Start prune run",
            "launch the pruner with current settings",
            Action::Activate,
        ),
        Command::new(
            "cancel",
            "Cancel run",
            "stop at the next delete checkpoint",
            Action::Back,
        ),
        Command::new(
            "open-run",
            "Open run screen",
            "live progress, per-target stats and log",
            Action::Goto(Screen::Run),
        ),
        Command::new(
            "archive",
            "Open archive",
            "browse the on-disk deletion history",
            Action::Goto(Screen::Archive),
        ),
        Command::new(
            "refresh-archive",
            "Reload archive",
            "re-read the history directory",
            Action::Refresh,
        ),
        Command::new(
            "dry-run",
            "Toggle dry run",
            "plan-only, never DELETE",
            Action::ToggleDryRun,
        ),
        Command::new("help", "Show key bindings", "help overlay", Action::Help),
        Command::new("quit", "Quit", "exit the TUI", Action::Quit),
    ]
}
