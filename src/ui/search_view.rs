use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::{App, AstVizMode, SearchFocus, ROWS_PER_RESULT};
use crate::syntax::SyntaxSpan;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    // Vertical stack: query bar (3) + filter bar (6 = include+exclude) + body (rest)
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Min(1),
        ])
        .split(area);

    render_search_bar(f, app, outer[0]);
    render_filter_bar(f, app, outer[1]);

    // Body: results list on left, source+AST on right
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(36), Constraint::Min(1)])
        .split(outer[2]);

    // Store areas for mouse hit-testing (updated every frame)
    app.search_results_area = body[0];

    render_results_list(f, app, body[0]);
    render_right_pane(f, app, body[1]);
}

// ── Search bar ────────────────────────────────────────────────────────────

/// Known shorthand prefixes and the accent color to use for each.
/// Order here matches the hint chips in the search bar title.
const SHORTHANDS: &[(&str, Color)] = &[
    ("fn:",      Color::Rgb(78,  201, 176)),  // teal
    ("call:",    Color::Rgb(220, 160,  80)),  // orange
    ("var:",     Color::Rgb(197, 134, 192)),  // purple
    ("class:",   Color::Rgb(86,  156, 214)),  // blue
    ("type:",    Color::Rgb(100, 180, 240)),  // light-blue
    ("include:", Color::Rgb(150, 200, 100)),  // green
    ("param:",   Color::Rgb(220, 220, 100)),  // yellow
    ("field:",   Color::Rgb(200, 140, 200)),  // lavender
    ("lambda:",  Color::Rgb(255, 130, 160)),  // rose/pink
    ("macro:",   Color::Rgb(210,  80,  80)),  // red
    ("ns:",      Color::Rgb(80,  210, 220)),  // cyan
    ("op:",      Color::Rgb(240, 180,  40)),  // amber
    ("using:",   Color::Rgb(100, 210, 160)),  // mint
    ("tpl:",     Color::Rgb(200, 100, 220)),  // magenta
    ("throw:",   Color::Rgb(220, 100,  60)),  // red-orange
    ("cast:",    Color::Rgb(180, 160, 120)),  // warm-tan
];

fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.search_focus == SearchFocus::Query;
    let base_color = if app.search_grep_mode { Color::Yellow } else { Color::Cyan };
    let border_color = if focused { base_color } else { Color::Rgb(60, 80, 100) };

    // Mode label chip
    let mode_label = if app.search_grep_mode { "GREP" } else { "AST " };
    let mode_span = Span::styled(
        format!(" {} ", mode_label),
        Style::default().fg(Color::Black).bg(base_color).add_modifier(Modifier::BOLD),
    );

    // Regex toggle badge
    let regex_badge = if app.search_use_regex {
        Span::styled(" [.*] ", Style::default().fg(Color::Black).bg(Color::Rgb(200, 120, 60)).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" [.*] ", Style::default().fg(Color::Rgb(70, 70, 90)))
    };

    // Case-sensitive toggle badge
    let case_badge = if app.search_case_sensitive {
        Span::styled(" [Aa] ", Style::default().fg(Color::Black).bg(Color::Rgb(80, 160, 220)).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" [Aa] ", Style::default().fg(Color::Rgb(70, 70, 90)))
    };

    // Build title spans
    let mut title_spans: Vec<Span> = vec![mode_span, regex_badge, case_badge];

    if app.search_grep_mode {
        title_spans.push(Span::styled(
            "  type text  Tab:filters  Enter:run  Alt+R:regex  Alt+C:case  Esc:back",
            Style::default().fg(Color::Rgb(80, 80, 100)),
        ));
    } else {
        // Color-coded prefix chips, one per shorthand
        title_spans.push(Span::styled(" ", Style::default()));
        for &(prefix, color) in SHORTHANDS {
            title_spans.push(Span::styled(
                prefix,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
            title_spans.push(Span::raw(" "));
        }
        title_spans.push(Span::styled(
            " (ts-query)  Tab:filters  Alt+R  Alt+C",
            Style::default().fg(Color::Rgb(70, 70, 95)),
        ));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Line::from(title_spans));
    let inner = block.inner(area);
    f.render_widget(block, area);

    render_query_input(f, app.search_input.as_str(), app.search_input.cursor_col(),
                       focused, app.search_grep_mode, inner);
}

// ── Filter bar (include / exclude) ───────────────────────────────────────

fn render_filter_bar(f: &mut Frame, app: &App, area: Rect) {
    // Stack include on top, exclude below
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(area);

    render_filter_box(
        f,
        "✚ files to include",
        "e.g. src/**,*.cpp",
        app.search_include.as_str(),
        app.search_include.cursor_col(),
        app.search_focus == SearchFocus::Include,
        Color::Rgb(80, 180, 80),
        rows[0],
    );
    render_filter_box(
        f,
        "⊘ files to exclude",
        "e.g. tests/**,vendor/**",
        app.search_exclude.as_str(),
        app.search_exclude.cursor_col(),
        app.search_focus == SearchFocus::Exclude,
        Color::Rgb(200, 80, 80),
        rows[1],
    );
}

#[allow(clippy::too_many_arguments)]
fn render_filter_box(
    f: &mut Frame,
    title: &str,
    placeholder: &str,
    text: &str,
    cursor_col: usize,
    focused: bool,
    accent: Color,
    area: Rect,
) {
    let border_color = if focused { accent } else { Color::Rgb(50, 55, 70) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(if focused { accent } else { Color::Rgb(100, 100, 120) })
                .add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() }),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if text.is_empty() && !focused {
        // Show placeholder
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", placeholder),
                Style::default().fg(Color::Rgb(60, 65, 80)),
            )),
            inner,
        );
    } else {
        render_text_input(f, text, cursor_col, focused, accent, inner);
    }
}

