use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::ast_diff::AstLine;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    // Vertical split: input bar on top, body below
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    render_search_bar(f, app, outer[0]);

    // Body: results list on left, source+AST on right
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(32), Constraint::Min(1)])
        .split(outer[1]);

    render_results_list(f, app, body[0]);
    render_right_pane(f, app, body[1]);
}

// ── Search bar ────────────────────────────────────────────────────────────

fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Search  [fn: call: var: class: type: include: param:]  Enter:run  ↑↓:results  o:open  Esc:back ",
            Style::default().fg(Color::DarkGray),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = app.search_input.as_str();
    let cursor_col = app.search_input.cursor_col();

    // Render with a block cursor at cursor_col
    let before = &text[..app.search_input.cursor_col().min(text.len())];
    let before_chars: String = text.chars().take(cursor_col).collect();
    let at_char: String = text.chars().nth(cursor_col).map(|c| c.to_string()).unwrap_or_else(|| " ".to_string());
    let after_chars: String = text.chars().skip(cursor_col + 1).collect();
    let _ = before; // suppress unused warning

    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(before_chars),
        Span::styled(at_char, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(after_chars),
    ]);
    f.render_widget(Paragraph::new(line), inner);

    // Also set terminal cursor for the OS IME / accessibility
    let cx = inner.x + 2 + cursor_col as u16;
    if cx < inner.x + inner.width {
        f.set_cursor_position((cx, inner.y));
    }
}

// ── Results list ──────────────────────────────────────────────────────────

fn render_results_list(f: &mut Frame, app: &App, area: Rect) {
    let n = app.search_results.len();
    let title = if n == 0 {
        " Results ".to_string()
    } else {
        format!(" {} results ", n)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 80, 100)))
        .title(Span::styled(title, Style::default().fg(Color::Rgb(150, 150, 200))));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;
    let scroll = {
        let s = app.search_scroll;
        // Ensure selected is visible
        if app.search_selected < s {
            app.search_selected
        } else if app.search_selected >= s + view_height {
            app.search_selected - view_height + 1
        } else {
            s
        }
    };
    let end = (scroll + view_height).min(n);
    let width = inner.width as usize;

    let mut lines: Vec<Line> = Vec::with_capacity(view_height);

    if app.search_results.is_empty() {
        lines.push(Line::styled(
            " No results yet. Press Enter to search.",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        for (i, result) in app.search_results[scroll..end].iter().enumerate() {
            let abs_i = scroll + i;
            let is_sel = abs_i == app.search_selected;

            let short = result.short_path();
            let lineno = format!(":{}", result.line + 1);
            let header = trunc(&format!("{}{}", short, lineno), width.saturating_sub(1));
            let snippet = trunc(&result.snippet, width.saturating_sub(2));

            let hstyle = if is_sel {
                Style::default()
                    .bg(Color::Rgb(30, 50, 90))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(140, 160, 200))
            };
            let sstyle = if is_sel {
                Style::default().bg(Color::Rgb(20, 35, 65)).fg(Color::Rgb(200, 220, 255))
            } else {
                Style::default().fg(Color::Rgb(100, 110, 130))
            };

            lines.push(Line::styled(format!(" {}", header), hstyle));
            lines.push(Line::styled(format!("  {}", snippet), sstyle));
        }
    }

    while lines.len() < view_height {
        lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

// ── Right pane: source + AST ──────────────────────────────────────────────

fn render_right_pane(f: &mut Frame, app: &App, area: Rect) {
    let halves = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_source_pane(f, app, halves[0]);
    render_ast_pane(f, app, halves[1]);
}

fn render_source_pane(f: &mut Frame, app: &App, area: Rect) {
    let file_name = app
        .search_results
        .get(app.search_selected)
        .map(|r| r.short_path())
        .unwrap_or("");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 80, 60)))
        .title(Span::styled(
            format!(" {} ", file_name),
            Style::default().fg(Color::Rgb(150, 220, 150)),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;
    let scroll = app.search_source_scroll;
    let total = app.search_source_lines.len();
    let end = (scroll + view_height).min(total);
    let width = inner.width as usize;

    let lines: Vec<Line> = (scroll..end)
        .map(|row| {
            let is_highlight = row == app.search_source_highlight;
            let lno = format!("{:4} ", row + 1);
            let text = trunc(
                app.search_source_lines.get(row).map(|s| s.as_str()).unwrap_or(""),
                width.saturating_sub(6),
            );

            if is_highlight {
                Line::from(vec![
                    Span::styled(lno, Style::default().fg(Color::Rgb(220, 200, 80)).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!("▶ {}", text),
                        Style::default()
                            .bg(Color::Rgb(50, 50, 0))
                            .fg(Color::Rgb(255, 240, 100))
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled(lno, Style::default().fg(Color::Rgb(70, 70, 90))),
                    Span::styled(format!("  {}", text), Style::default()),
                ])
            }
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_ast_pane(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(" AST ", Style::default().fg(Color::Cyan)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;
    let nodes = &app.search_ast_nodes;
    let scroll = app.search_ast_scroll;
    let end = (scroll + view_height).min(nodes.len());
    let width = inner.width as usize;

    let lines: Vec<Line> = nodes[scroll..end]
        .iter()
        .enumerate()
        .map(|(i, node)| render_ast_node(node, scroll + i == app.search_ast_highlight, width))
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_ast_node(node: &AstLine, is_highlight: bool, width: usize) -> Line<'static> {
    let indent = "  ".repeat(node.depth);
    let text_part = node
        .leaf_text
        .as_ref()
        .map(|t| format!(" \"{}\"", trunc(t, 20)))
        .unwrap_or_default();
    let row_tag = format!(":L{}", node.source_row + 1);
    let content = trunc(&format!("{}{}{}{}", indent, node.kind, text_part, row_tag), width);

    if is_highlight {
        Line::from(vec![Span::styled(
            format!("● {}", content),
            Style::default()
                .fg(Color::Rgb(255, 220, 50))
                .bg(Color::Rgb(50, 40, 0))
                .add_modifier(Modifier::BOLD),
        )])
    } else {
        let color = match node.source_row {
            _ if node.kind.contains("function") => Color::Rgb(100, 160, 255),
            _ if node.kind.contains("class") || node.kind.contains("struct") => Color::Rgb(220, 180, 80),
            _ if node.kind.contains("identifier") => Color::Rgb(180, 180, 200),
            _ => Color::Rgb(140, 140, 160),
        };
        Line::styled(format!("  {}", content), Style::default().fg(color))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn trunc(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    }
}
