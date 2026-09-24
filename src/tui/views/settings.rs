use ratatui::layout::{Constraint, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::action::Field;
use super::super::state::{mask_sessionid, AppState};
use super::super::theme;
use super::super::widgets::{kv, panel, split_v};
use crate::config::ScopeKind;

pub fn draw(f: &mut Frame, app: &AppState, area: Rect) {
    let parts = split_v(area, &[Constraint::Min(8), Constraint::Length(6)]);
    draw_fields(f, app, parts[0]);
    draw_context(f, app, parts[1]);
}

fn visible(field: Field, app: &AppState) -> bool {
    if field == Field::Payload {
        return app.form.scope.kind.has_payload();
    }
    true
}

fn draw_fields(f: &mut Frame, app: &AppState, area: Rect) {
    let editing = app.settings.editing;

    let header = if editing {
        vec![
            Span::styled(" Settings ", theme::strong()),
            Span::styled("· EDITING (Enter/Tab commit, Esc cancel) ", theme::warn()),
        ]
    } else {
        vec![
            Span::styled(" Settings ", theme::strong()),
            Span::styled(
                "· Tab/j/k move · Enter edit/cycle · Space toggle · F5 run ",
                theme::dim(),
            ),
        ]
    };
    let inner = panel(Line::from(header.clone()), !editing).inner(area);

    let focused = app.settings.field();
    let label_width = 17;
    let mut lines: Vec<Line> = Vec::new();

    let mut focused_line: Option<usize> = None;

    for field in Field::ALL {
        if !visible(field, app) {
            continue;
        }
        let is_cur = field == focused;
        if is_cur {
            focused_line = Some(lines.len());
        }
        let mut spans: Vec<Span> = Vec::new();

        if is_cur {
            spans.push(Span::styled("▶ ", theme::accent()));
        } else {
            spans.push(Span::raw("  "));
        }

        let label_style = if is_cur {
            theme::strong()
        } else {
            theme::dim()
        };
        spans.push(Span::styled(
            format!("{:<label_width$}", field.label()),
            label_style,
        ));

        if is_cur && editing {
            let buf = &app.settings.buffer;
            let chars: Vec<char> = buf.chars().collect();
            let cur = app.settings.buffer_cursor.min(chars.len());
            let before: String = chars[..cur].iter().collect();
            let after: String = chars[cur..].iter().collect();
            spans.push(Span::styled(before, theme::selected()));
            spans.push(Span::styled(
                "▏",
                theme::accent().add_modifier(Modifier::SLOW_BLINK),
            ));
            spans.push(Span::styled(after, theme::selected()));
        } else {
            match field {
                Field::SessionId => {
                    spans.push(Span::styled(
                        mask_sessionid(&app.form.sessionid),
                        theme::base(),
                    ));
                }
                Field::Scope => {
                    spans.push(Span::styled(
                        format!("[ {} ]", app.form.scope.kind.label()),
                        theme::accent().add_modifier(Modifier::BOLD),
                    ));
                    if let Some(picked) = app.picker_override() {
                        spans.push(Span::styled(
                            format!("  ⤳ limited to {} picked thread(s)", picked.len()),
                            theme::warn().add_modifier(Modifier::BOLD),
                        ));
                    } else if app.form.scope.kind == ScopeKind::Posts
                        && !app.targets.selected_threads.is_empty()
                    {
                        spans.push(Span::styled(
                            "  (picks apply to dms/all scopes)",
                            theme::dim(),
                        ));
                    }
                }
                Field::DmKind => {
                    spans.push(Span::styled(
                        format!("[ {} ]", app.form.dm_kind.label()),
                        theme::accent().add_modifier(Modifier::BOLD),
                    ));
                }
                Field::DryRun => {
                    let box_str = if app.form.dry_run {
                        "[✓] on"
                    } else {
                        "[ ] off"
                    };
                    let style = if app.form.dry_run {
                        theme::warn().add_modifier(Modifier::BOLD)
                    } else {
                        theme::dim()
                    };
                    spans.push(Span::styled(box_str, style));
                }
                Field::HideThreads => {
                    let box_str = if app.form.hide_threads {
                        "[✓] on"
                    } else {
                        "[ ] off"
                    };
                    spans.push(Span::styled(box_str, theme::base()));
                }
                Field::ContentLikes
                | Field::ContentSaved
                | Field::ContentComments
                | Field::ContentArchived
                | Field::ContentReposts => {
                    let on = match field {
                        Field::ContentLikes => app.form.content.likes,
                        Field::ContentSaved => app.form.content.saved,
                        Field::ContentComments => app.form.content.comments,
                        Field::ContentArchived => app.form.content.archived,
                        _ => app.form.content.reposts,
                    };
                    let box_str = if on { "[✓] on" } else { "[ ] off" };
                    let style = if on {
                        theme::warn().add_modifier(Modifier::BOLD)
                    } else {
                        theme::dim()
                    };
                    spans.push(Span::styled(box_str, style));
                }
                _ => {
                    let val = app.form.get_text(field);
                    let display = if val.trim().is_empty() {
                        "(none)".to_string()
                    } else {
                        val
                    };
                    let style = if is_cur {
                        theme::strong()
                    } else {
                        theme::base()
                    };
                    spans.push(Span::styled(display, style));
                }
            }
        }

        if let Some(err) = app.settings.errors.get(&field) {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(format!("(! {err})"), theme::err()));
        }

        lines.push(Line::from(spans));
    }

    let height = inner.height as usize;
    let total = lines.len();
    let offset = match focused_line {
        Some(line) if height > 0 && total > height => (line + 1).saturating_sub(height),
        _ => 0,
    };

    let mut header = header;
    if total > height && height > 0 {
        let shown = (offset + 1).min(total);
        let last = (offset + height).min(total);
        header.push(Span::styled(
            format!("· {shown}-{last}/{total} "),
            theme::warn(),
        ));
    }
    f.render_widget(panel(Line::from(header), !editing), area);

    f.render_widget(
        Paragraph::new(lines)
            .style(theme::base())
            .scroll((offset as u16, 0)),
        inner,
    );
}

fn draw_context(f: &mut Frame, app: &AppState, area: Rect) {
    let block = panel("Field details", false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let focused = app.settings.field();
    let mut lines = Vec::new();
    lines.push(kv(
        "Field:",
        Span::styled(focused.label(), theme::strong()),
        10,
    ));
    lines.push(kv(
        "Hint:",
        Span::styled(focused.hint(), theme::accent()),
        10,
    ));

    if let Some(err) = app.settings.errors.get(&focused) {
        lines.push(kv("Error:", Span::styled(err.clone(), theme::err()), 10));
    } else {
        let note = match focused {
            Field::SessionId => "Loaded from INSTAGRAM_SESSIONID in .env or shell.",
            Field::Scope => {
                "Pick 'threads by ID' to prune specific threads or use Targets screen (2)."
            }
            Field::DryRun => "Safety first: always dry-run before actual deletion.",
            Field::Throttle => {
                "Instagram mobile API aggressively limits DELETE requests; 2000ms recommended."
            }
            Field::Protect => "Matches username, participant PK, or thread ID.",
            _ => "Press Enter to edit or cycle value.",
        };
        lines.push(kv("Note:", Span::styled(note, theme::dim()), 10));
    }

    f.render_widget(Paragraph::new(lines).style(theme::base()), inner);
}
