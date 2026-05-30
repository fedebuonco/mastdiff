use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::text_diff::{DiffLine, DiffStatus, InlineSpan};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    if app.unified_view {
        render_unified(f, app, area);
    } else {
        render_side_by_side(f, app, area);
    }
}

// ── Side-by-side ──────────────────────────────────────────────────────────

fn render_side_by_side(f: &mut Frame, app: &App, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let title_style = Style::default().fg(Color::Rgb(100, 130, 200));
    let border_style = Style::default().fg(Color::Rgb(60, 60, 80));

    let lb = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            format!(" {} ", short_name(&app.left_path)),
            title_style,
        ));
    let rb = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            format!(" {} ", short_name(&app.right_path)),
            title_style,
        ));

    let il = lb.inner(panes[0]);
    let ir = rb.inner(panes[1]);
    let view_height = il.height as usize;

    f.render_widget(lb, panes[0]);
    f.render_widget(rb, panes[1]);

    let scroll = app.scroll;
    let total = if app.context_only {
        app.context_indices.len()
    } else {
        app.diff_lines.len()
    };
    let end = (scroll + view_height).min(total);

    let mut left_lines: Vec<Line> = Vec::with_capacity(view_height);
    let mut right_lines: Vec<Line> = Vec::with_capacity(view_height);

    for disp_row in scroll..end {
        let diff_row = app.display_to_diff(disp_row);
        let dl = &app.diff_lines[diff_row];

        let is_cursor = disp_row == app.cursor;
        let in_sel = in_selection(app, disp_row);

        let lno_l = lineno_str(dl.left_lineno);
        let lno_r = lineno_str(dl.right_lineno);

        let lp = diff_prefix(&dl.left_status);
        let rp = diff_prefix(&dl.right_status);

        let base_l = row_style(&dl.left_status, is_cursor, in_sel);
        let base_r = row_style(&dl.right_status, is_cursor, in_sel);

        let lno_style = dim_lineno(base_l);
        let lno_r_style = dim_lineno(base_r);

        let prefix_w = lno_l.len() + lp.len();
        let avail_l = (il.width as usize).saturating_sub(prefix_w + 1);
        let avail_r = (ir.width as usize).saturating_sub(prefix_w + 1);

        let left_line = if app.inline_diff && !dl.left_spans.is_empty() {
            let lt = dl.left_text.as_deref().unwrap_or("");
            build_inline_line(
                &lno_l, lp, lt, &dl.left_spans,
                base_l, lno_style,
                changed_highlight(&dl.left_status),
                is_cursor, avail_l,
            )
        } else {
            let text = trunc(dl.left_text.as_deref().unwrap_or(""), avail_l);
            let content = format!("{}{}{}", lno_l, lp, text);
            if is_cursor {
                Line::from(vec![Span::styled(content, base_l)])
            } else {
                Line::from(vec![
                    Span::styled(lno_l.clone(), lno_style),
                    Span::styled(format!("{}{}", lp, text), base_l),
                ])
            }
        };

        let right_line = if app.inline_diff && !dl.right_spans.is_empty() {
            let rt = dl.right_text.as_deref().unwrap_or("");
            build_inline_line(
                &lno_r, rp, rt, &dl.right_spans,
                base_r, lno_r_style,
                changed_highlight(&dl.right_status),
                is_cursor, avail_r,
            )
        } else {
            let text = trunc(dl.right_text.as_deref().unwrap_or(""), avail_r);
            let content = format!("{}{}{}", lno_r, rp, text);
            if is_cursor {
                Line::from(vec![Span::styled(content, base_r)])
            } else {
                Line::from(vec![
                    Span::styled(lno_r.clone(), lno_r_style),
                    Span::styled(format!("{}{}", rp, text), base_r),
                ])
            }
        };

        left_lines.push(left_line);
        right_lines.push(right_line);
    }

    while left_lines.len() < view_height {
        left_lines.push(Line::raw(""));
        right_lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(left_lines), il);
    f.render_widget(Paragraph::new(right_lines), ir);
}

#[allow(clippy::too_many_arguments)]
fn build_inline_line(
    lno: &str,
    prefix: &str,
    text: &str,
    spans: &[InlineSpan],
    base: Style,
    lno_style: Style,
    highlight: Style,
    is_cursor: bool,
    avail: usize,
) -> Line<'static> {
    let mut parts: Vec<Span<'static>> = Vec::new();
    parts.push(Span::styled(lno.to_string(), if is_cursor { base } else { lno_style }));
    parts.push(Span::styled(prefix.to_string(), base));

    let mut used = 0usize;
    for sp in spans {
        if used >= avail {
            break;
        }
        let chunk = text.get(sp.start..sp.end).unwrap_or("").to_string();
        let remaining = avail - used;
        let chunk = trunc(&chunk, remaining);
        used += chunk.len();
        let style = if is_cursor {
            base
        } else if sp.changed {
            highlight
        } else {
            base
        };
        parts.push(Span::styled(chunk, style));
    }

    Line::from(parts)
}

// ── Unified view ──────────────────────────────────────────────────────────

