use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::config::Config;
use crate::events::{LogLevel, PrunerEvent};
use crate::history::DeleteOutcome;
use crate::tui::action::{Overlay, Screen};
use crate::tui::state::AppState;
use crate::tui::views;

fn render(app: &AppState, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|f| views::render(f, app))
        .expect("draw to test terminal");
    buffer_lines(terminal.backend())
}

fn buffer_lines(backend: &TestBackend) -> Vec<String> {
    let buffer = backend.buffer();
    let mut lines = Vec::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            let cell = buffer.cell((x, y)).expect("valid cell");
            line.push_str(cell.symbol());
        }
        lines.push(line);
    }
    lines
}

fn text(lines: &[String]) -> String {
    lines.join("\n")
}

const SIZES: [(u16, u16); 3] = [(60, 20), (100, 32), (160, 45)];

fn app_with_data() -> AppState {
    let mut cfg = Config::from_env();
    cfg.sessionid = "fake_sessionid_123456789".into();
    let mut app = AppState::new(cfg);
    app.connection = crate::tui::state::Connection::Ready;
    app.identity = Some(crate::tui::state::Identity {
        user_id: "12345".into(),
        username: "my_user".into(),
        full_name: "My Name".into(),
    });
    app.targets.threads = vec![crate::model::DirectThread {
        thread_id: "thread_100".into(),
        pk: Some("thread_100".into()),
        thread_title: Some("alice_and_bob".into()),
        is_group: false,
        users: vec![crate::model::UserShort {
            pk: "999".into(),
            username: "alice".into(),
            full_name: "Alice A".into(),
        }],
        items: vec![crate::model::DirectMessage {
            item_id: "msg_1".into(),
            id: None,
            user_id: Some("999".into()),
            timestamp: 1700000000000000,
            item_type: Some("text".into()),
            text: Some("hello world".into()),
        }],
        has_older: false,
        prev_cursor: None,
        oldest_cursor: None,
    }];
    app.apply_event(PrunerEvent::Started { channel_count: 1 });
    app.apply_event(PrunerEvent::PlanReady { total: 2 });
    app.apply_event(PrunerEvent::ChannelStarted {
        channel_id: "thread_100".into(),
        channel_name: "alice".into(),
        index: 1,
        total: 1,
        phase: crate::events::Phase::Delete,
    });
    app.apply_event(PrunerEvent::MessageDeleted {
        channel_id: "thread_100".into(),
        message_id: "msg_1".into(),
        preview: "hello world".into(),
        dry_run: false,
        outcome: DeleteOutcome::Deleted,
        record: record(DeleteOutcome::Deleted),
    });
    app.apply_event(PrunerEvent::Log {
        level: LogLevel::Warning,
        message: "a warning line".into(),
    });
    app.run.status = crate::tui::state::RunStatus::Running;
    app.archive.days = vec![chrono::Utc::now().date_naive()];
    app.archive.records = vec![record(DeleteOutcome::Deleted)];
    app
}

fn record(outcome: DeleteOutcome) -> crate::history::DeletedRecord {
    crate::history::DeletedRecord {
        deleted_at: chrono::Utc::now(),
        outcome,
        target_id: "thread_100".into(),
        target_name: "alice".into(),
        item_id: "msg_1".into(),
        kind: "dm".into(),
        author_id: "12345".into(),
        author_name: "my_user".into(),
        content: "archived body".into(),
        timestamp: "2026-09-17T12:00:00Z".into(),
        media_type: None,
        attachments: Vec::new(),
        preview: "archived body".into(),
    }
}

#[test]
fn every_screen_renders_at_every_size() {
    for screen in Screen::ALL {
        for (w, h) in SIZES {
            let mut app = app_with_data();
            app.screen = screen;
            let lines = render(&app, w, h);
            let body = text(&lines);
            assert!(
                body.contains(screen.title()),
                "{screen:?} at {w}x{h} did not draw its tab:\n{body}"
            );
            assert!(
                body.trim().len() > 40,
                "{screen:?} at {w}x{h} rendered almost nothing:\n{body}"
            );
        }
    }
}

#[test]
fn settings_screen_shows_fields_and_validation_state() {
    let app = app_with_data();
    let lines = render(&app, 100, 32);
    let body = text(&lines);
    assert!(body.contains("sessionid"));
    assert!(body.contains("scope"));
    assert!(body.contains("throttle"));
}

#[test]
fn settings_keeps_the_focused_content_toggle_on_screen() {
    let mut app = app_with_data();
    app.screen = Screen::Settings;
    app.settings.cursor = crate::tui::action::Field::ALL
        .iter()
        .position(|f| *f == crate::tui::action::Field::ContentReposts)
        .expect("content field present");

    let lines = render(&app, 100, 20);
    let body = text(&lines);
    assert!(
        body.contains("undo reposts"),
        "focused content toggle scrolled off screen:\n{body}"
    );

    app.settings.cursor = 0;
    let body = text(&render(&app, 100, 20));
    assert!(body.contains("sessionid"), "top rows not restored:\n{body}");
}

#[test]
fn settings_editor_renders_unicode_cursor_safely() {
    let mut app = app_with_data();
    app.screen = Screen::Settings;
    app.form.before = "😀date".into();
    app.settings.cursor = crate::tui::action::Field::ALL
        .iter()
        .position(|field| *field == crate::tui::action::Field::Before)
        .expect("before field present");
    app.settings.start_edit("😀date");
    app.settings.buffer_cursor = 1;

    let body = text(&render(&app, 100, 24));
    assert!(body.contains("date"));
}

#[test]
fn run_screen_shows_progress_stats_and_log() {
    let mut app = app_with_data();
    app.screen = Screen::Run;
    let lines = render(&app, 100, 32);
    let body = text(&lines);
    assert!(body.contains("Status:"));
    assert!(body.contains("Scanned:"));
    assert!(body.contains("Deleted:"));
    assert!(body.contains("Activity Log"));
}

#[test]
fn targets_screen_lists_threads() {
    let mut app = app_with_data();
    app.screen = Screen::Targets;
    let lines = render(&app, 100, 32);
    let body = text(&lines);
    assert!(body.contains("Direct Threads"));
    assert!(body.contains("alice"));
}

#[test]
fn archive_screen_shows_days_and_records() {
    let mut app = app_with_data();
    app.screen = Screen::Archive;
    let lines = render(&app, 100, 32);
    let body = text(&lines);
    assert!(body.contains("Days"));
    assert!(body.contains("Targets"));
    assert!(body.contains("Records"));
}

#[test]
fn overlays_render_and_cover_the_screen() {
    for overlay in [Overlay::Help, Overlay::Palette, Overlay::Confirm] {
        let mut app = app_with_data();
        app.overlay = overlay;
        let lines = render(&app, 100, 32);
        let body = text(&lines);
        match overlay {
            Overlay::Help => assert!(body.contains("Keyboard Shortcuts")),
            Overlay::Palette => assert!(body.contains("Command palette")),
            Overlay::Confirm => assert!(body.contains("Confirm Permanent Deletion")),
            Overlay::None => {}
        }
    }
}