/// Syntax-aware query input: colours the recognised prefix (e.g. `fn:`) differently
/// from the filter text that follows.  Falls back to plain rendering for grep mode
/// or raw tree-sitter queries.
fn render_query_input(
    f: &mut Frame,
    text: &str,
    cursor_col: usize,
    focused: bool,
    grep_mode: bool,
    area: Rect,
) {
    // Resolve the prefix length and per-segment colours.
    let (prefix_len, kw_color, ft_color): (usize, Color, Color) = if grep_mode {
        (0, Color::White, Color::White)
    } else if text.starts_with('(') || text.starts_with('[') {
        // Raw TS query — whole thing in orange
        (0, Color::Rgb(220, 160, 80), Color::Rgb(220, 160, 80))
    } else {
        let mut found = (0usize, Color::White, Color::White);
        for &(prefix, color) in SHORTHANDS {
            if text.starts_with(prefix) {
                found = (prefix.len(), color, Color::White);
                break;
            }
        }
        found
    };

    let chars: Vec<char> = text.chars().collect();
    let n     = chars.len();
    let plen  = prefix_len.min(n);
    // Clamp cursor into [0, n] so it is always a valid split point.
    let cur   = cursor_col.min(n);

    // Bright block cursor — high contrast regardless of surrounding colours.
    let cur_style = Style::default().fg(Color::Black).bg(Color::White);
    let kw_style  = Style::default().fg(kw_color).add_modifier(Modifier::BOLD);
    let ft_style  = Style::default().fg(ft_color);

    if focused {
        let mut spans: Vec<Span> = vec![Span::raw(" ")];

        if cur < plen {
            // ── Cursor is inside the keyword prefix ──────────────────────
            // [kw 0..cur] [cursor] [kw cur+1..plen] [ft plen..n]
            if cur > 0 {
                spans.push(Span::styled(chars[..cur].iter().collect::<String>(), kw_style));
            }
            spans.push(Span::styled(chars[cur..cur+1].iter().collect::<String>(), cur_style));
            if cur + 1 < plen {
                spans.push(Span::styled(chars[cur+1..plen].iter().collect::<String>(), kw_style));
            }
            if plen < n {
                spans.push(Span::styled(chars[plen..].iter().collect::<String>(), ft_style));
            }
        } else {
            // ── Cursor is in the filter portion (or at end) ───────────────
            // [kw 0..plen] [ft plen..cur] [cursor] [ft cur+1..n]
            if plen > 0 {
                spans.push(Span::styled(chars[..plen].iter().collect::<String>(), kw_style));
            }
            if cur > plen {
                spans.push(Span::styled(chars[plen..cur].iter().collect::<String>(), ft_style));
            }
            // Character at cursor (or a space if we're past the end)
            let at: String = if cur < n {
                chars[cur..cur+1].iter().collect()
            } else {
                " ".to_string()
            };
            spans.push(Span::styled(at, cur_style));
            if cur + 1 < n {
                spans.push(Span::styled(chars[cur+1..].iter().collect::<String>(), ft_style));
            }
        }

        f.render_widget(Paragraph::new(Line::from(spans)), area);
        // Also set the real terminal cursor (enables blinking in supporting terminals).
        let cx = area.x + 1 + cur as u16;
        if cx < area.x + area.width {
            f.set_cursor_position((cx, area.y));
        }
    } else {
        // Not focused — dim the prefix colour, grey out the filter.
        let prefix: String = chars[..plen].iter().collect();
        let filter: String = chars[plen..].iter().collect();
        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled(prefix, Style::default().fg(kw_color).add_modifier(Modifier::DIM)),
            Span::styled(filter, Style::default().fg(Color::Rgb(130, 130, 150))),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }
}

