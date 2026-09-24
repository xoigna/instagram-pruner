pub mod archive;
pub mod overlays;
pub mod run;
pub mod settings;
pub mod targets;

use ratatui::layout::Constraint;
use ratatui::Frame;

use super::state::AppState;
use super::widgets::{draw_footer, draw_header, split_v};

pub fn render(f: &mut Frame, app: &AppState) {
    let area = f.area();
    let parts = split_v(
        area,
        &[
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ],
    );
    f.render_widget(
        ratatui::widgets::Block::new().style(super::theme::base()),
        area,
    );
    draw_header(f, app, parts[0]);
    match app.screen {
        super::action::Screen::Settings => settings::draw(f, app, parts[1]),
        super::action::Screen::Targets => targets::draw(f, app, parts[1]),
        super::action::Screen::Run => run::draw(f, app, parts[1]),
        super::action::Screen::Archive => archive::draw(f, app, parts[1]),
    }
    draw_footer(f, app, parts[2]);
    overlays::draw(f, app, area);
}
