use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use super::super::state::{AppState, Connection};
use super::super::theme;
use super::super::widgets::{ellipsize, panel, split_v};
use crate::model::DirectThread;
pub fn draw(f: &mut Frame, app: &AppState, area: Rect) {
    let parts = split_v(area, &[Constraint::Min(8), Constraint::Length(5)]);
    draw_list(f, app, parts[0]);
    draw_summary(f, app, parts[1]);
}

fn recipients(t: &DirectThread) -> String {
    t.display_name()
}

fn last_snippet(t: &DirectThread) -> String {
    if let Some(item) = t.items.first() {
        if let Some(txt) = &item.text {
            return txt.replace('\n', " ");
        }
        if let Some(t_type) = &item.item_type {
            return format!("[{t_type}]");
        }
        format!("[item:{}]", item.item_id)
    } else {
        "(empty)".to_string()
    }
}

fn draw_list(f: &mut Frame, app: &AppState, area: Rect) {
    let threads = app.targets.filtered_threads(app.form.dm_kind);
    let title = format!(
        " Direct Threads ({} available) · Space pick · P protect · Enter copy to scope ",
        threads.len()
    );
    let block = panel(title, true);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.targets.loading {
        let msg = Paragraph::new("Loading inbox threads from Instagram…").style(theme::accent());
        f.render_widget(msg, inner);
        return;
    }

    if threads.is_empty() {
        let msg = Paragraph::new(
            "No threads found matching filter. Press 'R' to reconnect or check DM kind in Settings.",
        )
        .style(theme::dim());
        f.render_widget(msg, inner);
        return;
    }

    let header_cells = [
        "",
        "Sel",
        "Prot",
        "Type",
        "Title / Participants",
        "Last item",
        "Thread ID",
    ]
    .into_iter()
    .map(|h| Cell::from(Span::styled(h, theme::strong())));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = threads.iter().enumerate().map(|(i, thread)| {
        let is_selected_cursor = i == app.targets.thread_cursor;
        let is_checked = app.targets.selected_threads.contains(&thread.thread_id);
        let is_prot = app.targets.is_protected(thread);

        let pointer = if is_selected_cursor { "▶" } else { " " };
        let sel_box = if is_checked { "[✓]" } else { "[ ]" };
        let prot_tag = if is_prot { "[P]" } else { "   " };
        let type_tag = if thread.is_group { "group" } else { "1on1" };

        let title_str = ellipsize(&recipients(thread), 26);
        let snippet = ellipsize(&last_snippet(thread), 24);

        let style = if is_selected_cursor {
            theme::selected()
        } else {
            theme::base()
        };

        Row::new(vec![
            Cell::from(Span::styled(pointer, theme::accent())),
            Cell::from(Span::styled(
                sel_box,
                if is_checked {
                    theme::ok().add_modifier(Modifier::BOLD)
                } else {
                    theme::dim()
                },
            )),
            Cell::from(Span::styled(
                prot_tag,
                if is_prot {
                    theme::warn().add_modifier(Modifier::BOLD)
                } else {
                    theme::dim()
                },
            )),
            Cell::from(Span::styled(type_tag, theme::dim())),
            Cell::from(Span::styled(title_str, theme::strong())),
            Cell::from(Span::styled(snippet, theme::dim())),
            Cell::from(Span::styled(thread.thread_id.clone(), theme::dim())),
        ])
        .style(style)
    });

    let widths = [
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(6),
        Constraint::Length(28),
        Constraint::Min(20),
        Constraint::Length(22),
    ];

    let mut state = TableState::default();
    state.select(Some(app.targets.thread_cursor));
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected());
    f.render_stateful_widget(table, inner, &mut state);
}

fn draw_summary(f: &mut Frame, app: &AppState, area: Rect) {
    let block = panel("Selection & Status", false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sel_count = app.targets.selected_threads.len();
    let prot_count = app.targets.protected_users.len();
    let threads_count = app.targets.threads.len();

    let conn_span = match (&app.connection, &app.identity) {
        (Connection::Ready, Some(id)) => {
            Span::styled(format!("Connected as @{}", id.username), theme::ok())
        }
        (Connection::Connecting, _) => Span::styled("Connecting to Instagram…", theme::warn()),
        (Connection::Failed, _) => Span::styled("Offline / probe failed", theme::err()),
        (Connection::Idle, _) => Span::styled("Not connected (press R to probe)", theme::dim()),
        _ => Span::styled("Connected", theme::ok()),
    };

    let line1 = Line::from(vec![
        Span::styled("Account: ", theme::dim()),
        conn_span,
        Span::raw("   ·   "),
        Span::styled(format!("{threads_count} threads loaded"), theme::base()),
        Span::raw("   ·   "),
        Span::styled(
            format!("{sel_count} selected"),
            if sel_count > 0 {
                theme::ok().add_modifier(Modifier::BOLD)
            } else {
                theme::dim()
            },
        ),
        Span::raw("   ·   "),
        Span::styled(format!("{prot_count} protected"), theme::warn()),
    ]);

    let line2 = match (app.picker_override(), sel_count) {
        (Some(picked), _) => Line::from(vec![
            Span::styled("Run: ", theme::dim()),
            Span::styled(
                format!("only {} picked thread(s)", picked.len()),
                theme::ok().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" — scope ignored.", theme::dim()),
        ]),
        (None, 0) => Line::from(vec![
            Span::styled("Run: ", theme::dim()),
            Span::styled("the scope decides; ", theme::dim()),
            Span::styled("Space", theme::key()),
            Span::styled(" picks a thread.", theme::dim()),
        ]),
        (None, n) => Line::from(vec![
            Span::styled("Run: ", theme::dim()),
            Span::styled(
                format!("{} scope decides; ", app.form.scope.kind.label()),
                theme::dim(),
            ),
            Span::styled("Enter", theme::key()),
            Span::styled(format!(" copies {n} picked."), theme::dim()),
        ]),
    };

    f.render_widget(
        Paragraph::new(vec![line1, line2]).style(theme::base()),
        inner,
    );
}
