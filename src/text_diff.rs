//! Line-level and character-level text diff using the `similar` crate.
//!
//! [`compute_diff`] produces a [`DiffLine`] per logical row, pairing left and
//! right sides and computing inline [`InlineSpan`] byte ranges for changed
//! characters within modified lines.
//!
//! [`hunk_positions`] and [`context_view`] are helpers for the TUI: the former
//! returns the start row of each changed block, the latter computes which rows
//! are within ± N lines of any change for the context-only display mode.

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
            for item in vis.iter_mut().take(e).skip(s) {
                *item = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── compute_diff ──────────────────────────────────────────────────────

    #[test]
    fn identical_files_all_equal() {
        let src = "line one\nline two\nline three\n";
        let lines = compute_diff(src, src, false);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.left_status == DiffStatus::Equal));
        assert!(lines.iter().all(|l| l.right_status == DiffStatus::Equal));
    }

    #[test]
    fn added_line_at_end() {
        let left  = "a\nb\n";
        let right = "a\nb\nc\n";
        let lines = compute_diff(left, right, false);
        let added: Vec<_> = lines.iter().filter(|l| l.right_status == DiffStatus::Added).collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].right_text.as_deref(), Some("c"));
    }

    #[test]
    fn removed_line_at_start() {
        let left  = "a\nb\nc\n";
        let right = "b\nc\n";
        let lines = compute_diff(left, right, false);
        let removed: Vec<_> = lines.iter().filter(|l| l.left_status == DiffStatus::Removed).collect();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].left_text.as_deref(), Some("a"));
    }

    #[test]
    fn changed_line_produces_removed_and_added() {
        let left  = "foo bar\n";
        let right = "foo baz\n";
        let lines = compute_diff(left, right, false);
        assert!(lines.iter().any(|l| l.left_status  == DiffStatus::Removed));
        assert!(lines.iter().any(|l| l.right_status == DiffStatus::Added));
    }

    #[test]
    fn empty_inputs_produce_no_lines() {
        let lines = compute_diff("", "", false);
        assert!(lines.is_empty());
    }

    #[test]
    fn ignore_ws_treats_whitespace_diff_as_equal() {
        let left  = "int  x = 1;\n";
        let right = "int x = 1;\n";   // one fewer space
        // Without ignore_ws: different
        let strict = compute_diff(left, right, false);
        assert!(strict.iter().any(|l| l.left_status != DiffStatus::Equal));
        // With ignore_ws: same
        let lax = compute_diff(left, right, true);
        assert!(lax.iter().all(|l| l.left_status == DiffStatus::Equal));
    }

    #[test]
    fn line_numbers_increment_correctly() {
        let src = "a\nb\nc\n";
        let lines = compute_diff(src, src, false);
        for (i, l) in lines.iter().enumerate() {
            assert_eq!(l.left_lineno,  Some(i));
            assert_eq!(l.right_lineno, Some(i));
        }
    }

    // ── hunk_positions ────────────────────────────────────────────────────

    #[test]
    fn no_hunks_when_files_identical() {
        let src = "x\ny\nz\n";
        let lines = compute_diff(src, src, false);
        assert!(hunk_positions(&lines).is_empty());
    }

    #[test]
    fn single_hunk_detected() {
        let left  = "a\nb\nc\n";
        let right = "a\nX\nc\n";
        let lines = compute_diff(left, right, false);
        let hunks = hunk_positions(&lines);
        assert_eq!(hunks.len(), 1);
    }

    #[test]
    fn two_separate_hunks_detected() {
        let left  = "a\nb\nc\nd\ne\n";
        let right = "a\nX\nc\nd\nY\n";
        let lines = compute_diff(left, right, false);
        let hunks = hunk_positions(&lines);
        assert_eq!(hunks.len(), 2);
    }

    // ── context_view ─────────────────────────────────────────────────────

    #[test]
    fn context_view_no_changes_is_empty() {
        let src = "a\nb\nc\n";
        let lines = compute_diff(src, src, false);
        assert!(context_view(&lines, 2).is_empty());
    }

    #[test]
    fn context_view_includes_surrounding_lines() {
        let left  = "a\nb\nc\nd\ne\n";
        let right = "a\nb\nX\nd\ne\n";
        let lines = compute_diff(left, right, false);
        let ctx = context_view(&lines, 1);
        // Changed line is index 2 (c→X); with ctx=1 we expect indices 1,2,3
        assert!(ctx.contains(&1));
        assert!(ctx.contains(&2));
        assert!(ctx.contains(&3));
        assert!(!ctx.contains(&0)); // too far before
        assert!(!ctx.contains(&4)); // too far after
    }

    // ── inline_spans ──────────────────────────────────────────────────────

    #[test]
    fn inline_spans_identical_strings_all_unchanged() {
        let (ls, rs) = inline_spans("hello", "hello");
        assert!(ls.iter().all(|s| !s.changed));
        assert!(rs.iter().all(|s| !s.changed));
    }

    #[test]
    fn inline_spans_changed_byte_marked() {
        let (ls, rs) = inline_spans("bar", "baz");
        let l_changed: Vec<_> = ls.iter().filter(|s| s.changed).collect();
        let r_changed: Vec<_> = rs.iter().filter(|s| s.changed).collect();
        assert!(!l_changed.is_empty());
        assert!(!r_changed.is_empty());
    }

    #[test]
    fn inline_spans_cover_full_strings() {
        let l = "abcde";
        let r = "abXde";
        let (ls, rs) = inline_spans(l, r);
        let l_end = ls.iter().map(|s| s.end).max().unwrap_or(0);
        let r_end = rs.iter().map(|s| s.end).max().unwrap_or(0);
        assert_eq!(l_end, l.len());
        assert_eq!(r_end, r.len());
    }
}


