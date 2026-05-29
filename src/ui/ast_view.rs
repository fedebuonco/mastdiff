use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::ast_diff::{row_has_children, AstLine, NodeStatus};

pub fn render_diff(f: &mut Frame, app: &App, area: Rect) {
    let Some(ref result) = app.ast_result else {
        return;
    };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(outer[0]);

    let border_style = Style::default().fg(Color::Cyan);
    let title_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);

    let lb = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            format!(" AST ← {} ", short_name(&app.left_path)),
            title_style,
        ));
    let rb = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            format!(" AST → {} ", short_name(&app.right_path)),
            title_style,
        ));

    let il = lb.inner(panes[0]);
    let ir = rb.inner(panes[1]);
    let view_height = il.height as usize;

    f.render_widget(lb, panes[0]);
    f.render_widget(rb, panes[1]);

    let scroll = app.ast_scroll;
    let vis = &app.ast_filter_rows;
    let end = (scroll + view_height).min(vis.len());

    let mut left_lines: Vec<Line> = Vec::with_capacity(view_height);
    let mut right_lines: Vec<Line> = Vec::with_capacity(view_height);

    for (disp_idx, &raw) in vis[scroll..end].iter().enumerate() {
        let cursor_disp = scroll + disp_idx;
        let is_cursor = cursor_disp == app.ast_cursor;

        let left_node = result.left_nodes.get(raw);
        let right_node = result.right_nodes.get(raw);
        let collapsed = app.ast_collapsed.contains(&raw);
        let has_children = row_has_children(raw, &result.left_nodes, &result.right_nodes);

        left_lines.push(render_ast_node(
            left_node,
            is_cursor,
            collapsed,
            has_children,
            il.width as usize,
        ));
        right_lines.push(render_ast_node(
            right_node,
            is_cursor,
            collapsed,
            has_children,
            ir.width as usize,
        ));
    }

    while left_lines.len() < view_height {
        left_lines.push(Line::raw(""));
        right_lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(left_lines), il);
    f.render_widget(Paragraph::new(right_lines), ir);

    // Legend
    let legend = Line::from(vec![
        Span::styled("  same", Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("+ added", Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled("─ removed", Style::default().fg(Color::Red)),
        Span::raw("  "),
        Span::styled("~ modified", Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(
            format!("filter:{}", app.ast_filter.label()),
            Style::default().fg(Color::Cyan),
        ),
    ]);
    f.render_widget(Paragraph::new(legend), outer[1]);
}

fn render_ast_node(
    node: Option<&AstLine>,
    is_cursor: bool,
    collapsed: bool,
    has_children: bool,
    width: usize,
) -> Line<'static> {
    let Some(node) = node else {
        return Line::raw("");
    };
    if node.empty {
        return if is_cursor {
            Line::styled("~", Style::default().add_modifier(Modifier::REVERSED))
        } else {
            Line::raw("")
        };
    }

    let (color, marker) = node_style_marker(&node.status);
    let fold_icon = if has_children {
        if collapsed { "▶ " } else { "▼ " }
    } else {
        "  "
    };
    let indent = "  ".repeat(node.depth);
    let text_part = match &node.leaf_text {
        Some(t) => format!(" \"{}\"", trunc_str(t, 25)),
        None => String::new(),
    };
    let content = format!("{}{}{}{}{}", indent, fold_icon, marker, node.kind, text_part);
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

// ── Colour by status ──────────────────────────────────────────────────────

fn node_style_marker(status: &NodeStatus) -> (Color, &'static str) {
    match status {
        NodeStatus::Same => (Color::Rgb(160, 160, 180), "  "),
        NodeStatus::Added => (Color::Rgb(100, 220, 100), "+ "),
        NodeStatus::Removed => (Color::Rgb(220, 100, 100), "─ "),
        NodeStatus::Modified => (Color::Rgb(220, 200, 80), "~ "),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

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
