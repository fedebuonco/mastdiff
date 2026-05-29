use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, PartialEq)]
pub enum DiffStatus {
    Equal,
    Added,
    Removed,
}

/// A byte-range within a line, marked as changed or not.
#[derive(Debug, Clone)]
pub struct InlineSpan {
    pub start: usize,
    pub end: usize,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub left_lineno: Option<usize>,
    pub right_lineno: Option<usize>,
    pub left_text: Option<String>,
    pub right_text: Option<String>,
    pub left_status: DiffStatus,
    pub right_status: DiffStatus,
    /// Byte-range spans for inline char diff on the left side
    pub left_spans: Vec<InlineSpan>,
    /// Byte-range spans for inline char diff on the right side
    pub right_spans: Vec<InlineSpan>,
}

pub fn compute_diff(left: &str, right: &str, ignore_ws: bool) -> Vec<DiffLine> {
    let (lp, rp) = if ignore_ws {
        (normalize_ws(left), normalize_ws(right))
    } else {
        (left.to_string(), right.to_string())
    };

    let diff = TextDiff::from_lines(&lp, &rp);
    let mut result: Vec<DiffLine> = Vec::new();
    let mut left_lineno = 0usize;
    let mut right_lineno = 0usize;
    let mut left_buf: Vec<(usize, String)> = Vec::new();
    let mut right_buf: Vec<(usize, String)> = Vec::new();

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let text = change.value().trim_end_matches('\n').to_string();
            match change.tag() {
                ChangeTag::Equal => {
                    flush_bufs(&mut result, &mut left_buf, &mut right_buf);
                    result.push(DiffLine {
                        left_lineno: Some(left_lineno),
                        right_lineno: Some(right_lineno),
                        left_text: Some(text.clone()),
                        right_text: Some(text),
                        left_status: DiffStatus::Equal,
                        right_status: DiffStatus::Equal,
                        left_spans: vec![],
                        right_spans: vec![],
                    });
                    left_lineno += 1;
                    right_lineno += 1;
                }
                ChangeTag::Delete => {
                    left_buf.push((left_lineno, text));
                    left_lineno += 1;
                }
                ChangeTag::Insert => {
                    right_buf.push((right_lineno, text));
                    right_lineno += 1;
                }
            }
        }
    }
    flush_bufs(&mut result, &mut left_buf, &mut right_buf);
    result
}

fn flush_bufs(
    result: &mut Vec<DiffLine>,
    left_buf: &mut Vec<(usize, String)>,
    right_buf: &mut Vec<(usize, String)>,
) {
    let max = left_buf.len().max(right_buf.len());
    for i in 0..max {
        let (ll, lt, ls) = if i < left_buf.len() {
            (Some(left_buf[i].0), Some(left_buf[i].1.clone()), DiffStatus::Removed)
        } else {
            (None, None, DiffStatus::Equal)
        };
        let (rl, rt, rs) = if i < right_buf.len() {
            (Some(right_buf[i].0), Some(right_buf[i].1.clone()), DiffStatus::Added)
        } else {
            (None, None, DiffStatus::Equal)
        };

        let (left_spans, right_spans) = match (&lt, &rt) {
            (Some(l), Some(r)) => inline_spans(l, r),
            _ => (vec![], vec![]),
        };

        result.push(DiffLine {
            left_lineno: ll,
            right_lineno: rl,
            left_text: lt,
            right_text: rt,
            left_status: ls,
            right_status: rs,
            left_spans,
            right_spans,
        });
    }
    left_buf.clear();
    right_buf.clear();
}

/// Compute character-level diff spans for a pair of changed lines.
pub fn inline_spans(left: &str, right: &str) -> (Vec<InlineSpan>, Vec<InlineSpan>) {
    let diff = TextDiff::from_chars(left, right);
    let mut ls: Vec<InlineSpan> = Vec::new();
    let mut rs: Vec<InlineSpan> = Vec::new();
    let mut lp = 0usize;
    let mut rp = 0usize;

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let len = change.value().len(); // byte length of this char
            match change.tag() {
                ChangeTag::Equal => {
                    ls.push(InlineSpan { start: lp, end: lp + len, changed: false });
                    rs.push(InlineSpan { start: rp, end: rp + len, changed: false });
                    lp += len;
                    rp += len;
                }
                ChangeTag::Delete => {
                    ls.push(InlineSpan { start: lp, end: lp + len, changed: true });
                    lp += len;
                }
                ChangeTag::Insert => {
                    rs.push(InlineSpan { start: rp, end: rp + len, changed: true });
                    rp += len;
                }
            }
        }
    }
    (ls, rs)
}

/// Row indices (into diff_lines) where a new diff hunk begins.
pub fn hunk_positions(diff_lines: &[DiffLine]) -> Vec<usize> {
    let mut hunks = Vec::new();
    let mut in_hunk = false;
    for (i, dl) in diff_lines.iter().enumerate() {
        let changed =
            dl.left_status != DiffStatus::Equal || dl.right_status != DiffStatus::Equal;
        if changed && !in_hunk {
            hunks.push(i);
            in_hunk = true;
        } else if !changed {
            in_hunk = false;
        }
    }
    hunks
}

/// Sorted indices of rows visible in context-only mode (changed ± `ctx` lines).
pub fn context_view(diff_lines: &[DiffLine], ctx: usize) -> Vec<usize> {
    let n = diff_lines.len();
    let mut vis = vec![false; n];
    for (i, dl) in diff_lines.iter().enumerate() {
        if dl.left_status != DiffStatus::Equal || dl.right_status != DiffStatus::Equal {
            let s = i.saturating_sub(ctx);
            let e = (i + ctx + 1).min(n);
            for j in s..e {
                vis[j] = true;
            }
        }
    }
    vis.iter()
        .enumerate()
        .filter(|(_, v)| **v)
        .map(|(i, _)| i)
        .collect()
}

fn normalize_ws(s: &str) -> String {
    s.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}
