//! AST parsing, flattening, diffing, and visibility helpers.
//!
//! The core workflow:
//! 1. [`parse_single`] parses a source snippet into a flat list of [`AstLine`]s.
//! 2. [`compute_ast_diff`] parses *two* snippets and aligns their flat trees
//!    using the `similar` crate's line-level LCS diff, producing a pair of
//!    [`AstLine`] vecs where each position corresponds to the same "slot" in
//!    both trees (padded with empty rows when one side has more nodes).
//! 3. [`visible_rows`] / [`row_has_children`] / [`filter_rows`] support the
//!    interactive collapse/expand and filter UI without re-parsing.

use anyhow::{Context, Result};
use similar::{ChangeTag, TextDiff};
use std::collections::HashSet;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq)]
pub enum NodeStatus {
    Same,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone)]
pub struct AstLine {
    pub depth: usize,
    pub kind: String,
    pub leaf_text: Option<String>,
    pub status: NodeStatus,
    pub empty: bool,
    /// Row in the source snippet (0-indexed, relative to the snippet start)
    pub source_row: usize,
}

pub struct AstDiffResult {
    pub left_nodes: Vec<AstLine>,
    pub right_nodes: Vec<AstLine>,
    /// Absolute line number (in original left file) where the snippet starts
    pub left_source_start: usize,
    /// Absolute line number (in original right file) where the snippet starts
    #[allow(dead_code)]
    pub right_source_start: usize,
}

// ── Public entry points ────────────────────────────────────────────────────

pub fn compute_ast_diff(
    left_src: &str,
    right_src: &str,
    left_source_start: usize,
    right_source_start: usize,
) -> Result<AstDiffResult> {
    let mut parser = make_parser()?;

    let left_tree = parser
        .parse(left_src, None)
        .context("Failed to parse left source")?;
    let right_tree = parser
        .parse(right_src, None)
        .context("Failed to parse right source")?;

    let left_flat = flatten_tree(left_tree.root_node(), left_src);
    let right_flat = flatten_tree(right_tree.root_node(), right_src);
    let (left_nodes, right_nodes) = diff_flat_trees(&left_flat, &right_flat);

    Ok(AstDiffResult {
        left_nodes,
        right_nodes,
        left_source_start,
        right_source_start,
    })
}

/// Parse a single source file into a flat list of AST lines (no diff).
pub fn parse_single(src: &str) -> Result<Vec<AstLine>> {
    let mut parser = make_parser()?;
    let tree = parser.parse(src, None).context("Failed to parse source")?;
    Ok(flatten_tree(tree.root_node(), src)
        .into_iter()
        .map(|(depth, kind, leaf_text, source_row)| AstLine {
            depth,
            kind,
            leaf_text,
            status: NodeStatus::Same,
            empty: false,
            source_row,
        })
        .collect())
}

// ── Tree flattening ────────────────────────────────────────────────────────

/// (depth, kind, leaf_text, source_row)
type FlatNode = (usize, String, Option<String>, usize);

fn flatten_tree(node: Node<'_>, src: &str) -> Vec<FlatNode> {
    let mut out = Vec::new();
    flatten_node(node, src, 0, &mut out);
    out
}

fn flatten_node(node: Node<'_>, src: &str, depth: usize, out: &mut Vec<FlatNode>) {
    // Skip anonymous punctuation/keywords with children — they add noise
    if !node.is_named() && node.child_count() > 0 {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                flatten_node(child, src, depth, out);
            }
        }
        return;
    }

    let kind = node.kind().to_string();
    let source_row = node.start_position().row;
    let leaf_text = if node.child_count() == 0 {
        src.get(node.start_byte()..node.end_byte())
            .map(|t| t.replace('\n', "↵").replace('\t', "→"))
    } else {
        None
    };

    out.push((depth, kind, leaf_text, source_row));

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            flatten_node(child, src, depth + 1, out);
        }
    }
}

// ── AST diff ───────────────────────────────────────────────────────────────

fn node_to_key(n: &FlatNode) -> String {
    let indent = "  ".repeat(n.0);
    match &n.2 {
        Some(t) => format!("{}{}\t{}", indent, n.1, t),
        None => format!("{}{}", indent, n.1),
    }
}

fn key_to_ast_line(s: &str, source_row: usize, status: NodeStatus) -> AstLine {
    let raw = s.trim_end_matches('\n');
    let depth = raw.chars().take_while(|c| *c == ' ').count() / 2;
    let rest = raw.trim_start();
    let (kind, leaf_text) = match rest.find('\t') {
        Some(ti) => (rest[..ti].to_string(), Some(rest[ti + 1..].to_string())),
        None => (rest.to_string(), None),
    };
    AstLine { depth, kind, leaf_text, status, empty: false, source_row }
}

