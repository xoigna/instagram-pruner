use ratatui::style::{Color, Modifier, Style};

use crate::events::LogLevel;
use crate::history::DeleteOutcome;

pub const BG: Color = Color::Rgb(16, 18, 22);

pub const FG: Color = Color::Rgb(222, 225, 230);

pub const DIM: Color = Color::Rgb(118, 126, 140);

pub const ACCENT: Color = Color::Rgb(122, 186, 255);

pub const OK: Color = Color::Rgb(126, 214, 152);

pub const WARN: Color = Color::Rgb(238, 200, 112);

pub const ERR: Color = Color::Rgb(238, 124, 124);

pub const SEL: Color = Color::Rgb(44, 52, 66);

pub const LINE: Color = Color::Rgb(62, 70, 84);

pub fn base() -> Style {
    Style::default().fg(FG).bg(BG)
}

pub fn dim() -> Style {
    Style::default().fg(DIM)
}

pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

pub fn ok() -> Style {
    Style::default().fg(OK)
}

pub fn warn() -> Style {
    Style::default().fg(WARN)
}

pub fn err() -> Style {
    Style::default().fg(ERR)
}

pub fn strong() -> Style {
    Style::default().fg(FG).add_modifier(Modifier::BOLD)
}

pub fn focus() -> Style {
    Style::default().fg(ACCENT)
}

pub fn border() -> Style {
    Style::default().fg(LINE)
}

pub fn selected() -> Style {
    Style::default().bg(SEL).fg(FG).add_modifier(Modifier::BOLD)
}

pub fn key() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn level(level: LogLevel) -> Style {
    match level {
        LogLevel::Info => Style::default().fg(ACCENT),
        LogLevel::Success => Style::default().fg(OK),
        LogLevel::Warning => Style::default().fg(WARN),
        LogLevel::Error => Style::default().fg(ERR),
    }
}

pub fn outcome(outcome: DeleteOutcome) -> Style {
    match outcome {
        DeleteOutcome::Deleted => Style::default().fg(OK),
        DeleteOutcome::AlreadyGone => Style::default().fg(DIM),
        DeleteOutcome::Forbidden => Style::default().fg(WARN),
        DeleteOutcome::DryRun => Style::default().fg(ACCENT),
    }
}
