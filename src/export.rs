use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::Result;

use crate::text_diff::{DiffLine, DiffStatus};

/// Write a standard unified-diff patch file.
pub fn to_patch(
    diff_lines: &[DiffLine],
    left_path: &str,
    right_path: &str,
    output: &str,
    context: usize,
) -> Result<String> {
    let mut buf = BufWriter::new(File::create(output)?);
    writeln!(buf, "--- {}", left_path)?;
    writeln!(buf, "+++ {}", right_path)?;

    // Identify hunk ranges (changed lines ± context)
    let n = diff_lines.len();
    let mut in_range = vec![false; n];
    for (i, dl) in diff_lines.iter().enumerate() {
        if dl.left_status != DiffStatus::Equal || dl.right_status != DiffStatus::Equal {
            let s = i.saturating_sub(context);
            let e = (i + context + 1).min(n);
            for j in s..e {
                in_range[j] = true;
            }
        }
    }

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

        let left_start = diff_lines[hunk_start].left_lineno.unwrap_or(0) + 1;
        let right_start = diff_lines[hunk_start].right_lineno.unwrap_or(0) + 1;
        let left_count = diff_lines[hunk_start..hunk_end]
            .iter()
            .filter(|d| d.left_lineno.is_some())
            .count();
        let right_count = diff_lines[hunk_start..hunk_end]
            .iter()
            .filter(|d| d.right_lineno.is_some())
            .count();

        writeln!(
            buf,
            "@@ -{},{} +{},{} @@",
            left_start, left_count, right_start, right_count
        )?;

        for dl in &diff_lines[hunk_start..hunk_end] {
            if dl.left_status == DiffStatus::Removed || dl.left_status == DiffStatus::Equal {
                if let Some(t) = &dl.left_text {
                    let prefix = if dl.left_status == DiffStatus::Removed { '-' } else { ' ' };
                    writeln!(buf, "{}{}", prefix, t)?;
                }
            }
            if dl.right_status == DiffStatus::Added {
                if let Some(t) = &dl.right_text {
                    writeln!(buf, "+{}", t)?;
                }
            }
        }
    }

    Ok(output.to_string())
}

/// Write a self-contained HTML side-by-side diff.
pub fn to_html(
    diff_lines: &[DiffLine],
    left_path: &str,
    right_path: &str,
    output: &str,
) -> Result<String> {
    let mut buf = BufWriter::new(File::create(output)?);

    writeln!(buf, "<!DOCTYPE html>")?;
    writeln!(buf, "<html><head><meta charset='utf-8'>")?;
    writeln!(
        buf,
        "<title>astdiff: {} vs {}</title>",
        esc(left_path),
        esc(right_path)
    )?;
    writeln!(
        buf,
        r#"<style>
body{{font-family:monospace;font-size:13px;background:#1e1e2e;color:#cdd6f4;margin:0}}
h1{{padding:8px 12px;font-size:.9rem;background:#181825;color:#89b4fa;margin:0}}
table{{width:100%;border-collapse:collapse;table-layout:fixed}}
td,th{{padding:1px 6px;white-space:pre;overflow:hidden;text-overflow:ellipsis}}
th{{background:#181825;color:#585b70;font-weight:normal;border-bottom:1px solid #313244}}
.lno{{color:#585b70;user-select:none;width:4em;text-align:right;border-right:1px solid #313244}}
.eq{{}}
.add{{background:#1a2f1a;color:#a6e3a1}}
.rm{{background:#2f1a1a;color:#f38ba8}}
.sep{{background:#1e1e3e;color:#89b4fa;font-style:italic}}
</style></head><body>"#
    )?;
    writeln!(
        buf,
        "<h1>astdiff — {} vs {}</h1>",
        esc(left_path),
        esc(right_path)
    )?;
    writeln!(buf, "<table>")?;
    writeln!(
        buf,
        "<tr><th class='lno'></th><th>{}</th><th class='lno'></th><th>{}</th></tr>",
        esc(left_path),
        esc(right_path)
    )?;

    for dl in diff_lines {
        let lc = match dl.left_status {
            DiffStatus::Equal => "eq",
            DiffStatus::Removed => "rm",
            DiffStatus::Added => "eq",
        };
        let rc = match dl.right_status {
            DiffStatus::Equal => "eq",
            DiffStatus::Added => "add",
            DiffStatus::Removed => "eq",
        };

        let ll = dl
            .left_lineno
            .map(|n| (n + 1).to_string())
            .unwrap_or_default();
        let rl = dl
            .right_lineno
            .map(|n| (n + 1).to_string())
            .unwrap_or_default();
        let lt = esc(dl.left_text.as_deref().unwrap_or(""));
        let rt = esc(dl.right_text.as_deref().unwrap_or(""));

        writeln!(
            buf,
            "<tr><td class='lno'>{ll}</td><td class='{lc}'>{lt}</td>\
             <td class='lno' style='border-left:1px solid #313244'>{rl}</td><td class='{rc}'>{rt}</td></tr>"
        )?;
    }

    writeln!(buf, "</table></body></html>")?;
    Ok(output.to_string())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
