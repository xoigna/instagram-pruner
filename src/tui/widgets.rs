use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

use crate::events::LogLevel;

use super::action::Screen;
use super::state::{AppState, Connection};
use super::theme;

pub fn panel<'a>(title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
    let border_style = if focused {
        theme::focus()
    } else {
        theme::border()
    };
    Block::new()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .border_style(border_style)
        .title_style(if focused {
            theme::strong()
        } else {
            theme::dim().add_modifier(Modifier::BOLD)
        })
        .title(title)
        .padding(Padding::horizontal(1))
}

pub fn hints(items: &[(&str, &str)]) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, label)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme::base()));
        }
        spans.push(Span::styled(format!("[{key}]"), theme::key()));
        spans.push(Span::styled(format!(" {label}"), theme::dim()));
    }
    Line::from(spans)
}

pub fn badge(text: &str, style: Style) -> Span<'static> {
    Span::styled(format!(" {text} "), style.add_modifier(Modifier::BOLD))
}

pub fn tab_line(current: Screen) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    for (i, screen) in Screen::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("·", theme::border()));
        }
        let num = i + 1;
        let is_cur = *screen == current;
        let style = if is_cur {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            theme::dim()
        };
        let num_style = if is_cur {
            theme::accent().add_modifier(Modifier::BOLD)
        } else {
            theme::key()
        };
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("{num}:"), num_style));
        spans.push(Span::styled(format!("{} ", screen.title()), style));
    }
    Line::from(spans)
}

pub fn status_spans(app: &AppState) -> Vec<Span<'static>> {
    let mut spans: Vec<Span> = Vec::new();
    if app.form.dry_run {
        spans.push(badge("DRY RUN", theme::warn()));
        spans.push(Span::raw(" "));
    }
    if app.run.is_running() {
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let phase = match app.run.phase {
            crate::events::Phase::Scan => "scanning",
            crate::events::Phase::Delete => "deleting",
        };
        let idx = (app.run.plan_done + app.run.channels.len()) % frames.len();
        spans.push(Span::styled(
            format!("{} {phase} ", frames[idx]),
            theme::accent().add_modifier(Modifier::BOLD),
        ));
    }
    let (text, style) = match (&app.connection, &app.identity) {
        (Connection::Ready, Some(id)) => (format!("@{}", id.username), theme::ok()),
        (Connection::Ready, None) => ("connected".to_string(), theme::ok()),
        (Connection::Connecting, _) => ("connecting…".to_string(), theme::warn()),
        (Connection::Failed, _) => ("offline".to_string(), theme::err()),
        (Connection::Idle, _) => ("not connected".to_string(), theme::dim()),
    };
    spans.push(Span::styled(text, style));
    spans
}

pub fn draw_header(f: &mut Frame, app: &AppState, area: Rect) {
    let right = status_spans(app);
    let right_width: u16 = right.iter().map(|s| s.width() as u16).sum();
    let areas = split(
        area,
        &[Constraint::Min(1), Constraint::Length(right_width.max(1))],
    );
    f.render_widget(Paragraph::new(tab_line(app.screen)), areas[0]);
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        areas[1],
    );
}

pub fn draw_footer(f: &mut Frame, app: &AppState, area: Rect) {
    let hints = footer_hints(app);
    let hint_width = hints.width() as u16;
    let toast_width = area.width.saturating_sub(hint_width);
    let areas = split(area, &[Constraint::Min(1), Constraint::Length(toast_width)]);
    f.render_widget(Paragraph::new(hints), areas[0]);
    if let Some(toast) = &app.toast {
        let style = theme::level(toast.level).add_modifier(Modifier::BOLD);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(toast.text.clone(), style)))
                .alignment(Alignment::Right),
            areas[1],
        );
    }
}

pub fn footer_hints(app: &AppState) -> Line<'static> {
    let items: Vec<(&str, &str)> = if app.run.is_running() {
        vec![
            ("Esc", "cancel run"),
            ("1-4", "screens"),
            ("j/k", "log"),
            ("?", "help"),
        ]
    } else {
        match app.screen {
            Screen::Settings => vec![
                ("1-4", "screens"),
                ("j/k", "move"),
                ("Enter", "edit/cycle"),
                ("Space", "toggle"),
                ("Tab", "field"),
                ("F5", "run"),
            ],
            Screen::Targets => vec![
                ("1-4", "screens"),
                ("j/k", "move"),
                ("Space", "select"),
                ("P", "protect"),
                ("Enter", "use selection"),
                ("Tab", "threads/posts"),
                ("R", "reconnect"),
            ],
            Screen::Run => vec![
                ("1-4", "screens"),
                ("F5", "run"),
                ("Esc", "cancel"),
                ("j/k", "scroll log"),
                ("Ctrl-L", "clear log"),
            ],
            Screen::Archive => vec![
                ("1-4", "screens"),
                ("Tab", "column"),
                ("j/k", "move"),
                ("Enter", "detail"),
                ("y", "yank"),
                ("]", "scope"),
            ],
        }
    };
    hints(&items)
}

pub fn centered_rect(width_pct: u16, height_pct: u16, area: Rect) -> Rect {
    let vertical = ratatui::layout::Layout::vertical([
        Constraint::Percentage((100 - height_pct) / 2),
        Constraint::Percentage(height_pct),
        Constraint::Percentage((100 - height_pct) / 2),
    ])
    .split(area);
    let horizontal = ratatui::layout::Layout::horizontal([
        Constraint::Percentage((100 - width_pct) / 2),
        Constraint::Percentage(width_pct),
        Constraint::Percentage((100 - width_pct) / 2),
    ])
    .split(vertical[1]);
    horizontal[1]
}

pub fn split(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    ratatui::layout::Layout::horizontal(constraints.to_vec())
        .split(area)
        .to_vec()
}

pub fn split_v(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    ratatui::layout::Layout::vertical(constraints.to_vec())
        .split(area)
        .to_vec()
}

pub fn kv(label: &str, value: Span<'static>, label_width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<label_width$}"), theme::dim()),
        value,
    ])
}

pub fn clock(at: std::time::SystemTime) -> String {
    use chrono::{DateTime, Local};
    let dt: DateTime<Local> = at.into();
    dt.format("%H:%M:%S").to_string()
}

pub fn human_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn ellipsize(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn level_tag(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "info",
        LogLevel::Success => "done",
        LogLevel::Warning => "warn",
        LogLevel::Error => "err ",
    }
}

pub fn wrap() -> Wrap {
    Wrap { trim: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_duration_formats_each_magnitude() {
        use std::time::Duration;
        assert_eq!(human_duration(Duration::from_secs(9)), "9s");
        assert_eq!(human_duration(Duration::from_secs(64)), "1m 04s");
        assert_eq!(human_duration(Duration::from_secs(3720)), "1h 02m");
    }

    #[test]
    fn split_partitions_the_full_area() {
        let area = Rect::new(0, 0, 80, 10);
        let parts = split(area, &[Constraint::Length(10), Constraint::Min(1)]);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].width, 10);
        assert_eq!(parts[0].width + parts[1].width, 80);
        assert_eq!(parts[1].x, 10);
    }
}
