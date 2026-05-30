use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Line,
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, AppMode};

mod ast_view;
mod diff_view;
mod help_view;
mod project_view;
mod search_view;
mod single_view;

pub fn render(f: &mut Frame, app: &mut App, _terminal_height: usize) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let view_height = chunks[0].height.saturating_sub(2) as usize;

    match app.mode {
        AppMode::TextDiff => {
            app.clamp_scroll(view_height);
            diff_view::render(f, app, chunks[0]);
        }
        AppMode::AstDiff => {
            app.clamp_ast_scroll(view_height);
            ast_view::render_diff(f, app, chunks[0]);
        }
        AppMode::SingleFile => {
            app.clamp_scroll(view_height);
            app.clamp_single_scroll(view_height);
            single_view::render(f, app, chunks[0]);
        }
        AppMode::ProjectBrowser => {
            project_view::render(f, app, chunks[0]);
        }
        AppMode::Search => {
            search_view::render(f, app, chunks[0]);
        }
        AppMode::Help => {
            // Render the previous view behind the help overlay, then draw help on top.
            match app.help_prev_mode {
                AppMode::TextDiff => {
                    app.clamp_scroll(view_height);
                    diff_view::render(f, app, chunks[0]);
                }
                AppMode::AstDiff => {
                    app.clamp_ast_scroll(view_height);
                    ast_view::render_diff(f, app, chunks[0]);
                }
                AppMode::SingleFile => {
                    app.clamp_scroll(view_height);
                    app.clamp_single_scroll(view_height);
                    single_view::render(f, app, chunks[0]);
                }
                AppMode::ProjectBrowser => {
                    project_view::render(f, app, chunks[0]);
                }
                AppMode::Search => {
                    search_view::render(f, app, chunks[0]);
                }
                AppMode::Help => {}
            }
            help_view::render(f, app, chunks[0]);
        }
    }

    let status = Paragraph::new(Line::raw(app.status_msg.as_str()))
        .style(Style::default().bg(Color::Rgb(25, 25, 45)).fg(Color::White));
    f.render_widget(status, chunks[1]);
}
