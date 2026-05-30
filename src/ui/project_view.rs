use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::{App, ProjectRow, ProjectView};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    render_header(f, app, chunks[0]);
    render_file_list(f, app, chunks[1]);
}

// ── Header bar ────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let n_shown = app.project_display.len();
    let n_total = app.project_files.len();

    let view_tabs = Line::from(vec![
        tab_span("1:TUs",    app.project_view == ProjectView::Tus),
        Span::raw(" "),
        tab_span("2:Sources", app.project_view == ProjectView::Sources),
        Span::raw(" "),
        tab_span("3:Headers", app.project_view == ProjectView::Headers),
        Span::raw(" "),
        tab_span("4:CMake",  app.project_view == ProjectView::Cmake),
        Span::styled(
            format!("   {}/{} items", n_shown, n_total),
            Style::default().fg(Color::Rgb(80, 80, 100)),
        ),
        if !app.cmake_targets.is_empty() {
            Span::styled(
                format!("   {} cmake targets", app.cmake_targets.len()),
                Style::default().fg(Color::Rgb(100, 160, 80)),
            )
        } else {
            Span::raw("")
        },
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.project_filter_active {
            Style::default().fg(Color::Rgb(100, 180, 255))
        } else {
            Style::default().fg(Color::Rgb(60, 60, 80))
        })
        .title(Span::styled(
            format!(" {} ", app.left_path),
            Style::default().fg(Color::Rgb(100, 180, 255)),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.project_filter_active {
        // Show text input for filter
        let filter = app.project_filter.as_str();
        let line = Line::from(vec![
            Span::styled("  filter: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(
                if filter.is_empty() { String::new() } else { filter.to_string() },
                Style::default().fg(Color::White),
            ),
            Span::styled("  Esc:clear  Enter:confirm", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line), inner);
        let cx = inner.x + 10 + app.project_filter.cursor_col() as u16;
        if cx < inner.x + inner.width {
            f.set_cursor_position((cx, inner.y));
        }
    } else if !app.project_filter.as_str().is_empty() {
        // Filter set but inactive — show filter + tabs
        let filter_span = Span::styled(
            format!("  filter:{:?}  ", app.project_filter.as_str()),
            Style::default().fg(Color::Rgb(140, 140, 160)),
        );
        let mut spans = vec![filter_span];
        spans.extend(view_tabs.spans);
        f.render_widget(Paragraph::new(Line::from(spans)), inner);
    } else {
        f.render_widget(Paragraph::new(view_tabs), inner);
    }
}

fn tab_span(label: &'static str, active: bool) -> Span<'static> {
    if active {
        Span::styled(
            format!("[{}]", label),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(80, 130, 220))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {} ", label),
            Style::default().fg(Color::Rgb(90, 90, 120)),
        )
    }
}

// ── File list ─────────────────────────────────────────────────────────────

fn render_file_list(f: &mut Frame, app: &App, area: Rect) {
    let view_label = match app.project_view {
        ProjectView::Tus     => "Translation Units  Space:expand  Tab:switch view",
        ProjectView::Sources => "Source Files  Tab:switch view",
        ProjectView::Headers => "Header Files  Tab:switch view",
        ProjectView::Cmake   => "CMake Targets  Space:expand  Tab:switch view",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)))
        .title(Span::styled(
            format!(" {}  [Enter:open  s:filter  g:grep  f:ast  Ctrl+o:editor] ", view_label),
            Style::default().fg(Color::DarkGray),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;
    let n = app.project_display.len();
    if n == 0 {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  (nothing to show)",
                Style::default().fg(Color::Rgb(70, 70, 90)),
            )),
            inner,
        );
        return;
    }

    let scroll = if app.project_cursor >= view_height {
        app.project_cursor - view_height + 1
    } else {
        0
    };
    let end = (scroll + view_height).min(n);
    let width = inner.width as usize;

    let lines: Vec<Line> = app.project_display[scroll..end]
        .iter()
        .enumerate()
        .map(|(i, row)| render_row(app, row, scroll + i == app.project_cursor, width))
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_row(app: &App, row: &ProjectRow, is_cursor: bool, width: usize) -> Line<'static> {
    match row {
        ProjectRow::Source(idx) => {
            let tu = &app.project_files[*idx];
            let expanded = app.project_expanded.contains(idx);
            let has_headers = !tu.associated_headers.is_empty();

            let expand_icon = match (has_headers, expanded) {
                (true, true)  => "▾ ",
                (true, false) => "▸ ",
                _             => "  ",
            };
            let arrow = if is_cursor { "▶" } else { " " };
            let prefix = format!("{}{}", arrow, expand_icon);
            file_line(&prefix, tu.short_name(), &tu.size_label(), is_cursor, width,
                      Color::Rgb(150, 200, 255))
        }

        ProjectRow::Header { path, size, .. } => {
            let name = short_name(path);
            let size_label = size_label(*size);
            let prefix = if is_cursor { "▶  ├ " } else { "   ├ " };
            file_line(prefix, name, &size_label, is_cursor, width,
                      Color::Rgb(100, 180, 130))
        }

        ProjectRow::LooseHeader(idx) => {
            let tu = &app.project_files[*idx];
            let arrow = if is_cursor { "▶ " } else { "  " };
            file_line(arrow, tu.short_name(), &tu.size_label(), is_cursor, width,
                      Color::Rgb(100, 200, 140))
        }

        ProjectRow::CmakeTarget(idx) => {
            let tgt = &app.cmake_targets[*idx];
            let expanded = app.project_expanded.contains(idx);
            let has_sources = !tgt.sources.is_empty();
            let kind_icon = if tgt.kind == "executable" { "⚙ " } else { "◫ " };
            let expand_icon = match (has_sources, expanded) {
                (true, true)  => "▾ ",
                (true, false) => "▸ ",
                _             => "  ",
            };
            let arrow = if is_cursor { "▶" } else { " " };
            let prefix = format!("{}{}{}", arrow, expand_icon, kind_icon);
            let src_count = if tgt.sources.is_empty() {
                String::new()
            } else {
                format!("{} src", tgt.sources.len())
            };
            file_line(&prefix, &tgt.name, &src_count, is_cursor, width,
                      Color::Rgb(220, 170, 80))
        }

        ProjectRow::CmakeSource { path, .. } => {
            let name = short_name(path);
            let prefix = if is_cursor { "▶  ├ " } else { "   ├ " };
            file_line(prefix, name, "", is_cursor, width,
                      Color::Rgb(180, 180, 200))
        }
    }
}