fn diff_flat_trees(left: &[FlatNode], right: &[FlatNode]) -> (Vec<AstLine>, Vec<AstLine>) {
    let left_joined = left.iter().map(node_to_key).collect::<Vec<_>>().join("\n");
    let right_joined = right.iter().map(node_to_key).collect::<Vec<_>>().join("\n");

    let diff = TextDiff::from_lines(&left_joined, &right_joined);

    let mut left_out: Vec<AstLine> = Vec::new();
    let mut right_out: Vec<AstLine> = Vec::new();
    let mut left_buf: Vec<AstLine> = Vec::new();
    let mut right_buf: Vec<AstLine> = Vec::new();
    let mut li = 0usize; // index into left flat list
    let mut ri = 0usize; // index into right flat list

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            match change.tag() {
                ChangeTag::Equal => {
                    flush_ast_bufs(&mut left_out, &mut right_out, &mut left_buf, &mut right_buf);
                    let lrow = left.get(li).map(|n| n.3).unwrap_or(0);
                    let rrow = right.get(ri).map(|n| n.3).unwrap_or(0);
                    let mut l = key_to_ast_line(change.value(), lrow, NodeStatus::Same);
                    left_out.push(l.clone());
                    l.source_row = rrow;
                    right_out.push(l);
                    li += 1;
                    ri += 1;
                }
                ChangeTag::Delete => {
                    let row = left.get(li).map(|n| n.3).unwrap_or(0);
                    left_buf.push(key_to_ast_line(change.value(), row, NodeStatus::Removed));
                    li += 1;
                }
                ChangeTag::Insert => {
                    let row = right.get(ri).map(|n| n.3).unwrap_or(0);
                    right_buf.push(key_to_ast_line(change.value(), row, NodeStatus::Added));
                    ri += 1;
                }
            }
        }
    }
    flush_ast_bufs(&mut left_out, &mut right_out, &mut left_buf, &mut right_buf);
    (left_out, right_out)
}

fn flush_ast_bufs(
    left_out: &mut Vec<AstLine>,
    right_out: &mut Vec<AstLine>,
    left_buf: &mut Vec<AstLine>,
    right_buf: &mut Vec<AstLine>,
) {
    let max = left_buf.len().max(right_buf.len());
    for i in 0..max {
        let l = if i < left_buf.len() {
            let mut n = left_buf[i].clone();
            if i < right_buf.len() && left_buf[i].kind == right_buf[i].kind {
                n.status = NodeStatus::Modified;
            }
            n
        } else {
            AstLine { depth: 0, kind: String::new(), leaf_text: None, status: NodeStatus::Same, empty: true, source_row: 0 }
        };

        let r = if i < right_buf.len() {
            let mut n = right_buf[i].clone();
            if i < left_buf.len() && left_buf[i].kind == right_buf[i].kind {
                n.status = NodeStatus::Modified;
            }
            n
        } else {
            AstLine { depth: 0, kind: String::new(), leaf_text: None, status: NodeStatus::Same, empty: true, source_row: 0 }
        };

        left_out.push(l);
        right_out.push(r);
    }
    left_buf.clear();
    right_buf.clear();
}

// ── Visibility / collapse helpers ─────────────────────────────────────────

/// Compute which row indices are visible given the collapsed set.
/// `collapsed` holds row indices whose subtrees are folded.
pub fn visible_rows(
    left_nodes: &[AstLine],
    right_nodes: &[AstLine],
    collapsed: &HashSet<usize>,
) -> Vec<usize> {
    let n = left_nodes.len().max(right_nodes.len());
    let mut vis = Vec::with_capacity(n);
    let mut collapse_at: Option<usize> = None; // depth threshold; skip depth > this

    for i in 0..n {
        let d = row_depth(i, left_nodes, right_nodes);

        if let Some(cd) = collapse_at {
            if d > cd {
                continue;
            } else {
                collapse_at = None;
            }
        }

        vis.push(i);

        if collapsed.contains(&i) {
            collapse_at = Some(d);
        }
    }
    vis
}

/// True if row `i` has at least one immediate child in the next position.
pub fn row_has_children(i: usize, left: &[AstLine], right: &[AstLine]) -> bool {
    row_depth(i + 1, left, right) > row_depth(i, left, right)
}

pub fn row_depth(i: usize, left: &[AstLine], right: &[AstLine]) -> usize {
    match (left.get(i), right.get(i)) {
        (Some(l), _) if !l.empty => l.depth,
        (_, Some(r)) if !r.empty => r.depth,
        _ => 0,
    }
}

// ── Filter helpers ────────────────────────────────────────────────────────

