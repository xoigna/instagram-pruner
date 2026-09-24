use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::super::action::{self, Overlay};
use super::super::state::AppState;
use super::super::theme;
use super::super::widgets::{centered_rect, panel, split_v};

pub fn draw(f: &mut Frame, app: &AppState, area: Rect) {
    match app.overlay {
        Overlay::None => {}
        Overlay::Palette => draw_palette(f, app, area),
        Overlay::Confirm => draw_confirm(f, app, area),
        Overlay::Help => draw_help(f, area),
    }
}

fn draw_palette(f: &mut Frame, app: &AppState, area: Rect) {
    let win = centered_rect(60, 45, area);
    f.render_widget(Clear, win);
    let block = panel(
        Line::from(vec![
            Span::styled(" Command palette ", theme::strong()),
            Span::styled("· type to filter · Enter run · Esc close ", theme::dim()),
        ]),
        true,
    );
    let inner = block.inner(win);
    f.render_widget(block, win);

    let prompt = Line::from(vec![
        Span::styled("> ", theme::accent().add_modifier(Modifier::BOLD)),
        Span::styled(app.palette.query.clone(), theme::strong()),
        Span::styled("▏", theme::accent().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    let parts = split_v(inner, &[Constraint::Length(1), Constraint::Min(1)]);
    f.render_widget(Paragraph::new(prompt), parts[0]);

    let items: Vec<ListItem> = app
        .palette
        .matches
        .iter()
        .filter_map(|&i| action::commands().into_iter().nth(i))
        .map(|cmd| {
            let line = Line::from(vec![
                Span::styled(format!("{:<18}", cmd.label), theme::strong()),
                Span::styled(format!(" {}", cmd.hint), theme::dim()),
            ]);
            ListItem::new(line)
        })
        .collect();

    if items.is_empty() {
        let empty = Paragraph::new("No matching commands").style(theme::dim());
        f.render_widget(empty, parts[1]);
        return;
    }

    let mut state = ListState::default();
    state.select(Some(app.palette.cursor.min(items.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .style(theme::base())
            .highlight_style(theme::selected())
            .highlight_symbol("▶ "),
        parts[1],
        &mut state,
    );
}

fn draw_confirm(f: &mut Frame, app: &AppState, area: Rect) {
    let win = centered_rect(56, 32, area);
    f.render_widget(Clear, win);
    let block = panel(
        Span::styled(
            " Confirm Permanent Deletion ",
            theme::err().add_modifier(Modifier::BOLD),
        ),
        true,
    );
    let inner = block.inner(win);
    f.render_widget(block, win);

    let summary = app.form.scope.summary();
    let lines = vec![
        Line::from(vec![
            Span::styled("WARNING: ", theme::err().add_modifier(Modifier::BOLD)),
            Span::styled("You are about to launch a LIVE prune run.", theme::strong()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Scope: ", theme::dim()),
            Span::styled(summary, theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("Items will be ", theme::base()),
            Span::styled(
                "PERMANENTLY DELETED",
                theme::err().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" from your Instagram account.", theme::base()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Press ", theme::dim()),
            Span::styled("Enter", theme::key()),
            Span::styled(" or ", theme::dim()),
            Span::styled("y", theme::key()),
            Span::styled(" to proceed, or ", theme::dim()),
            Span::styled("Esc", theme::key()),
            Span::styled(" / ", theme::dim()),
            Span::styled("n", theme::key()),
            Span::styled(" to cancel.", theme::dim()),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let win = centered_rect(68, 82, area);
    f.render_widget(Clear, win);
    let block = panel(" Keyboard Shortcuts · Esc/? close ", true);
    let inner = block.inner(win);
    f.render_widget(block, win);

    let sections: [(&str, &[(&str, &str)]); 6] = [
        (
            "Navigation",
            &[
                (
                    "1 - 4",
                    "Switch screen tabs (Settings / Targets / Run / Archive)",
                ),
                (
                    "Tab / BackTab",
                    "Move focus between fields, columns, or views",
                ),
                (
                    "j / k, Down / Up",
                    "Navigate rows, items, or scroll active log",
                ),
                ("h / l", "Previous / Next screen tab"),
            ],
        ),
        (
            "Settings & Editing",
            &[
                ("Enter / i / e", "Edit text field or cycle choice"),
                (
                    "Space",
                    "Toggle checkbox (dry-run, hide-threads, content classes)",
                ),
                ("Left / Right, [ / ]", "Cycle choice value"),
                ("Enter / Tab", "Commit edit in text editor"),
                ("Esc / Ctrl-C", "Cancel text edit buffer"),
            ],
        ),
        (
            "Targets & Protection",
            &[
                (
                    "Space",
                    "Pick thread — narrows dms/all runs to the picked ones",
                ),
                ("P", "Protect / unprotect focused thread from deletion"),
                ("a / u / v", "Pick all / none / invert selection"),
                ("Enter", "Copy picked threads into the thread-ids scope"),
                ("R", "Probe / reconnect account and refresh threads"),
            ],
        ),
        (
            "Execution & History",
            &[
                ("F5 / Enter", "Start prune run with current settings"),
                ("Esc / F6", "Cancel active run at next checkpoint"),
                ("D / d", "Toggle dry-run mode"),
                ("y", "Copy selected archive record JSON to clipboard"),
                ("]", "Cycle archive bucket (all / live / dry-run)"),
            ],
        ),
        (
            "What gets deleted",
            &[
                (
                    "scope",
                    "posts / all DMs / all / none / threads. 'none' runs only the content classes",
                ),
                (
                    "unlike all",
                    "Remove every like in your liked feed (feed/liked/)",
                ),
                ("unsave all", "Remove every saved post (feed/saved/posts/)"),
                (
                    "delete comments",
                    "Your own comments on your own posts (walks your feed)",
                ),
                ("delete archived", "Permanently delete archived media"),
                (
                    "undo reposts",
                    "Unsave reposted media; needs a repost collection",
                ),
                ("limit", "Cap planned items across the run; 0 = unlimited"),
                ("before / after", "Only items inside this date window"),
            ],
        ),
        (
            "Global",
            &[
                (": / p", "Open command palette"),
                ("?", "Show this help overlay"),
                ("q / Ctrl-C", "Quit application"),
                ("Ctrl-L", "Clear run activity log"),
            ],
        ),
    ];

    let mut lines = Vec::new();
    for (title, items) in sections {
        lines.push(Line::from(Span::styled(
            format!("── {title} ──────────────────────────────────────"),
            theme::accent().add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in items {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<18}", key), theme::key()),
                Span::styled(desc.to_string(), theme::base()),
            ]));
        }
        lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}