fn file_line(
    prefix: &str,
    name: &str,
    right: &str,
    is_cursor: bool,
    width: usize,
    fg: Color,
) -> Line<'static> {
    let prefix_len = prefix.chars().count();
    let right_len = right.chars().count();
    let gap = if right_len > 0 { 2 } else { 0 };
    let name_max = width.saturating_sub(prefix_len + right_len + gap);
    let name_trunc: String = name.chars().take(name_max).collect();
    let padding = " ".repeat(name_max.saturating_sub(name_trunc.chars().count()) + gap);

    let bg = if is_cursor { Color::Rgb(30, 40, 70) } else { Color::Reset };
    let name_fg = if is_cursor { Color::White } else { fg };
    let right_fg = if is_cursor { Color::Rgb(160, 160, 200) } else { Color::DarkGray };

    Line::from(vec![
        Span::styled(
            format!("{}{}{}", prefix, name_trunc, padding),
            Style::default().fg(name_fg).bg(bg)
                .add_modifier(if is_cursor { Modifier::BOLD } else { Modifier::empty() }),
        ),
        Span::styled(right.to_string(), Style::default().fg(right_fg).bg(bg)),
    ])
}

fn short_name(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

fn size_label(size: u64) -> String {
    if size < 1024 { format!("{}B", size) }
    else if size < 1024 * 1024 { format!("{:.1}KB", size as f64 / 1024.0) }
    else { format!("{:.1}MB", size as f64 / (1024.0 * 1024.0)) }
}
