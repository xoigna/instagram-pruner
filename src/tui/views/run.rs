use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, ListItem, Paragraph, Sparkline};
use ratatui::Frame;

use crate::events::{LogEntry, Phase};

use super::super::state::{AppState, RunStatus};
use super::super::theme;
use super::super::widgets::{clock, ellipsize, human_duration, level_tag, panel, split, split_v};

pub fn draw(f: &mut Frame, app: &AppState, area: Rect) {
    let parts = split_v(area, &[Constraint::Length(7), Constraint::Min(6)]);
    draw_summary(f, app, parts[0]);
    let bottom = split(parts[1], &[Constraint::Percentage(68), Constraint::Min(20)]);
    draw_log(f, app, bottom[0]);
    draw_stats(f, app, bottom[1]);
}

fn status_line(app: &AppState) -> Line<'static> {
    let run = &app.run;
    let (text, style) = match run.status {
        RunStatus::Idle => ("idle — press F5 to start run", theme::dim()),
        RunStatus::Running => match run.phase {
            Phase::Scan => ("scanning targets for deletable items…", theme::warn()),
            Phase::Delete => ("deleting items…", theme::accent()),
        },
        RunStatus::Completed => ("completed", theme::ok()),
        RunStatus::Cancelled => ("cancelled by user", theme::warn()),
        RunStatus::Failed => ("failed", theme::err()),
    };
    let dry = if app.form.dry_run { " · DRY RUN" } else { "" };
    Line::from(vec![
        Span::styled("Status: ", theme::dim()),
        Span::styled(format!("{text}{dry}"), style.add_modifier(Modifier::BOLD)),
    ])
}

fn draw_summary(f: &mut Frame, app: &AppState, area: Rect) {
    let block = panel("Prune progress", false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let parts = split_v(
        inner,
        &[
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
        ],
    );

    f.render_widget(Paragraph::new(status_line(app)), parts[0]);

    let (ratio, label) = if app.run.plan_total > 0 {
        let r = (app.run.plan_done as f64 / app.run.plan_total as f64).clamp(0.0, 1.0);
        let pct = (r * 100.0) as u64;
        (
            r,
            format!(
                "{pct}% ({}/{} items)",
                app.run.plan_done, app.run.plan_total
            ),
        )
    } else if app.run.is_running() {
        let scanned = app.run.stats.messages_scanned;
        (0.0, format!("scanning… {scanned} items inspected"))
    } else {
        (0.0, "ready".to_string())
    };

    let gauge_style = if app.form.dry_run {
        theme::warn()
    } else {
        theme::accent()
    };
    let gauge = Gauge::default()
        .gauge_style(gauge_style)
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, parts[1]);

    let stats = &app.run.stats;
    let elapsed = human_duration(app.run.elapsed());
    let metrics = Line::from(vec![
        Span::styled("Scanned: ", theme::dim()),
        Span::styled(format!("{}  ", stats.messages_scanned), theme::base()),
        Span::styled("Deleted: ", theme::dim()),
        Span::styled(
            format!("{}  ", stats.messages_deleted),
            theme::ok().add_modifier(Modifier::BOLD),
        ),
        Span::styled("Failed: ", theme::dim()),
        Span::styled(
            format!("{}  ", stats.messages_failed),
            if stats.messages_failed > 0 {
                theme::err()
            } else {
                theme::dim()
            },
        ),
        Span::styled("429 hits: ", theme::dim()),
        Span::styled(
            format!("{}  ", stats.rate_limit_hits),
            if stats.rate_limit_hits > 0 {
                theme::warn()
            } else {
                theme::dim()
            },
        ),
        Span::styled("Time: ", theme::dim()),
        Span::styled(elapsed, theme::base()),
    ]);
    f.render_widget(Paragraph::new(metrics), parts[2]);
}

fn log_window(app: &AppState, height: usize) -> Vec<LogEntry> {
    let total = app.run.log.len();
    if total == 0 {
        return Vec::new();
    }
    let scroll = app.run.log_scroll.min(total.saturating_sub(1));
    let end = total - scroll;
    let start = end.saturating_sub(height);
    app.run
        .log
        .iter()
        .skip(start)
        .take(end - start)
        .cloned()
        .collect()
}

fn draw_log(f: &mut Frame, app: &AppState, area: Rect) {
    let scroll_tag = if app.run.log_scroll > 0 {
        format!(" (scroll: +{})", app.run.log_scroll)
    } else {
        String::new()
    };
    let title = format!(" Activity Log{scroll_tag} · j/k scroll · Ctrl-L clear ");
    let block = panel(title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let height = inner.height as usize;
    let entries = log_window(app, height);
    let items: Vec<ListItem> = entries
        .iter()
        .map(|entry| {
            let time_str = clock(entry.at);
            let tag = level_tag(entry.level);
            let style = theme::level(entry.level);
            let line = Line::from(vec![
                Span::styled(format!("{time_str} "), theme::dim()),
                Span::styled(format!("[{tag}] "), style.add_modifier(Modifier::BOLD)),
                Span::styled(entry.message.clone(), theme::base()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = ratatui::widgets::List::new(items).style(theme::base());
    f.render_widget(list, inner);
}

fn draw_stats(f: &mut Frame, app: &AppState, area: Rect) {
    let parts = split_v(area, &[Constraint::Min(6), Constraint::Length(6)]);
    draw_channel_table(f, app, parts[0]);
    draw_rate(f, app, parts[1]);
}

fn draw_channel_table(f: &mut Frame, app: &AppState, area: Rect) {
    let block = panel("Targets processed", false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();
    if let Some(prog) = &app.run.current_progress {
        let name = ellipsize(&prog.channel_name, 16);
        lines.push(Line::from(vec![
            Span::styled("▶ ", theme::accent()),
            Span::styled(format!("{name:<16}"), theme::strong()),
            Span::styled(format!(" del:{}", prog.deleted), theme::ok()),
        ]));
    }

    for (_, name, stats) in app.run.channels.iter().rev().take(inner.height as usize) {
        let n = ellipsize(name, 16);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{n:<16}"), theme::dim()),
            Span::styled(format!(" del:{}", stats.deleted), theme::dim()),
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled("No targets yet", theme::dim())));
    }

    f.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}

fn draw_rate(f: &mut Frame, app: &AppState, area: Rect) {
    let block = panel("Rate limiter (ms)", false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let parts = split_v(inner, &[Constraint::Length(1), Constraint::Min(2)]);
    let hits = app.run.rate_limiter.hits_429;
    let cd = app.run.rate_limiter.cooldown_remaining_ms;
    let status = Line::from(vec![
        Span::styled(
            format!("Hits: {hits}  "),
            if hits > 0 {
                theme::warn()
            } else {
                theme::dim()
            },
        ),
        Span::styled(
            format!("Cooldown: {cd}ms"),
            if cd > 0 { theme::err() } else { theme::dim() },
        ),
    ]);
    f.render_widget(Paragraph::new(status), parts[0]);

    let data: Vec<u64> = app.run.sparkline_data.iter().copied().collect();
    let spark = Sparkline::default().style(theme::accent()).data(&data);
    f.render_widget(spark, parts[1]);
}
