use std::fs;

use rayon::prelude::*;
use tree_sitter::{Node, Parser};

// ── Query model ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum QueryFilter {
    /// Plain text grep — no structural constraint
    Any,
    Function,  // fn:
    Call,      // call:
    Variable,  // var:
    Class,     // class:
    TypeRef,   // type:
    Include,   // include:
    Parameter, // param:
    Field,     // field:
}

impl QueryFilter {
    #[allow(dead_code)]
    pub fn label(&self) -> &str {
        match self {
            Self::Any => "text",
            Self::Function => "fn",
            Self::Call => "call",
            Self::Variable => "var",
            Self::Class => "class",
            Self::TypeRef => "type",
            Self::Include => "include",
            Self::Parameter => "param",
            Self::Field => "field",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub filter: QueryFilter,
}

impl SearchQuery {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

pub fn parse_query(raw: &str) -> SearchQuery {
    let prefixes: &[(&str, QueryFilter)] = &[
        ("fn:", QueryFilter::Function),
        ("call:", QueryFilter::Call),
        ("var:", QueryFilter::Variable),
        ("class:", QueryFilter::Class),
        ("type:", QueryFilter::TypeRef),
        ("include:", QueryFilter::Include),
        ("param:", QueryFilter::Parameter),
        ("field:", QueryFilter::Field),
    ];
    for (prefix, filter) in prefixes {
        if let Some(text) = raw.strip_prefix(prefix) {
            return SearchQuery { text: text.to_string(), filter: filter.clone() };
        }
    }
    SearchQuery { text: raw.to_string(), filter: QueryFilter::Any }
}

// ── Result model ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_path: String,
    pub line: usize,       // 0-indexed line of the match
    #[allow(dead_code)]
    pub col: usize,        // 0-indexed column
    pub snippet: String,   // short context string (~50 chars)
    pub node_kind: String, // tree-sitter kind of the matching node
}

impl SearchResult {
    pub fn short_path(&self) -> &str {
        std::path::Path::new(&self.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.file_path)
    }
}

// ── Project-wide parallel search ─────────────────────────────────────────

pub fn search_project(files: &[String], query: &SearchQuery) -> Vec<SearchResult> {
    if query.is_empty() {
        return vec![];
    }

    let mut results: Vec<SearchResult> = files
        .par_iter()
        .flat_map(|f| search_file(f, query))
        .collect();

    results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    results
}

// ── Single-file search ────────────────────────────────────────────────────

pub fn search_file(file_path: &str, query: &SearchQuery) -> Vec<SearchResult> {
    let Ok(src) = fs::read_to_string(file_path) else {
        return vec![];
    };
    search_src(file_path, &src, query)
}

pub fn search_src(file_path: &str, src: &str, query: &SearchQuery) -> Vec<SearchResult> {
    if query.filter == QueryFilter::Any {
        return grep(file_path, src, &query.text);
    }

    let Ok(mut parser) = make_parser() else {
        return vec![];
    };
    let Some(tree) = parser.parse(src, None) else {
        return vec![];
    };

    let target_kinds = kinds_for_filter(&query.filter);
    let needle = query.text.to_lowercase();
    let mut out = Vec::new();
    collect_matches(tree.root_node(), src, file_path, &target_kinds, &needle, &mut out);
    out
}

// ── Plain-text grep (Any filter) ─────────────────────────────────────────

fn grep(file_path: &str, src: &str, text: &str) -> Vec<SearchResult> {
    if text.is_empty() {
        return vec![];
    }
    let needle = text.to_lowercase();
    src.lines()
        .enumerate()
        .filter_map(|(line, content)| {
            let col = content.to_lowercase().find(&needle)?;
            Some(SearchResult {
                file_path: file_path.to_string(),
                line,
                col,
                snippet: make_snippet(content, col),
                node_kind: "text".into(),
            })
        })
        .take(500)
        .collect()
}

// ── AST walk ─────────────────────────────────────────────────────────────

fn collect_matches(
    node: Node<'_>,
    src: &str,
    file_path: &str,
    target_kinds: &[&str],
    needle: &str,
    out: &mut Vec<SearchResult>,
) {
    if target_kinds.contains(&node.kind()) {
        if let Some(node_src) = src.get(node.start_byte()..node.end_byte()) {
            if node_src.to_lowercase().contains(needle) {
                let line = node.start_position().row;
                let col = node.start_position().column;
                let line_text = src.lines().nth(line).unwrap_or("");
                out.push(SearchResult {
                    file_path: file_path.to_string(),
                    line,
                    col,
                    snippet: make_snippet(line_text, col),
                    node_kind: node.kind().to_string(),
                });
                // Don't recurse into matched node to avoid duplicates
                return;
            }
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            collect_matches(child, src, file_path, target_kinds, needle, out);
        }
    }
}

// ── Kind tables ───────────────────────────────────────────────────────────

fn kinds_for_filter(filter: &QueryFilter) -> Vec<&'static str> {
    match filter {
        QueryFilter::Any => vec![],
        QueryFilter::Function => vec![
            "function_definition",
            "function_declarator",
        ],
        QueryFilter::Call => vec!["call_expression"],
        QueryFilter::Variable => vec![
            "init_declarator",
            "declaration",
        ],
        QueryFilter::Class => vec![
            "class_specifier",
            "struct_specifier",
            "union_specifier",
            "enum_specifier",
        ],
        QueryFilter::TypeRef => vec!["type_identifier", "primitive_type"],
        QueryFilter::Include => vec!["preproc_include"],
        QueryFilter::Parameter => vec!["parameter_declaration"],
        QueryFilter::Field => vec!["field_declaration"],
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn make_snippet(line: &str, col: usize) -> String {
    let start = col.saturating_sub(4);
    line.chars().skip(start).take(48).collect::<String>().trim_end().to_string()
}

fn make_parser() -> anyhow::Result<Parser> {
    let mut p = Parser::new();
    p.set_language(&tree_sitter_cpp::language())?;
    Ok(p)
}
