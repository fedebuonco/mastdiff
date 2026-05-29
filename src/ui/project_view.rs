use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    render_filter_bar(f, app, chunks[0]);
    render_file_list(f, app, chunks[1]);
}

fn render_filter_bar(f: &mut Frame, app: &App, area: Rect) {
    let filter = app.project_filter.as_str();
    let query_display = if filter.is_empty() {
        Span::styled("  Type to filter files…", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(format!("  {}", filter), Style::default().fg(Color::White))
    };

    let n_shown = app.project_filtered.len();
    let n_total = app.project_files.len();
    let count = Span::styled(
        format!("  {}/{} files", n_shown, n_total),
        Style::default().fg(Color::DarkGray),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)))
        .title(Span::styled(
            format!(" Project: {} ", app.left_path),
            Style::default().fg(Color::Rgb(100, 180, 255)),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let line = Line::from(vec![
        Span::styled("🔍 ", Style::default().fg(Color::Cyan)),
        query_display,
        count,
    ]);
    f.render_widget(Paragraph::new(line), inner);

    // Show terminal cursor in filter input
    let cursor_col = 3 + app.project_filter.cursor_col() as u16;
    if cursor_col < inner.width {
        f.set_cursor_position((inner.x + cursor_col, inner.y));
    }
}

fn render_file_list(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)))
        .title(Span::styled(
            " Translation Units  [Enter:open  s:search  j/k:move] ",
            Style::default().fg(Color::DarkGray),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;
    let n = app.project_filtered.len();

    // Compute scroll to keep cursor visible
    let scroll = if app.project_cursor >= view_height {
        app.project_cursor - view_height + 1
    } else {
        0
    };
    let end = (scroll + view_height).min(n);

    let width = inner.width as usize;

    let lines: Vec<Line> = app.project_filtered[scroll..end]
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let tu = &app.project_files[idx];
            let is_cursor = (scroll + i) == app.project_cursor;

            let arrow = if is_cursor { "▶ " } else { "  " };
            let name = tu.short_name();
            let size = tu.size_label();
            // Pad name so size aligns to right
            let name_max = width.saturating_sub(arrow.len() + size.len() + 2);
            let name_trunc: String = name.chars().take(name_max).collect();
            let padding = " ".repeat(name_max.saturating_sub(name_trunc.chars().count()) + 1);

            let style = if is_cursor {
                Style::default()
                    .bg(Color::Rgb(30, 40, 70))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(180, 180, 200))
            };
            let size_style = if is_cursor {
                style
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Line::from(vec![
                Span::styled(format!("{}{}{}", arrow, name_trunc, padding), style),
                Span::styled(size, size_style),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}
