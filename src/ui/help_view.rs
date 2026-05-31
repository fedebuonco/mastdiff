use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, _app: &App, area: Rect) {
    // Draw a centred modal over the whole terminal.
    let popup = centred(area, 90, 90);

    // Clear the background behind the popup so content underneath doesn't bleed through.
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(100, 140, 220)))
        .title(Line::from(vec![
            Span::styled(
                " mastdiff — keyboard reference ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(100, 140, 220))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Esc / q / ? to close ",
                Style::default().fg(Color::Rgb(120, 120, 150)),
            ),
        ]));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    // Split inner area into two columns.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    f.render_widget(Paragraph::new(left_column()), cols[0]);
    f.render_widget(Paragraph::new(right_column()), cols[1]);
}

// ── Column content ────────────────────────────────────────────────────────

fn left_column() -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    section(&mut lines, "SEARCH — AST operators  (press f from project view)");
    row(&mut lines, "fn:<name>",      "function definitions matching <name>");
    row(&mut lines, "call:<name>",    "call sites of <name> (excludes nested args)");
    row(&mut lines, "var:<name>",     "variable declarations");
    row(&mut lines, "class:<name>",   "class / struct / enum definitions");
    row(&mut lines, "field:<name>",   "struct/class field declarations");
    row(&mut lines, "param:<name>",   "function parameters");
    row(&mut lines, "type:<name>",    "type identifiers");
    row(&mut lines, "include:<hdr>",  "#include directives");
    row(&mut lines, "lambda:",        "lambda expressions  (filter by text)");
    row(&mut lines, "macro:<name>",   "#define / macro definitions");
    row(&mut lines, "ns:<name>",      "namespace definitions");
    row(&mut lines, "op:<sym>",       "operator overloads  (e.g. op:==)");
    row(&mut lines, "using:<name>",   "using declarations");
    row(&mut lines, "tpl:<name>",     "template declarations (fn or class)");
    row(&mut lines, "throw:",         "throw statements");
    row(&mut lines, "cast:<type>",    "cast expressions (C-style + static/dynamic/…)");
    blank(&mut lines);
    row(&mut lines, "(node_kind) @cap","raw tree-sitter S-expression query");
    row(&mut lines, "  e.g.",         "(call_expression function: (identifier) @fn)");
    blank(&mut lines);

    section(&mut lines, "SEARCH — GREP  (press g from project view)");
    row(&mut lines, "<text>",         "plain-text search across all files");
    row(&mut lines, "",               "default: case-insensitive  (Alt+C to toggle)");
    blank(&mut lines);

    section(&mut lines, "SEARCH — navigation & options");
    row(&mut lines, "Enter",          "run the search immediately");
    row(&mut lines, "Alt+R",          "toggle regex mode  [.*] badge lit = ON");
    row(&mut lines, "Alt+C",          "toggle case-sensitive  [Aa] badge lit = ON");
    row(&mut lines, "↑ / ↓",          "move through results");
    row(&mut lines, "PgUp / PgDn",    "jump 10 results");
    row(&mut lines, "Ctrl+o",         "open result in external editor");
    row(&mut lines, "Esc",            "return to previous view");
    row(&mut lines, "scroll (mouse)", "scroll source / AST / results independently");
    row(&mut lines, "click (mouse)",  "select a result row");
    row(&mut lines, "drag (mouse)",   "select text in source pane");
    row(&mut lines, "Ctrl+C",         "copy source selection to clipboard");
    row(&mut lines, "Ctrl+Y",         "copy highlighted AST node to clipboard");

    lines
}

fn right_column() -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    section(&mut lines, "PROJECT BROWSER");
    row(&mut lines, "j / k  ↑↓",     "navigate file list");
    row(&mut lines, "Enter",          "open file in AST browser");
    row(&mut lines, "s",              "filter file list by name");
    row(&mut lines, "g",              "grep search across all files");
    row(&mut lines, "f",              "AST / tree-sitter search");
    row(&mut lines, "Ctrl+o",         "open in external editor");
    row(&mut lines, "q",              "quit");
    blank(&mut lines);

    section(&mut lines, "DIFF VIEW  (two-file mode)");
    row(&mut lines, "s / e",          "select left / right pane");
    row(&mut lines, "Enter",          "open AST diff of selection");
    row(&mut lines, "n / N",          "next / previous hunk");
    row(&mut lines, "u",              "toggle unified view");
    row(&mut lines, "c",              "toggle context-only view");
    row(&mut lines, "w",              "toggle ignore-whitespace");
    row(&mut lines, "i",              "toggle inline diff");
    row(&mut lines, "x",              "export patch file");
    row(&mut lines, "X",              "export HTML diff");
    blank(&mut lines);

    section(&mut lines, "AST / SINGLE-FILE VIEW");
    row(&mut lines, "j / k  ↑↓",     "scroll source");
    row(&mut lines, "g",              "grep search in this file");
    row(&mut lines, "f",              "AST / tree-sitter search in this file");
    row(&mut lines, "s / e + Enter",  "select line range → zoom AST to slice");
    row(&mut lines, "Space",          "collapse / expand AST node");
    row(&mut lines, "Shift+F",        "cycle AST filter (all / functions / classes / vars)");
    row(&mut lines, "v",              "cycle AST viz (Tree ↔ Timeline)");
    row(&mut lines, "dbl-click",      "timeline: zoom into node  (dbl-click base to zoom out)");
    row(&mut lines, "Esc",            "clear selection / zoom  (or back to project)");
    row(&mut lines, "Ctrl+o",         "open current line in external editor");
    blank(&mut lines);

    section(&mut lines, "GLOBAL");
    row(&mut lines, "?",              "this help page  (press again to close)");
    row(&mut lines, "Ctrl+o",         "open file at cursor in configured editor");
    row(&mut lines, "q",              "quit the application");
    blank(&mut lines);

    section(&mut lines, "CONFIG  (~/.config/mastdiff/config.toml)");
    row(&mut lines, "open_in",        "\"vim\" or \"vscode\"");
    row(&mut lines, "log_level",      "\"off\" | \"error\" | \"warn\" | \"info\" | \"debug\" | \"trace\"");

    lines
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn section(lines: &mut Vec<Line<'static>>, title: &'static str) {
    lines.push(Line::from(vec![Span::styled(
        format!(" {} ", title),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(80, 120, 200))
            .add_modifier(Modifier::BOLD),
    )]));
}

fn blank(lines: &mut Vec<Line<'static>>) {
    lines.push(Line::raw(""));
}

fn row(lines: &mut Vec<Line<'static>>, key: &'static str, desc: &'static str) {
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {:20}", key),
            Style::default()
                .fg(Color::Rgb(220, 190, 80))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc, Style::default().fg(Color::Rgb(190, 190, 210))),
    ]));
}

/// Return a `Rect` centred in `area` with the given percentage of width/height.
fn centred(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = (area.width * pct_w / 100).min(area.width);
    let h = (area.height * pct_h / 100).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}