/// Render a text input line with an optional blinking-style cursor.
fn render_text_input(f: &mut Frame, text: &str, cursor_col: usize, focused: bool, _accent: Color, area: Rect) {
    if focused {
        let chars: Vec<char> = text.chars().collect();
        let n   = chars.len();
        let cur = cursor_col.min(n);

        let before: String = chars[..cur].iter().collect();
        let at: String     = if cur < n { chars[cur..cur+1].iter().collect() } else { " ".to_string() };
        let after: String  = if cur + 1 < n { chars[cur+1..].iter().collect() } else { String::new() };

        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled(before, Style::default().fg(Color::White)),
            Span::styled(at,    Style::default().fg(Color::Black).bg(Color::White)),
            Span::styled(after, Style::default().fg(Color::White)),
        ]);
        f.render_widget(Paragraph::new(line), area);

        // Terminal cursor position (enables blinking in supporting terminals).
        let cx = area.x + 1 + cur as u16;
        if cx < area.x + area.width {
            f.set_cursor_position((cx, area.y));
        }
    } else {
        // Dimmed, no cursor
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", text),
                Style::default().fg(Color::Rgb(150, 150, 170)),
            )),
            area,
        );
    }
}

// ── Results list ──────────────────────────────────────────────────────────

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn render_results_list(f: &mut Frame, app: &App, area: Rect) {
    let n = app.search_results.len();
    let mode_tag = if app.search_query.grep_mode {
        "grep"
    } else if !app.search_query.ts_query_src.is_empty() {
        "ts"
    } else {
        ""
    };

    let (title, title_color) = if app.search_running {
        let frame = SPINNER[(app.spinner_tick / 2) as usize % SPINNER.len()];
        let t = if n == 0 {
            format!(" ⚙ {} Searching… [{}] ", frame, mode_tag)
        } else {
            format!(" ⚙ {} {} [{}] ", frame, n, mode_tag)
        };
        (t, Color::Rgb(220, 170, 60))
    } else if n == 0 {
        (format!(" Results [{}] ", mode_tag), Color::Rgb(170, 170, 220))
    } else {
        (format!(" {}/{} [{}] ", app.search_selected + 1, n, mode_tag), Color::Rgb(170, 170, 220))
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.search_running {
            Color::Rgb(160, 120, 40)
        } else {
            Color::Rgb(80, 80, 110)
        }))
        .title(Span::styled(title, Style::default().fg(title_color).add_modifier(Modifier::BOLD)));
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
        let msg = if app.search_running {
            " Searching…"
        } else if !app.search_source_lines.is_empty() {
            // File is open but no search has been run yet — browsing mode.
            " g:grep  f:ast-search  — type a query to search this file"
        } else {
            " Type a query to search…"
        };
        lines.push(Line::styled(
            msg,
            Style::default().fg(Color::Rgb(100, 100, 130)),
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
    // Prefer the selected result's path; fall back to the directly-opened file name.
    let file_name = app
        .search_results
        .get(app.search_selected)
        .map(|r| r.short_path())
        .unwrap_or_else(|| short_name(&app.search_open_file));

    // Show "Drag to select • Ctrl+C copy" hint in title when a selection exists
    let sel_hint = if app.search_sel.is_some() { "  [Ctrl+C:copy]" } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 80, 60)))
        .title(Span::styled(
            format!(" {}{} ", file_name, sel_hint),
            Style::default().fg(Color::Rgb(150, 220, 150)),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;
    let total = app.search_source_lines.len();
    let scroll = app.search_source_scroll.min(total);
    let end = (scroll + view_height).min(total);
    let width = inner.width as usize;

    // Normalize selection to (start_line, start_col, end_line, end_col) or None
    let sel = app.search_sel;

    let lines: Vec<Line> = (scroll..end)
        .map(|row| {
            let is_highlight = row == app.search_source_highlight;
            let lno = format!("{:4} ", row + 1);
            let raw_text = app.search_source_lines.get(row).map(|s| s.as_str()).unwrap_or("");
            let max_text_width = width.saturating_sub(7); // 5 for lno, 2 for prefix "▶ "/"  "

            if is_highlight {
                // Highlighted row: gutter + arrow + highlighted plain text with optional sel overlay
                let text = trunc(raw_text, max_text_width);
                let mut spans = vec![
                    Span::styled(lno, Style::default().fg(Color::Rgb(220, 200, 80)).add_modifier(Modifier::BOLD)),
                ];
                if let Some(((sl, sc), (el, ec))) = sel {
                    if row >= sl && row <= el {
                        let chars: Vec<char> = format!("▶ {}", text).chars().collect();
                        // Prefix "▶ " is 2 chars; selection col offset starts there
                        let prefix_len = 2usize;
                        let sel_from = if row == sl { sc + prefix_len } else { prefix_len };
                        let sel_to   = if row == el { (ec + prefix_len).min(chars.len()) } else { chars.len() };
                        spans.extend(split_selection_spans(
                            &chars.iter().collect::<String>(),
                            sel_from, sel_to,
                            Style::default().bg(Color::Rgb(50, 50, 0)).fg(Color::Rgb(255, 240, 100)).add_modifier(Modifier::BOLD),
                            Style::default().bg(Color::Rgb(80, 70, 20)).fg(Color::Rgb(255, 255, 180)).add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(Span::styled(
                            format!("▶ {}", text),
                            Style::default().bg(Color::Rgb(50, 50, 0)).fg(Color::Rgb(255, 240, 100)).add_modifier(Modifier::BOLD),
                        ));
                    }
                } else {
                    spans.push(Span::styled(
                        format!("▶ {}", text),
                        Style::default().bg(Color::Rgb(50, 50, 0)).fg(Color::Rgb(255, 240, 100)).add_modifier(Modifier::BOLD),
                    ));
                }
                Line::from(spans)
            } else {
                // Normal row: gutter + syntax-coloured spans (with optional selection overlay)
                let empty_tokens: Vec<SyntaxSpan> = vec![];
                let tokens = app.search_source_tokens.get(row).unwrap_or(&empty_tokens);
                let gutter = Span::styled(lno, Style::default().fg(Color::Rgb(70, 70, 90)));
                let prefix  = Span::raw("  ");
                let mut spans = vec![gutter, prefix];

                if let Some(((sl, sc), (el, ec))) = sel {
                    if row >= sl && row <= el {
                        let raw_trunc = trunc(raw_text, max_text_width);
                        let sel_from = if row == sl { sc } else { 0 };
                        let sel_to   = if row == el { ec } else { raw_trunc.chars().count() };
                        // Build syntax spans then overlay selection highlight
                        let base = syntax_spans(raw_text, tokens, max_text_width);
                        spans.extend(overlay_selection(base, sel_from, sel_to));
                    } else {
                        spans.extend(syntax_spans(raw_text, tokens, max_text_width));
                    }
                } else {
                    spans.extend(syntax_spans(raw_text, tokens, max_text_width));
                }
                Line::from(spans)
            }
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

/// Split a string into up to three spans: before selection (base_style), selection
/// (sel_style), after selection (base_style).
fn split_selection_spans(
    text: &str,
    sel_from: usize,
    sel_to: usize,
    base_style: Style,
    sel_style: Style,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let sf = sel_from.min(n);
    let st = sel_to.min(n);
    let mut out = Vec::new();
    if sf > 0 {
        out.push(Span::styled(chars[..sf].iter().collect::<String>(), base_style));
    }
    if sf < st {
        out.push(Span::styled(chars[sf..st].iter().collect::<String>(), sel_style));
    }
    if st < n {
        out.push(Span::styled(chars[st..].iter().collect::<String>(), base_style));
    }
    out
}

/// Walk existing syntax spans and apply selection highlight in the [sel_from, sel_to) char range.
/// The spans already have their text; we split and re-style as needed.
fn overlay_selection(spans: Vec<Span<'static>>, sel_from: usize, sel_to: usize) -> Vec<Span<'static>> {
    let sel_bg = Color::Rgb(60, 80, 140);
    let sel_fg = Color::White;
    let mut result: Vec<Span<'static>> = Vec::new();
    let mut pos = 0usize; // char position in the rendered text

    for span in spans {
        let span_chars: Vec<char> = span.content.chars().collect();
        let span_len = span_chars.len();
        let span_end = pos + span_len;

        let base_style = span.style;
        let in_sel_from = sel_from.max(pos);
        let in_sel_to   = sel_to.min(span_end);

        if in_sel_from >= in_sel_to {
            // No overlap — keep as-is
            result.push(span);
        } else {
            // before selection
            let before = in_sel_from - pos;
            if before > 0 {
                result.push(Span::styled(
                    span_chars[..before].iter().collect::<String>(),
                    base_style,
                ));
            }
            // selected part
            let sel_start = in_sel_from - pos;
            let sel_end   = in_sel_to - pos;
            result.push(Span::styled(
                span_chars[sel_start..sel_end].iter().collect::<String>(),
                base_style.bg(sel_bg).fg(sel_fg),
            ));
            // after selection
            if sel_end < span_len {
                result.push(Span::styled(
                    span_chars[sel_end..].iter().collect::<String>(),
                    base_style,
                ));
            }
        }
        pos = span_end;
    }
    result
}

fn render_ast_pane(f: &mut Frame, app: &mut App, area: Rect) {
    let browsing = app.search_results.is_empty() && !app.search_source_lines.is_empty();
    let nav_hint = if browsing && app.search_ast_viz == AstVizMode::Tree {
        "  ↑↓:nav  Space:fold"
    } else {
        ""
    };
    let viz_label = app.search_ast_viz.label();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Line::from(vec![
            Span::styled(
                format!(" AST [{}]{} ", viz_label, nav_hint),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" Alt+v:cycle-viz ", Style::default().fg(Color::Rgb(80, 80, 110))),
        ]));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let view_height = inner.height as usize;

    // Clamp-to-cursor only makes sense for the tree view.
    if app.search_ast_viz == AstVizMode::Tree {
        app.clamp_search_ast_scroll(view_height);
    }
    let scroll = app.search_ast_scroll;

    match app.search_ast_viz {
        AstVizMode::Tree     => render_ast_tree(f, app, inner, scroll, view_height),
        AstVizMode::Timeline => render_ast_timeline(f, app, inner, scroll),
    }
}

fn render_ast_tree(f: &mut Frame, app: &App, area: Rect, scroll: usize, view_height: usize) {
    let visible = &app.search_ast_visible;
    let nodes = &app.search_ast_nodes;

    log::debug!(
        "render_ast_tree: nodes={} visible={} cursor={} scroll={}",
        nodes.len(), visible.len(), app.search_ast_cursor, scroll
    );

    if visible.is_empty() || nodes.is_empty() {
        log::debug!("render_ast_tree: early-return (empty nodes or visible)");
        return;
    }

    // Safety: all raw indices in visible must point into nodes.
    let max_raw = visible.iter().copied().max().unwrap_or(0);
    if max_raw >= nodes.len() {
        log::warn!(
            "render_ast_tree: stale visible index {} >= nodes.len() {}; skipping",
            max_raw, nodes.len()
        );
        return;
    }

    let end = (scroll + view_height).min(visible.len());
    let width = area.width as usize;

    let lines: Vec<Line> = visible[scroll..end]
        .iter()
        .enumerate()
        .map(|(i, &raw)| {
            let vis_pos = scroll + i;
            let is_cursor   = vis_pos == app.search_ast_cursor;
            let is_match    = raw == app.search_ast_highlight;
            let collapsed   = app.search_ast_collapsed.contains(&raw);
            let dummy: Vec<crate::ast_diff::AstLine> = vec![];
            let has_children = crate::ast_diff::row_has_children(raw, nodes, &dummy);
            render_ast_tree_node(nodes, visible, vis_pos, raw, is_cursor, is_match,
                                 collapsed, has_children, width)
        })
        .collect();

    f.render_widget(Paragraph::new(lines), area);
}

// ── Timeline (Gantt swimlane) ────────────────────────────────────────────────

fn render_ast_timeline(f: &mut Frame, app: &App, area: Rect, depth_scroll: usize) {
    let nodes = &app.search_ast_nodes;
    let non_empty: Vec<(usize, &crate::ast_diff::AstLine)> = nodes.iter()
        .enumerate()
        .filter(|(_, n)| !n.empty && !n.kind.is_empty())
        .collect();
    if non_empty.is_empty() { return; }

    let width  = area.width  as usize;
    let height = area.height as usize;

    let cursor_raw = app.search_ast_visible.get(app.search_ast_cursor).copied().unwrap_or(usize::MAX);
    let match_raw  = app.search_ast_highlight;

    // ── Zoom base ─────────────────────────────────────────────────────────────
    // When a base node is set (via double-click), we restrict the view to its
    // subtree and rescale the X axis to its source span.
    let (min_row, max_row, base_depth, base_kind) =
        if let Some(br) = app.search_timeline_base.filter(|&br| br < nodes.len()) {
            let n = &nodes[br];
            (n.source_row, n.source_end_row, n.depth, n.kind.as_str())
        } else {
            let mn = non_empty.iter().map(|(_, n)| n.source_row).min().unwrap_or(0);
            let mx = non_empty.iter().map(|(_, n)| n.source_end_row).max().unwrap_or(0);
            (mn, mx, 0, "")
        };
    let total_span = (max_row - min_row + 1).max(1);

    // When zoomed, only show nodes that fall within the base's span.
    let visible_nodes: Vec<(usize, &crate::ast_diff::AstLine)> =
        if app.search_timeline_base.is_some() {
            non_empty.iter()
                .filter(|(_, n)| {
                    n.depth >= base_depth
                        && n.source_row >= min_row
                        && n.source_end_row <= max_row
                })
                .map(|&(i, n)| (i, n))
                .collect()
        } else {
            non_empty.clone()
        };

    let max_depth = visible_nodes.iter().map(|(_, n)| n.depth).max().unwrap_or(0);

    // Label column: "d99 " = 4 chars.
    let label_w = 4usize;
    let chart_w = width.saturating_sub(label_w);

    let scale = |row: usize| -> usize {
        ((row.saturating_sub(min_row)) * chart_w / total_span).min(chart_w)
    };

    let mut lines: Vec<Line> = Vec::with_capacity(height);

    // ── Header row ────────────────────────────────────────────────────────────
    {
        let mut ruler = vec![b' '; chart_w];
        let step = (total_span / (chart_w / 8).max(1)).max(1);
        let mut src = min_row;
        while src <= max_row {
            let col = scale(src);
            let label = format!("{}", src + 1);
            for (i, ch) in label.bytes().enumerate() {
                if col + i < chart_w { ruler[col + i] = ch; }
            }
            src += step;
        }
        let ruler_str = String::from_utf8_lossy(&ruler).into_owned();

        // If zoomed, show the base node kind as a breadcrumb instead of "src".
        let hdr_label = if !base_kind.is_empty() {
            let bc = format!(" ↳ {} ", base_kind);
            let bc_trimmed: String = bc.chars().take(label_w + 8).collect();
            Span::styled(bc_trimmed, Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(80, 130, 200))
                .add_modifier(Modifier::BOLD))
        } else {
            Span::styled(
                format!("{:>width$}", "src", width = label_w),
                Style::default().fg(Color::Rgb(80, 80, 110)),
            )
        };
        lines.push(Line::from(vec![
            hdr_label,
            Span::styled(ruler_str, Style::default().fg(Color::Rgb(70, 70, 100))),
        ]));
    }

    // ── One swimlane per depth level ──────────────────────────────────────────
    // When zoomed, depth labels are shown relative to the base (d0 = base depth).
    let depth_start = base_depth + depth_scroll;
    let depth_end   = (depth_start + height.saturating_sub(2)).min(max_depth);

    for depth in depth_start..=depth_end {
        if lines.len() >= height { break; }

        let mut cells:  Vec<u8>    = vec![b' '; chart_w];
        let mut colors: Vec<Color> = vec![Color::Reset; chart_w];
        let mut bolds:  Vec<bool>  = vec![false; chart_w];

        let mut depth_nodes: Vec<(usize, &crate::ast_diff::AstLine)> = visible_nodes
            .iter()
            .filter(|(_, n)| n.depth == depth)
            .map(|&(i, n)| (i, n))
            .collect();
        depth_nodes.sort_by_key(|(_, n)| n.source_row);

        for (raw_idx, node) in &depth_nodes {
            let x0 = scale(node.source_row);
            let x1 = scale(node.source_end_row + 1).max(x0 + 1).min(chart_w);

            let is_cursor = *raw_idx == cursor_raw;
            let is_match  = *raw_idx == match_raw;
            let is_base   = app.search_timeline_base == Some(*raw_idx);
            let col = if is_base                   { Color::Rgb(80, 200, 120) }  // green = current zoom root
                      else if is_cursor && is_match { Color::Rgb(255, 220, 50) }
                      else if is_cursor             { Color::Rgb(120, 180, 255) }
                      else if is_match              { Color::Rgb(220, 160, 40) }
                      else                          { ast_kind_color(&node.kind) };

            if x0 < chart_w { cells[x0] = b'['; colors[x0] = col; bolds[x0] = is_base || is_cursor; }
            let label_bytes: Vec<u8> = node.kind.bytes().collect();
            for x in (x0 + 1)..x1.saturating_sub(1).min(chart_w) {
                let li = x - x0 - 1;
                cells[x]  = if li < label_bytes.len() { label_bytes[li] } else { b'-' };
                colors[x] = col;
                bolds[x]  = is_base || is_cursor;
            }
            let xr = x1.saturating_sub(1);
            if xr > x0 && xr < chart_w {
                cells[xr] = b']'; colors[xr] = col; bolds[xr] = is_base || is_cursor;
            }
        }

        // Depth label: relative to base when zoomed.
        let rel_depth = depth - base_depth;
        let depth_lbl = Span::styled(
            format!("d{:<width$}", rel_depth, width = label_w.saturating_sub(1)),
            Style::default().fg(if rel_depth == 0 && app.search_timeline_base.is_some() {
                Color::Rgb(80, 200, 120) // base row label in green
            } else {
                Color::Rgb(90, 90, 130)
            }),
        );

        let mut spans: Vec<Span> = vec![depth_lbl];
        let mut i = 0;
        while i < chart_w {
            let col   = colors[i];
            let bold  = bolds[i];
            let start = i;
            while i < chart_w && colors[i] == col && bolds[i] == bold { i += 1; }
            let text: String = cells[start..i].iter().map(|&b| b as char).collect();
            if col == Color::Reset {
                spans.push(Span::raw(text));
            } else {
                let mut sty = Style::default().fg(col);
                if bold { sty = sty.add_modifier(Modifier::BOLD); }
                spans.push(Span::styled(text, sty));
            }
        }
        lines.push(Line::from(spans));
    }

    while lines.len() < height {
        lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(lines), area);
}

/// Darken a color for use as a block background in the icicle view.
/// Compute the tree-line prefix for the node at `vis_pos` in the visible list.
/// Returns something like "│  ├─ " or "   └─ ".
fn tree_prefix(nodes: &[crate::ast_diff::AstLine], visible: &[usize], vis_pos: usize) -> String {
    let depth = nodes[visible[vis_pos]].depth;
    if depth == 0 {
        return String::new();
    }

    // For each depth level d (0 … depth-1), determine whether the ancestor at
    // that level still has more siblings coming (i.e. we need a │ continuation).
    let mut prefix = String::new();
    for d in 0..depth {
        // Scan forward from vis_pos to see if there's any node at depth d before
        // depth < d (which would indicate the ancestor at d still has a sibling).
        let has_continuation = visible[vis_pos + 1..]
            .iter()
            .find_map(|&r| {
                match nodes[r].depth.cmp(&d) {
                    std::cmp::Ordering::Equal   => Some(true),
                    std::cmp::Ordering::Less    => Some(false),
                    std::cmp::Ordering::Greater => None,
                }
            })
            .unwrap_or(false);

        if d < depth - 1 {
            prefix.push_str(if has_continuation { "│  " } else { "   " });
        } else {
            prefix.push_str(if has_continuation { "├─ " } else { "└─ " });
        }
    }
    prefix
}

#[allow(clippy::too_many_arguments)]
fn render_ast_tree_node(
    nodes: &[crate::ast_diff::AstLine],
    visible: &[usize],
    vis_pos: usize,
    raw: usize,
    is_cursor: bool,
    is_match: bool,
    collapsed: bool,
    has_children: bool,
    width: usize,
) -> Line<'static> {
    if raw >= nodes.len() {
        log::error!(
            "render_ast_tree_node: raw={} out of bounds (nodes.len={}), vis_pos={}, visible.len={}",
            raw, nodes.len(), vis_pos, visible.len()
        );
        return Line::raw(format!("  <invalid node {}>", raw));
    }
    let node = &nodes[raw];

    let tree_pfx = tree_prefix(nodes, visible, vis_pos);

    // Fold icon (only for nodes with children).
    let fold_icon = if has_children {
        if collapsed { "▶ " } else { "▼ " }
    } else {
        "  "
    };

    let text_part = node
        .leaf_text
        .as_ref()
        .map(|t| format!(" \"{}\"", trunc(t, 18)))
        .unwrap_or_default();
    let row_tag = format!(" :L{}", node.source_row + 1);

    let kind_color = ast_kind_color(&node.kind);

    if is_cursor && is_match {
        // Cursor + result match: bright yellow background
        let content = trunc(
            &format!("{}{}{}{}{}", tree_pfx, fold_icon, node.kind, text_part, row_tag),
            width,
        );
        Line::from(vec![Span::styled(
            content,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(255, 220, 50))
                .add_modifier(Modifier::BOLD),
        )])
    } else if is_cursor {
        // Cursor (no match): blue-ish highlight
        let pfx_len = tree_pfx.chars().count();
        let pfx_span = Span::styled(
            tree_pfx,
            Style::default().fg(Color::Rgb(80, 100, 140)).bg(Color::Rgb(30, 45, 80)),
        );
        let fold_span = Span::styled(
            fold_icon,
            Style::default().fg(Color::Rgb(150, 200, 255)).bg(Color::Rgb(30, 45, 80)),
        );
        let rest = trunc(&format!("{}{}{}", node.kind, text_part, row_tag), width.saturating_sub(pfx_len + 2));
        let rest_span = Span::styled(
            rest,
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(30, 45, 80))
                .add_modifier(Modifier::BOLD),
        );
        Line::from(vec![pfx_span, fold_span, rest_span])
    } else if is_match {
        // Result match (not cursor): amber glow
        let content = trunc(
            &format!("{}{}{}{}{}", tree_pfx, fold_icon, node.kind, text_part, row_tag),
            width,
        );
        Line::from(vec![Span::styled(
            content,
            Style::default()
                .fg(Color::Rgb(255, 220, 50))
                .bg(Color::Rgb(50, 40, 0))
                .add_modifier(Modifier::BOLD),
        )])
    } else {
        // Normal node: dim tree lines, colored kind.
        let pfx_span   = Span::styled(tree_pfx,  Style::default().fg(Color::Rgb(60, 70, 90)));
        let fold_span  = Span::styled(fold_icon, Style::default().fg(Color::Rgb(120, 140, 170)));
        let kind_width = width.saturating_sub(
            nodes[raw].depth * 3 + 2 // approximate prefix+icon width
        );
        let kind_str   = trunc(&node.kind, kind_width);
        let kind_span  = Span::styled(kind_str,  Style::default().fg(kind_color));
        let meta_str   = trunc(&format!("{}{}", text_part, row_tag),
                               width.saturating_sub(kind_width));
        let meta_span  = Span::styled(meta_str, Style::default().fg(Color::Rgb(90, 100, 120)));
        Line::from(vec![pfx_span, fold_span, kind_span, meta_span])
    }
}

fn ast_kind_color(kind: &str) -> Color {
    if kind.contains("function") || kind.contains("method") {
        Color::Rgb(100, 180, 255)  // blue
    } else if kind.contains("class") || kind.contains("struct") || kind.contains("enum") {
        Color::Rgb(220, 180, 80)   // yellow
    } else if kind.contains("declaration") || kind.contains("parameter") {
        Color::Rgb(100, 220, 150)  // green
    } else if kind.contains("comment") {
        Color::Rgb(90, 110, 90)    // muted green
    } else if kind.contains("string") || kind.contains("number") || kind.contains("literal") {
        Color::Rgb(200, 140, 100)  // orange
    } else if kind.contains("identifier") || kind.contains("name") || kind.contains("type") {
        Color::Rgb(180, 180, 210)  // light purple
    } else if kind.contains("operator") || kind.contains("punctuation") {
        Color::Rgb(130, 130, 150)  // gray
    } else {
        Color::Rgb(150, 155, 175)  // default
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn short_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Convert a line's `SyntaxSpan` list into ratatui `Span`s, truncating to `max` chars.
/// Gaps between spans are filled with the default terminal colour.
fn syntax_spans<'a>(text: &str, tokens: &[SyntaxSpan], max: usize) -> Vec<Span<'a>> {
    if tokens.is_empty() {
        // No highlight info — plain truncated text.
        return vec![Span::raw(trunc(text, max))];
    }

    let chars: Vec<char> = text.chars().take(max).collect();
    let visible_len = chars.len();
    if visible_len == 0 {
        return vec![];
    }

    // Sort spans by start position (they should already be ordered).
    let mut sorted: Vec<&SyntaxSpan> = tokens
        .iter()
        .filter(|s| s.start < visible_len)
        .collect();
    sorted.sort_by_key(|s| s.start);

    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut cursor = 0usize;

    let char_to_str = |range: std::ops::Range<usize>| -> String {
        chars[range].iter().collect()
    };

    for tok in &sorted {
        let start = tok.start.min(visible_len);
        let end   = tok.end.min(visible_len);
        if start >= end { continue; }

        // Gap before this token — default colour
        if cursor < start {
            spans.push(Span::raw(char_to_str(cursor..start)));
        }
        spans.push(Span::styled(
            char_to_str(start..end),
            Style::default().fg(tok.color),
        ));
        cursor = end;
    }

    // Trailing default-colour text
    if cursor < visible_len {
        spans.push(Span::raw(char_to_str(cursor..visible_len)));
    }

    // If the line was truncated, append ellipsis on the last span
    if text.chars().count() > max && max > 0 {
        if let Some(last) = spans.last_mut() {
            let s = last.content.to_mut();
            if !s.is_empty() {
                s.pop();
                s.push('…');
            } else {
                *last = Span::raw("…".to_string());
            }
        } else {
            spans.push(Span::raw("…".to_string()));
        }
    }

    spans
}

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
