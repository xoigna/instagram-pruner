use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::history::DeletedRecord;

use super::super::state::{AppState, ArchiveFocus};
use super::super::theme;
use super::super::widgets::{ellipsize, panel, split, split_v, wrap};

pub fn draw(f: &mut Frame, app: &AppState, area: Rect) {
    let parts = split_v(area, &[Constraint::Min(8), Constraint::Length(6)]);
    let top = split(
        parts[0],
        &[
            Constraint::Length(18),
            Constraint::Length(26),
            Constraint::Min(30),
        ],
    );
    draw_days(f, app, top[0]);
    draw_channels(f, app, top[1]);
    draw_records(f, app, top[2]);
    draw_detail(f, app, parts[1]);
}

fn draw_days(f: &mut Frame, app: &AppState, area: Rect) {
    let is_focused = app.archive.focus == ArchiveFocus::Days;
    let title = format!(" Days ({}) ", app.archive.days.len());
    let block = panel(title, is_focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .archive
        .days
        .iter()
        .enumerate()
        .map(|(i, day)| {
            let is_cur = i == app.archive.day_cursor;
            let style = if is_cur && is_focused {
                theme::selected()
            } else if is_cur {
                theme::strong()
            } else {
                theme::base()
            };
            let pointer = if is_cur { "▶ " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(pointer, theme::accent()),
                Span::styled(day.to_string(), style),
            ]))
        })
        .collect();

    f.render_widget(List::new(items).style(theme::base()), inner);
}

fn draw_channels(f: &mut Frame, app: &AppState, area: Rect) {
    let is_focused = app.archive.focus == ArchiveFocus::Channels;
    let title = format!(" Targets ({}) ", app.archive.channels.len());
    let block = panel(title, is_focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .archive
        .channels
        .iter()
        .enumerate()
        .map(|(i, ch)| {
            let is_cur = i == app.archive.channel_cursor;
            let style = if is_cur && is_focused {
                theme::selected()
            } else if is_cur {
                theme::strong()
            } else {
                theme::base()
            };
            let pointer = if is_cur { "▶ " } else { "  " };
            let name = ellipsize(&ch.name, 18);
            ListItem::new(Line::from(vec![
                Span::styled(pointer, theme::accent()),
                Span::styled(name, style),
            ]))
        })
        .collect();

    f.render_widget(List::new(items).style(theme::base()), inner);
}

fn draw_records(f: &mut Frame, app: &AppState, area: Rect) {
    let is_focused = app.archive.focus == ArchiveFocus::Records;
    let scope_tag = match app.archive.scope {
        crate::history::HistoryScope::All => "all",
        crate::history::HistoryScope::Live => "live",
        crate::history::HistoryScope::DryRun => "dry-run",
    };
    let title = format!(
        " Records ({}) · scope: [{scope_tag}] (] cycle) ",
        app.archive.records.len()
    );
    let block = panel(title, is_focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .archive
        .records
        .iter()
        .enumerate()
        .map(|(i, rec)| {
            let is_cur = i == app.archive.record_cursor;
            let style = if is_cur && is_focused {
                theme::selected()
            } else if is_cur {
                theme::strong()
            } else {
                theme::base()
            };
            let pointer = if is_cur { "▶ " } else { "  " };
            let outcome_style = theme::outcome(rec.outcome);
            let outcome_tag = format!("[{:?}]", rec.outcome);
            let text = ellipsize(
                if !rec.preview.is_empty() {
                    &rec.preview
                } else if !rec.content.is_empty() {
                    &rec.content
                } else {
                    &rec.item_id
                },
                36,
            );

            ListItem::new(Line::from(vec![
                Span::styled(pointer, theme::accent()),
                Span::styled(format!("{outcome_tag:<10}"), outcome_style),
                Span::styled(
                    format!(" {} ", rec.deleted_at.format("%H:%M:%S")),
                    theme::dim(),
                ),
                Span::styled(text, style),
            ]))
        })
        .collect();

    f.render_widget(List::new(items).style(theme::base()), inner);
}

fn draw_detail(f: &mut Frame, app: &AppState, area: Rect) {
    let block = panel("Record details · 'y' yank JSON to clipboard", false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(record) = app.archive.records.get(app.archive.record_cursor) {
        let lines = detail_lines(record);
        f.render_widget(
            Paragraph::new(lines).style(theme::base()).wrap(wrap()),
            inner,
        );
    } else {
        let msg = Paragraph::new("Select a record above to inspect full metadata and text.")
            .style(theme::dim());
        f.render_widget(msg, inner);
    }
}

fn detail_lines(record: &DeletedRecord) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let outcome_style = theme::outcome(record.outcome).add_modifier(Modifier::BOLD);

    lines.push(Line::from(vec![
        Span::styled("Item ID: ", theme::dim()),
        Span::styled(record.item_id.clone(), theme::strong()),
        Span::raw("   ·   "),
        Span::styled("Outcome: ", theme::dim()),
        Span::styled(format!("{:?}", record.outcome), outcome_style),
        Span::raw("   ·   "),
        Span::styled("Time: ", theme::dim()),
        Span::styled(record.timestamp.clone(), theme::base()),
        Span::raw("   ·   "),
        Span::styled("Target: ", theme::dim()),
        Span::styled(record.target_name.clone(), theme::strong()),
    ]));

    if !record.author_name.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Author: ", theme::dim()),
            Span::styled(format!("@{}", record.author_name), theme::accent()),
        ]));
    }

    if !record.content.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Content: ", theme::dim()),
            Span::styled(record.content.clone(), theme::base()),
        ]));
    }

    if let Some(att) = record.attachments.first() {
        lines.push(Line::from(vec![
            Span::styled("Attachment URL: ", theme::dim()),
            Span::styled(att.url.clone(), theme::dim()),
        ]));
    }

    lines
}
