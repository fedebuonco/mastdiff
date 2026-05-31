use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::ast_diff::{row_has_children, AstLine};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let src_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)))
        .title(Span::styled(
            format!(" {} ", short_name(&app.left_path)),
            Style::default().fg(Color::Rgb(100, 180, 255)),
        ));
    let ast_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(
                " AST — {} [{}]  Shift+F:cycle filter ",
                short_name(&app.left_path),
                app.single_filter.label()
            ),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));

    let src_inner = src_block.inner(panes[0]);
    let ast_inner = ast_block.inner(panes[1]);
    let view_height = src_inner.height as usize;

    f.render_widget(src_block, panes[0]);
    f.render_widget(ast_block, panes[1]);

    // ── Source pane
    render_source(f, app, src_inner, view_height);

    // ── AST pane
    render_single_ast(f, app, ast_inner, view_height);
}

fn render_source(f: &mut Frame, app: &App, area: Rect, view_height: usize) {
    let scroll = app.scroll;
    let end = (scroll + view_height).min(app.left_lines.len());
    let width = area.width as usize;

    let lines: Vec<Line> = (scroll..end)
        .map(|row| {
            let is_cursor = row == app.cursor;
            let in_sel = in_selection(app, row);
            let lno = format!("{:4} ", row + 1);
            let text = trunc_str(app.left_lines.get(row).map(|s| s.as_str()).unwrap_or(""), width.saturating_sub(6));
            let style = if is_cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else if in_sel {
                Style::default().bg(Color::Rgb(30, 30, 70)).fg(Color::Rgb(160, 200, 255))
            } else {
                Style::default()
            };
            let lno_style = if is_cursor {
                style
            } else {
                Style::default().fg(Color::Rgb(70, 70, 90)).bg(style.bg.unwrap_or(Color::Reset))
            };
            Line::from(vec![
                Span::styled(lno, lno_style),
                Span::styled(text, style),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines), area);
}

fn render_single_ast(f: &mut Frame, app: &App, area: Rect, view_height: usize) {
    let scroll = app.single_scroll;
    let vis = &app.single_filter_rows;
    let end = (scroll + view_height).min(vis.len());
    let width = area.width as usize;

    let dummy: Vec<AstLine> = vec![];

    let lines: Vec<Line> = vis[scroll..end]
        .iter()
        .enumerate()
        .map(|(i, &raw)| {
            let cursor_disp = scroll + i;
            let is_cursor = cursor_disp == app.single_cursor;
            let node = app.single_ast.get(raw);
            let collapsed = app.single_collapsed.contains(&raw);
            let has_children = node
                .map(|_n| row_has_children(raw, &app.single_ast, &dummy))
                .unwrap_or(false);

            render_single_node(node, is_cursor, collapsed, has_children, width)
        })
        .collect();

    f.render_widget(Paragraph::new(lines), area);
}

fn render_single_node(
    node: Option<&AstLine>,
    is_cursor: bool,
    collapsed: bool,
    has_children: bool,
    width: usize,
) -> Line<'static> {
    let Some(node) = node else {
        return Line::raw("");
    };

    let color = kind_color(&node.kind);
    let fold_icon = if has_children {
        if collapsed { "▶ " } else { "▼ " }
    } else {
        "  "
    };
    let indent = "  ".repeat(node.depth);
    let text_part = match &node.leaf_text {
        Some(t) => format!(" \"{}\"", trunc_str(t, 24)),
        None => String::new(),
    };
    let row_indicator = format!(":{}", node.source_row + 1);
    let content = format!(
        "{}{}{}{}{} {}",
        indent, fold_icon, node.kind, text_part, row_indicator, ""
    );
    let content = trunc_str(&content, width);

    let style = if is_cursor {
        Style::default()
            .fg(color)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };

    Line::styled(content, style)
}

/// Colour AST nodes by their syntactic category.
fn kind_color(kind: &str) -> Color {
    if kind.contains("function") || kind.contains("method") {
        Color::Rgb(100, 180, 255) // blue — functions
    } else if kind.contains("class") || kind.contains("struct") || kind.contains("union") || kind.contains("enum") {
        Color::Rgb(220, 180, 80) // yellow — types
    } else if kind.contains("declaration") || kind.contains("parameter") {
        Color::Rgb(100, 220, 150) // green — declarations
    } else if kind.contains("comment") {
        Color::Rgb(100, 120, 100) // dark green — comments
    } else if kind.contains("string") || kind.contains("number") || kind.contains("literal") {
        Color::Rgb(200, 140, 100) // orange — literals
    } else if kind.contains("identifier") || kind.contains("name") {
        Color::Rgb(200, 200, 220) // light — identifiers
    } else {
        Color::Rgb(150, 150, 170) // default gray
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn in_selection(app: &App, row: usize) -> bool {
    match (app.selection_start, app.selection_end) {
        (Some(s), Some(e)) => row >= s && row <= e,
        (Some(s), None) => row == s,
        _ => false,
    }
}

fn short_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn trunc_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    }
}
