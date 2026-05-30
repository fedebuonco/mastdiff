use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::{App, ROWS_PER_RESULT};
use crate::ast_diff::AstLine;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    // Vertical split: input bar on top, body below
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    render_search_bar(f, app, outer[0]);

    // Body: results list on left, source+AST on right
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(36), Constraint::Min(1)])
        .split(outer[1]);

    // Store areas for mouse hit-testing (updated every frame)
    app.search_results_area = body[0];

    render_results_list(f, app, body[0]);
    render_right_pane(f, app, body[1]);
}

// ── Search bar ────────────────────────────────────────────────────────────

fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let (mode_label, hint) = if app.search_grep_mode {
        ("GREP", " type text to search across all files  Enter:run  Esc:back")
    } else {
        ("AST ", " fn: call: var: class: type: include: param: field:  or  (ts-query) @cap  Enter:run  Esc:back")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.search_grep_mode { Color::Yellow } else { Color::Cyan }))
        .title(Line::from(vec![
            Span::styled(
                format!(" {} ", mode_label),
                Style::default()
                    .fg(Color::Black)
                    .bg(if app.search_grep_mode { Color::Yellow } else { Color::Cyan })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = app.search_input.as_str();
    let cursor_col = app.search_input.cursor_col();

    let before_chars: String = text.chars().take(cursor_col).collect();
    let at_char: String = text.chars().nth(cursor_col).map(|c| c.to_string()).unwrap_or_else(|| " ".to_string());
    let after_chars: String = text.chars().skip(cursor_col + 1).collect();

    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(before_chars),
        Span::styled(at_char, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(after_chars),
    ]);
    f.render_widget(Paragraph::new(line), inner);

    let cx = inner.x + 2 + cursor_col as u16;
    if cx < inner.x + inner.width {
        f.set_cursor_position((cx, inner.y));
    }
}

// ── Results list ──────────────────────────────────────────────────────────

fn render_results_list(f: &mut Frame, app: &App, area: Rect) {
    let n = app.search_results.len();
    let mode_tag = if app.search_query.grep_mode {
        "grep"
    } else if !app.search_query.ts_query_src.is_empty() {
        "ts"
    } else {
        ""
    };
    let title = if n == 0 {
        format!(" Results [{}] ", mode_tag)
    } else {
        format!(" {}/{} [{}] ", app.search_selected + 1, n, mode_tag)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 80, 110)))
        .title(Span::styled(title, Style::default().fg(Color::Rgb(170, 170, 220)).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;
    // scroll is a result index (not a line index); clamp so selected is always visible
    let scroll = {
        let s = app.search_scroll;
        let visible_results = view_height / ROWS_PER_RESULT;
        if app.search_selected < s {
            app.search_selected
        } else if visible_results > 0 && app.search_selected >= s + visible_results {
            app.search_selected - visible_results + 1
        } else {
            s
        }
    };

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(view_height);

    if app.search_results.is_empty() {
        lines.push(Line::styled(
            " No results yet — press Enter to search.",
            Style::default().fg(Color::Rgb(80, 80, 100)),
        ));
    } else {
        // How many results fit?
        let visible_results = (view_height / ROWS_PER_RESULT).max(1);
        let end = (scroll + visible_results).min(n);

        for (slot, result) in app.search_results[scroll..end].iter().enumerate() {
            let abs_i = scroll + slot;
            let is_sel = abs_i == app.search_selected;
            // Zebra-stripe for non-selected rows (even/odd slot)
            let is_even = slot % 2 == 0;

            let short = result.short_path();
            let lineno = format!(":{}", result.line + 1);
            let cap = if result.capture_name.is_empty() {
                String::new()
            } else {
                format!(" @{}", result.capture_name)
            };

            if is_sel {
                // ── Selected result ───────────────────────────────────────
                let header_text = trunc(
                    &format!(" ▶ {}{}{}", short, lineno, cap),
                    width,
                );
                let snippet_text = trunc(
                    &format!("   {}", result.snippet),
                    width,
                );
                lines.push(Line::styled(
                    header_text,
                    Style::default()
                        .bg(Color::Rgb(40, 65, 120))
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::styled(
                    snippet_text,
                    Style::default()
                        .bg(Color::Rgb(28, 45, 88))
                        .fg(Color::Rgb(185, 215, 255)),
                ));
            } else {
                // ── Non-selected result (zebra-striped) ───────────────────
                let row_bg = if is_even {
                    Color::Rgb(18, 19, 30)
                } else {
                    Color::Rgb(24, 25, 40)
                };
                let header_text = trunc(
                    &format!("   {}{}{}", short, lineno, cap),
                    width,
                );
                let snippet_text = trunc(
                    &format!("   {}", result.snippet),
                    width,
                );
                lines.push(Line::styled(
                    header_text,
                    Style::default()
                        .bg(row_bg)
                        .fg(Color::Rgb(130, 155, 200)),
                ));
                lines.push(Line::styled(
                    snippet_text,
                    Style::default()
                        .bg(row_bg)
                        .fg(Color::Rgb(75, 80, 110)),
                ));
            }
        }
    }

    // Pad remaining rows
    while lines.len() < view_height {
        lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

// ── Right pane: source + AST ──────────────────────────────────────────────

fn render_right_pane(f: &mut Frame, app: &mut App, area: Rect) {
    let halves = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Store areas for mouse hit-testing
    app.search_source_area = halves[0];
    app.search_ast_area    = halves[1];

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
    let total = app.search_source_lines.len();
    let scroll = app.search_source_scroll.min(total);
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
    let scroll = app.search_ast_scroll.min(nodes.len());
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
        let color = match node.kind.as_str() {
            k if k.contains("function") => Color::Rgb(100, 160, 255),
            k if k.contains("class") || k.contains("struct") => Color::Rgb(220, 180, 80),
            k if k.contains("identifier") => Color::Rgb(180, 180, 200),
            _ => Color::Rgb(140, 140, 160),
        };
        Line::styled(format!("  {}", content), Style::default().fg(color))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn trunc(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        // Pad to max width so the background colour fills the whole row
        let mut out = s.to_string();
        while out.chars().count() < max {
            out.push(' ');
        }
        out
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    }
}