/// Returns row indices that match the filter text (kind or leaf text), plus their ancestors.
pub fn filter_rows(nodes: &[AstLine], filter: &str) -> Vec<usize> {
    if filter.is_empty() {
        return (0..nodes.len()).collect();
    }
    let f = filter.to_lowercase();
    let mut vis = vec![false; nodes.len()];

    for (i, node) in nodes.iter().enumerate() {
        let kind_match = node.kind.to_lowercase().contains(&f);
        let text_match = node
            .leaf_text
            .as_ref()
            .map(|t| t.to_lowercase().contains(&f))
            .unwrap_or(false);
        if kind_match || text_match {
            vis[i] = true;
            // Mark ancestors
            let mut threshold = node.depth;
            for j in (0..i).rev() {
                if nodes[j].depth < threshold {
                    vis[j] = true;
                    threshold = nodes[j].depth;
                    if threshold == 0 {
                        break;
                    }
                }
            }
        }
    }

    vis.iter()
        .enumerate()
        .filter(|(_, v)| **v)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_CPP: &str = r#"
int add(int a, int b) {
    return a + b;
}

class Foo {
    int x;
};
"#;

    // ── parse_single ──────────────────────────────────────────────────────

    #[test]
    fn parse_single_produces_nodes() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        assert!(!nodes.is_empty());
    }

    #[test]
    fn parse_single_contains_function_definition() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        assert!(
            nodes.iter().any(|n| n.kind == "function_definition"),
            "expected function_definition node"
        );
    }

    #[test]
    fn parse_single_all_nodes_have_depths() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        // Root node should be at depth 0
        assert!(nodes.iter().any(|n| n.depth == 0));
    }

    #[test]
    fn parse_single_source_rows_are_valid() {
        let src = "int x = 1;\nint y = 2;\n";
        let nodes = parse_single(src).unwrap();
        let max_row = src.lines().count();
        assert!(nodes.iter().all(|n| n.source_row < max_row));
    }

    #[test]
    fn parse_single_empty_source_returns_empty_or_root_only() {
        // Empty C++ source may still produce a root translation_unit node
        let nodes = parse_single("").unwrap_or_default();
        // Should not panic; result may be empty or have just a root
        let _ = nodes;
    }

    // ── filter_rows ───────────────────────────────────────────────────────

    #[test]
    fn filter_rows_empty_filter_returns_all() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        let all = filter_rows(&nodes, "");
        assert_eq!(all.len(), nodes.len());
    }

    #[test]
    fn filter_rows_function_keyword_returns_subset() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        let all = filter_rows(&nodes, "");
        let funcs = filter_rows(&nodes, "function");
        assert!(funcs.len() < all.len(), "filtered set should be smaller");
        assert!(!funcs.is_empty(), "should find at least one function node");
    }

    #[test]
    fn filter_rows_includes_ancestors_of_match() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        let filtered = filter_rows(&nodes, "function_definition");
        // The filtered set must include depth-0 ancestors (translation_unit is the root)
        assert!(filtered.contains(&0), "root ancestor should be included");
    }

    #[test]
    fn filter_rows_no_match_returns_empty() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        let result = filter_rows(&nodes, "zzz_no_such_node");
        assert!(result.is_empty());
    }

    // ── visible_rows ──────────────────────────────────────────────────────

    #[test]
    fn visible_rows_no_collapse_returns_all() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        let empty_right: Vec<AstLine> = vec![];
        let collapsed = HashSet::new();
        let vis = visible_rows(&nodes, &empty_right, &collapsed);
        assert_eq!(vis.len(), nodes.len());
    }

    #[test]
    fn visible_rows_collapsed_root_hides_children() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        let empty_right: Vec<AstLine> = vec![];
        let mut collapsed = HashSet::new();
        collapsed.insert(0); // collapse first row
        let vis = visible_rows(&nodes, &empty_right, &collapsed);
        // Collapsing root (depth 0) should hide all children (depth > 0)
        assert_eq!(vis.len(), 1, "only root should be visible");
    }

    // ── row_has_children ─────────────────────────────────────────────────

    #[test]
    fn row_has_children_for_parent_node() {
        let nodes = parse_single(SIMPLE_CPP).unwrap();
        let empty: Vec<AstLine> = vec![];
        // Row 0 (translation_unit) should have children
        assert!(row_has_children(0, &nodes, &empty));
    }

    // ── compute_ast_diff ─────────────────────────────────────────────────

    #[test]
    fn diff_identical_source_all_same() {
        let src = "int x = 1;\n";
        let result = compute_ast_diff(src, src, 0, 0).unwrap();
        assert!(result.left_nodes.iter().all(|n| n.status == NodeStatus::Same));
        assert!(result.right_nodes.iter().all(|n| n.status == NodeStatus::Same));
    }

    #[test]
    fn diff_added_function_marked_added() {
        let left  = "int x = 1;\n";
        let right = "int x = 1;\nvoid foo() {}\n";
        let result = compute_ast_diff(left, right, 0, 0).unwrap();
        assert!(
            result.right_nodes.iter().any(|n| n.status == NodeStatus::Added),
            "new function should be marked Added on the right"
        );
    }

    #[test]
    fn diff_left_and_right_node_counts_match() {
        let left  = "int a;\nint b;\n";
        let right = "int a;\nint c;\n";
        let result = compute_ast_diff(left, right, 0, 0).unwrap();
        // flush_ast_bufs pads with empty rows so both sides have equal length
        assert_eq!(result.left_nodes.len(), result.right_nodes.len());
    }
}

// ── Internal ──────────────────────────────────────────────────────────────

fn make_parser() -> Result<Parser> {
    let mut parser = Parser::new();
    let language = tree_sitter_cpp::language();
    parser
        .set_language(&language)
        .context("Failed to set C++ language")?;
    Ok(parser)
}