fn render_unified(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 80)))
        .title(Span::styled(
            format!(" {} → {} ", short_name(&app.left_path), short_name(&app.right_path)),
            Style::default().fg(Color::Rgb(100, 130, 200)),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = build_unified_lines(&app.diff_lines, app.context_lines);
    let total = lines.len();
    let view_height = inner.height as usize;
    let scroll = app.scroll.min(total.saturating_sub(1));
    let end = (scroll + view_height).min(total);

    let visible: Vec<Line> = lines[scroll..end]
        .iter()
        .map(|(text, style)| Line::styled(text.clone(), *style))
        .collect();

    f.render_widget(Paragraph::new(visible), inner);
}

fn build_unified_lines(diff_lines: &[DiffLine], ctx: usize) -> Vec<(String, Style)> {
    let n = diff_lines.len();
    let mut in_range = vec![false; n];
    for (i, dl) in diff_lines.iter().enumerate() {
        if dl.left_status != DiffStatus::Equal || dl.right_status != DiffStatus::Equal {
            let s = i.saturating_sub(ctx);
            let e = (i + ctx + 1).min(n);
            for item in in_range.iter_mut().take(e).skip(s) {
                *item = true;
            }
        }
    }

    let mut out: Vec<(String, Style)> = Vec::new();
    let mut i = 0;

    while i < n {
        if !in_range[i] {
            i += 1;
            continue;
        }

        let hunk_start = i;
        while i < n && in_range[i] {
            i += 1;
        }
        let hunk_end = i;

        let ls = diff_lines[hunk_start].left_lineno.unwrap_or(0) + 1;
        let rs = diff_lines[hunk_start].right_lineno.unwrap_or(0) + 1;
        let lc = diff_lines[hunk_start..hunk_end]
            .iter()
            .filter(|d| d.left_lineno.is_some())
            .count();
        let rc = diff_lines[hunk_start..hunk_end]
            .iter()
            .filter(|d| d.right_lineno.is_some())
            .count();

        out.push((
            format!("@@ -{},{} +{},{} @@", ls, lc, rs, rc),
            Style::default().fg(Color::Cyan),
        ));

        for dl in &diff_lines[hunk_start..hunk_end] {
            match (&dl.left_status, &dl.right_status) {
                (DiffStatus::Equal, DiffStatus::Equal) => {
                    let t = dl.left_text.as_deref().unwrap_or("");
                    out.push((format!(" {}", t), Style::default().fg(Color::Gray)));
                }
                (DiffStatus::Removed, _) => {
                    let t = dl.left_text.as_deref().unwrap_or("");
                    out.push((
                        format!("-{}", t),
                        Style::default().bg(Color::Rgb(50, 0, 0)).fg(Color::Rgb(220, 100, 100)),
                    ));
                    if dl.right_status == DiffStatus::Added {
                        let t = dl.right_text.as_deref().unwrap_or("");
                        out.push((
                            format!("+{}", t),
                            Style::default().bg(Color::Rgb(0, 45, 0)).fg(Color::Rgb(100, 220, 100)),
                        ));
                    }
                }
                (_, DiffStatus::Added) => {
                    let t = dl.right_text.as_deref().unwrap_or("");
                    out.push((
                        format!("+{}", t),
                        Style::default().bg(Color::Rgb(0, 45, 0)).fg(Color::Rgb(100, 220, 100)),
                    ));
                }
                _ => {}
            }
        }
    }

    out
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn in_selection(app: &App, row: usize) -> bool {
    match (app.selection_start, app.selection_end) {
        (Some(s), Some(e)) => row >= s && row <= e,
        (Some(s), None) => row == s,
        _ => false,
    }
}

fn row_style(status: &DiffStatus, is_cursor: bool, in_sel: bool) -> Style {
    let base = match status {
        DiffStatus::Equal => Style::default(),
        DiffStatus::Added => Style::default()
            .bg(Color::Rgb(0, 40, 0))
            .fg(Color::Rgb(120, 220, 120)),
        DiffStatus::Removed => Style::default()
            .bg(Color::Rgb(45, 0, 0))
            .fg(Color::Rgb(220, 100, 100)),
    };
    if is_cursor {
        base.add_modifier(Modifier::REVERSED)
    } else if in_sel {
        match status {
            DiffStatus::Equal => Style::default().bg(Color::Rgb(30, 30, 70)),
            _ => base.bg(Color::Rgb(60, 40, 80)),
        }
    } else {
        base
    }
}

fn changed_highlight(status: &DiffStatus) -> Style {
    match status {
        DiffStatus::Removed => Style::default()
            .bg(Color::Rgb(100, 0, 0))
            .fg(Color::Rgb(255, 160, 160))
            .add_modifier(Modifier::BOLD),
        DiffStatus::Added => Style::default()
            .bg(Color::Rgb(0, 80, 0))
            .fg(Color::Rgb(160, 255, 160))
            .add_modifier(Modifier::BOLD),
        DiffStatus::Equal => Style::default(),
    }
}

fn dim_lineno(base: Style) -> Style {
    // Dim the line-number portion
    Style::default()
        .fg(Color::Rgb(70, 70, 90))
        .bg(base.bg.unwrap_or(Color::Reset))
}

fn diff_prefix(status: &DiffStatus) -> &'static str {
    match status {
        DiffStatus::Removed => "─ ",
        DiffStatus::Added => "+ ",
        DiffStatus::Equal => "  ",
    }
}

fn lineno_str(n: Option<usize>) -> String {
    match n {
        Some(v) => format!("{:4} ", v + 1),
        None => "     ".to_string(),
    }
}

fn short_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn trunc(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    }
}
