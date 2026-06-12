//! Code search engine backed by tree-sitter queries and plain-text grep.
//!
//! The central type is [`SearchQuery`], produced by [`parse_query`] from
//! a user-facing string. Queries are either:
//!
//! * **Shorthand prefixes** (`fn:`, `call:`, `var:`, …) that expand to
//!   pre-written tree-sitter S-expression queries with an optional text
//!   filter applied to the captured node.
//! * **Raw tree-sitter S-expressions** (starting with `(` or `[`) forwarded
//!   directly to the query engine.
//! * **Plain-text grep** (anything else) — case-insensitive substring search.
//!
//! [`search_project`] runs searches in parallel across all files using Rayon
//! and returns deduplicated, sorted [`SearchResult`]s.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::SystemTime;

use rayon::prelude::*;
use regex::Regex;
use tree_sitter::{Parser, Query, QueryCursor, Tree};

use crate::ast_cache::AstCache;


// ── Predefined shorthand queries ──────────────────────────────────────────
// Each query must have exactly one capture group named @match.
// Multiple top-level patterns are fine — tree-sitter unions them.

const QUERY_FN: &str = "
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @match))
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @match)))
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (identifier) @match)))
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (qualified_identifier
        name: (identifier) @match))))
";

const QUERY_CALL: &str = "
(call_expression function: (identifier) @match)
(call_expression function: (field_expression field: (field_identifier) @match))
";

const QUERY_VAR: &str = "
(declaration declarator: (identifier) @match)
(declaration declarator: (init_declarator declarator: (identifier) @match))
(declaration declarator: (pointer_declarator declarator: (identifier) @match))
(declaration declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @match)))
(declaration declarator: (reference_declarator (identifier) @match))
(declaration declarator: (init_declarator declarator: (reference_declarator (identifier) @match)))
";

const QUERY_CLASS: &str = "
(class_specifier name: (type_identifier) @match)
(struct_specifier name: (type_identifier) @match)
(enum_specifier name: (type_identifier) @match)
";

const QUERY_TYPE: &str = "(type_identifier) @match";

const QUERY_INCLUDE: &str = "(preproc_include path: _ @match)";

const QUERY_PARAM: &str = "(parameter_declaration declarator: (identifier) @match)";

const QUERY_FIELD: &str = "(field_declaration declarator: (field_identifier) @match)";

const QUERY_LAMBDA: &str = "(lambda_expression) @match";

const QUERY_MACRO: &str = "
(preproc_def name: (identifier) @match)
(preproc_function_def name: (identifier) @match)
";

const QUERY_NS: &str = "
(namespace_definition name: (identifier) @match)
";

const QUERY_OP: &str = "
(function_definition
  declarator: (function_declarator
    declarator: (operator_name) @match))
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (operator_name) @match)))
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (operator_name) @match)))
";

const QUERY_USING: &str = "
(using_declaration declarator: _ @match)
";

const QUERY_TPL: &str = "
(template_declaration
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @match)))
(template_declaration
  (function_definition
    declarator: (function_declarator
      declarator: (qualified_identifier
        name: (identifier) @match))))
(template_declaration
  (class_specifier name: (type_identifier) @match))
(template_declaration
  (struct_specifier name: (type_identifier) @match))
";

const QUERY_THROW: &str = "
(throw_statement) @match
";

const QUERY_CAST: &str = "
(cast_expression type: _ @match)
(static_cast_expression type: _ @match)
(dynamic_cast_expression type: _ @match)
(reinterpret_cast_expression type: _ @match)
(const_cast_expression type: _ @match)
";

// ── Query model ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Raw text the user typed
    pub raw: String,
    /// Tree-sitter S-expression query source (empty in grep mode)
    pub ts_query_src: String,
    /// Substring filter applied to the captured node's text (empty = show all)
    pub capture_filter: String,
    /// True → ignore ts_query_src and do plain-text grep
    pub grep_mode: bool,
    /// True → skip captures whose containing call_expression is inside an argument_list
    pub filter_nested_calls: bool,
    /// True → treat `raw` (grep) / `capture_filter` (AST) as a regular expression.
    pub use_regex: bool,
    /// True → match text exactly as typed; false (default) → case-insensitive.
    pub case_sensitive: bool,
}

impl SearchQuery {
    pub fn is_empty(&self) -> bool {
        self.raw.trim().is_empty()
    }
}

/// Parse user input into a `SearchQuery`.
///
/// Rules:
/// - Starts with `(` or `[`  → raw tree-sitter query, no text filter
/// - `fn:text`, `call:text`, … → expand to predefined query, filter captures by `text`
/// - anything else            → plain-text grep
pub fn parse_query(raw: &str) -> SearchQuery {
    let _s = crate::tracer::span("search::parse_query");
    let trimmed = raw.trim();

    // Raw tree-sitter query
    if trimmed.starts_with('(') || trimmed.starts_with('[') {
        log::debug!("parse_query: raw ts-query ({} chars)", trimmed.len());
        return SearchQuery {
            raw: raw.to_string(),
            ts_query_src: trimmed.to_string(),
            capture_filter: String::new(),
            grep_mode: false,
            filter_nested_calls: false,
            use_regex: false,
            case_sensitive: false,
        };
    }

    // Shorthand prefixes → predefined queries
    let shorthands: &[(&str, &str, bool)] = &[
        ("fn:",      QUERY_FN,      false),
        ("call:",    QUERY_CALL,    true),  // skip calls nested inside argument lists
        ("var:",     QUERY_VAR,     false),
        ("class:",   QUERY_CLASS,   false),
        ("type:",    QUERY_TYPE,    false),
        ("include:", QUERY_INCLUDE, false),
        ("param:",   QUERY_PARAM,   false),
        ("field:",   QUERY_FIELD,   false),
        ("lambda:",  QUERY_LAMBDA,  false),
        ("macro:",   QUERY_MACRO,   false),
        ("ns:",      QUERY_NS,      false),
        ("op:",      QUERY_OP,      false),
        ("using:",   QUERY_USING,   false),
        ("tpl:",     QUERY_TPL,     false),
        ("throw:",   QUERY_THROW,   false),
        ("cast:",    QUERY_CAST,    false),
    ];
    for (prefix, ts_query, filter_nested) in shorthands {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            log::debug!(
                "parse_query: shorthand {:?} filter={:?} nested_filter={}",
                prefix, rest.trim(), filter_nested
            );
            return SearchQuery {
                raw: raw.to_string(),
                ts_query_src: ts_query.to_string(),
                capture_filter: rest.trim().to_string(),
                grep_mode: false,
                filter_nested_calls: *filter_nested,
                use_regex: false,
                case_sensitive: false,
            };
        }
    }

    // Plain-text grep
    log::debug!("parse_query: plain grep {:?}", raw);
    SearchQuery {
        raw: raw.to_string(),
        ts_query_src: String::new(),
        capture_filter: String::new(),
        grep_mode: true,
        filter_nested_calls: false,
        use_regex: false,
        case_sensitive: false,
    }
}

// ── Result model ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_path: String,
    pub line: usize,
    #[allow(dead_code)]
    pub col: usize,
    /// End of the matched node/text (0-based, inclusive line / exclusive col),
    /// so editors can highlight the exact match range.
    pub end_line: usize,
    pub end_col: usize,
    pub snippet: String,
    /// Actual tree-sitter node kind of the captured node (e.g. "identifier")
    pub node_kind: String,
    /// Name of the capture in the query (e.g. "match", "name")
    pub capture_name: String,
}

impl SearchResult {
    pub fn short_path(&self) -> &str {
        std::path::Path::new(&self.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.file_path)
    }
}

// ── File-path filter (include / exclude boxes) ────────────────────────────

/// A set of glob patterns parsed from one filter box (comma-separated).
///
/// Supports:
/// - `*`  — any characters except `/`
/// - `**` — any characters including `/`
/// - `?`  — any single character except `/`
///
/// If the pattern contains no `/`, it is matched against the **filename only**.
/// Otherwise it is matched against the **relative path** from the project root.
#[derive(Debug, Clone, Default)]
pub struct FileFilter {
    patterns: Vec<String>,
}

impl FileFilter {
    pub fn parse(raw: &str) -> Self {
        let patterns = raw
            .split(',')
            .flat_map(|s| s.split(';'))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self { patterns }
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Returns `true` if `path` matches **any** of the patterns.
    pub fn matches(&self, path: &str) -> bool {
        if self.patterns.is_empty() {
            return true;
        }
        self.patterns.iter().any(|pat| glob_match(pat, path))
    }
}

/// Match `path` against a glob `pattern`.
fn glob_match(pattern: &str, path: &str) -> bool {
    // Normalise separators.
    let path_norm: String = path.replace('\\', "/");
    let p = path_norm.as_str();

    // Patterns without `/` match against the filename component only.
    if !pattern.contains('/') {
        let fname = p.rsplit('/').next().unwrap_or(p);
        return glob_bytes(pattern.as_bytes(), fname.as_bytes());
    }

    glob_bytes(pattern.as_bytes(), p.as_bytes())
}

/// Recursive byte-level glob matcher.
/// `*`  matches any sequence of bytes that is not `/`.
/// `**` matches any sequence of bytes including `/`.
/// `?`  matches exactly one byte that is not `/`.
fn glob_bytes(p: &[u8], t: &[u8]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some(b'*') if p.get(1) == Some(&b'*') => {
            // `**` — consume optional trailing slash in pattern
            let rest = match p.get(2) {
                Some(b'/') => &p[3..],
                _           => &p[2..],
            };
            // Try: ** matches zero components (nothing consumed)
            if glob_bytes(rest, t) { return true; }
            // Try: ** matches one or more path components
            let mut i = 0;
            while i < t.len() {
                if t[i] == b'/' && glob_bytes(rest, &t[i + 1..]) {
                    return true;
                }
                i += 1;
            }
            // Also try matching rest against the very end (last component)
            glob_bytes(rest, &t[t.len()..]) // empty suffix
        }
        Some(b'*') => {
            // Single `*` — does not cross `/`
            let rest = &p[1..];
            if glob_bytes(rest, t) { return true; }
            let mut i = 0;
            while i < t.len() && t[i] != b'/' {
                i += 1;
                if glob_bytes(rest, &t[i..]) { return true; }
            }
            false
        }
        Some(b'?') => match t.first() {
            Some(&c) if c != b'/' => glob_bytes(&p[1..], &t[1..]),
            _ => false,
        },
        Some(&pc) => match t.first() {
            Some(&tc) if tc == pc => glob_bytes(&p[1..], &t[1..]),
            _ => false,
        },
    }
}

// ── Pre-compiled query (compile once, reuse across all files) ────────────

/// Everything expensive that is identical for every file in one search pass.
/// `Query::new` and regex compilation are moved here so the parallel workers
/// only pay for parsing + cursor traversal.
struct PreparedSearch {
    ts_query: Query,
    capture_regex: Option<Regex>,
    capture_filter_cmp: String,
}

impl PreparedSearch {
    fn new(query: &SearchQuery) -> Option<Self> {
        if query.grep_mode || query.ts_query_src.is_empty() {
            return None;
        }
        let lang = tree_sitter_cpp::language();
        let ts_query = match Query::new(&lang, &query.ts_query_src) {
            Ok(q) => q,
            Err(e) => {
                log::warn!("query compile error: {}", e);
                return None;
            }
        };
        let capture_filter_cmp = if query.case_sensitive {
            query.capture_filter.clone()
        } else {
            query.capture_filter.to_lowercase()
        };
        let capture_regex = if query.use_regex && !query.capture_filter.is_empty() {
            match regex::RegexBuilder::new(&query.capture_filter)
                .case_insensitive(!query.case_sensitive)
                .build()
            {
                Ok(re) => Some(re),
                Err(e) => {
                    log::warn!("capture filter regex error: {}", e);
                    return None;
                }
            }
        } else {
            None
        };
        Some(Self { ts_query, capture_regex, capture_filter_cmp })
    }
}

// ── Project-wide parallel search ─────────────────────────────────────────

/// Streaming search — sends per-file result batches through `tx` as soon as
/// each file is processed (using Rayon's thread pool).  Closes the channel
/// when all files are done so the receiver can detect completion via
/// `TryRecvError::Disconnected`.  Checks `cancel` before processing each
/// file; set it to `true` to stop early (e.g. when a new search starts).
///
/// Returns the number of files served from the AST cache (warm hits),
/// so callers can report cache effectiveness.
pub fn search_project_streaming(
    files: &[String],
    query: &SearchQuery,
    tx: mpsc::SyncSender<Vec<SearchResult>>,
    cancel: Arc<AtomicBool>,
    ast_cache: Arc<Mutex<AstCache>>,
) -> usize {
    let _s = crate::tracer::span("search::project_streaming");
    if query.is_empty() {
        return 0;
    }
    log::info!(
        "search (stream): mode={} files={}",
        if query.grep_mode { "grep" } else { "ts-query" },
        files.len(),
    );
    let prepared = PreparedSearch::new(query); // compile query once, share across workers

    // ── Phase 1: build cached-tree map (single-threaded, before Rayon) ─────
    // Skipped for grep queries — no tree-sitter parse cost to save.
    let cached_trees: HashMap<PathBuf, Tree> = if !query.grep_mode {
        let mut cache = ast_cache.lock().unwrap();
        if cache.is_enabled() {
            files.iter().filter_map(|f| {
                let path = PathBuf::from(f);
                let mtime = fs::metadata(&path).ok()?.modified().ok()?;
                cache.get(&path, mtime).map(|tree| (path, tree))
            }).collect()
        } else {
            HashMap::new()
        }
        // MutexGuard dropped here — lock released before Rayon
    } else {
        HashMap::new()
    };
    let cache_hits = cached_trees.len();

    // ── Phase 3 accumulator: workers push new entries here ──────────────────
    let new_entries: Mutex<Vec<NewCacheEntry>> = Mutex::new(Vec::new());

    // ── Phase 2: parallel search ─────────────────────────────────────────────
    files.par_iter().for_each_with(tx, |tx, f| {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let path = PathBuf::from(f.as_str());
        let cached = cached_trees.get(&path);
        let (hits, new_entry) = search_file_prepared(f, query, prepared.as_ref(), cached);
        if let Some(entry) = new_entry {
            new_entries.lock().unwrap().push(entry);
        }
        if !hits.is_empty() {
            log::debug!("search: {} hits in {}", hits.len(), f);
            let _ = tx.send(hits); // silently drop if receiver gone (cancelled)
        }
    });
    // tx is dropped here → channel closes → receiver sees Disconnected

    // ── Phase 3: merge newly parsed trees into cache ─────────────────────────
    let entries = new_entries.into_inner().unwrap();
    if !entries.is_empty() {
        let n = entries.len();
        let mut cache = ast_cache.lock().unwrap();
        for e in entries {
            cache.insert(e.path, e.mtime, e.tree, e.source_len);
        }
        log::debug!("ast_cache: inserted {} new entries", n);
    }

    cache_hits
}

// Carried from search_file_prepared back up to search_project_streaming for caching.
struct NewCacheEntry {
    path: PathBuf,
    mtime: SystemTime,
    tree: Tree,
    source_len: usize,
}

#[allow(dead_code)] // used by integration tests
pub fn search_project(files: &[String], query: &SearchQuery) -> Vec<SearchResult> {
    let _s = crate::tracer::span("search::project");
    if query.is_empty() {
        return vec![];
    }
    log::info!(
        "search: mode={} ts_query={:?} filter={:?} files={}",
        if query.grep_mode { "grep" } else { "ts-query" },
        if query.grep_mode { &query.raw } else { &query.ts_query_src },
        query.capture_filter,
        files.len(),
    );
    let prepared = PreparedSearch::new(query); // compile query once, share across workers
    let mut results: Vec<SearchResult> = files
        .par_iter()
        .flat_map(|f| {
            let (hits, _) = search_file_prepared(f, query, prepared.as_ref(), None);
            if !hits.is_empty() {
                log::debug!("search: {} hits in {}", hits.len(), f);
            }
            hits
        })
        .collect();
    results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    log::info!("search: {} total results across {} files", results.len(), files.len());
    results
}

// ── Single-file search ────────────────────────────────────────────────────

/// Used by the parallel project search — reads the file then delegates to
/// `search_src_prepared` which reuses the already-compiled `PreparedSearch`.
/// Returns search results and, on a cache miss parse, a `NewCacheEntry` to store.
fn search_file_prepared(
    file_path: &str,
    query: &SearchQuery,
    prepared: Option<&PreparedSearch>,
    cached_tree: Option<&Tree>,
) -> (Vec<SearchResult>, Option<NewCacheEntry>) {
    let _s = crate::tracer::span("search::file");
    let _io = crate::tracer::span("search::file::read");
    let Ok(src) = fs::read_to_string(file_path) else {
        log::warn!("cannot read {}: skipping", file_path);
        return (vec![], None);
    };
    drop(_io);
    match prepared {
        Some(p) => {
            let (results, new_tree) = search_src_prepared(file_path, &src, query, p, cached_tree);
            let new_entry = new_tree.and_then(|tree| {
                let mtime = fs::metadata(file_path).ok()?.modified().ok()?;
                Some(NewCacheEntry {
                    path: PathBuf::from(file_path),
                    mtime,
                    tree,
                    source_len: src.len(),
                })
            });
            (results, new_entry)
        }
        None => (search_src(file_path, &src, query), None),
    }
}

/// Core search against already-loaded source, using a pre-compiled query.
/// Returns results and, when the tree was freshly parsed (no cache hit),
/// the owned `Tree` so the caller can store it in the AST cache.
fn search_src_prepared(
    file_path: &str,
    src: &str,
    query: &SearchQuery,
    prepared: &PreparedSearch,
    cached_tree: Option<&Tree>,
) -> (Vec<SearchResult>, Option<Tree>) {
    let _s = crate::tracer::span("search::src");

    // Cheap pre-filter: if there's a capture filter, confirm the text exists in
    // the file before paying the tree-sitter parse cost.  Eliminates ~95% of
    // parse calls for typed queries like `fn:update` or `call:render`.
    if !prepared.capture_filter_cmp.is_empty() {
        let _pf = crate::tracer::span("search::src::prefilter");
        let hit = if let Some(ref re) = prepared.capture_regex {
            re.is_match(src)
        } else if query.case_sensitive {
            src.contains(prepared.capture_filter_cmp.as_str())
        } else {
            // case-insensitive: scan without allocating a lowercased copy by
            // using a byte-level ASCII fold (sufficient for identifier names).
            src.as_bytes()
                .windows(prepared.capture_filter_cmp.len())
                .any(|w| w.eq_ignore_ascii_case(prepared.capture_filter_cmp.as_bytes()))
        };
        if !hit {
            return (vec![], None);
        }
    }

    // Use the cached tree if available; otherwise parse.
    // `for_cache` is Some(tree) when we parsed (so the caller can cache it).
    let (tree, for_cache): (Tree, Option<Tree>) = if let Some(t) = cached_tree {
        (t.clone(), None) // cheap ts_tree_copy; no new entry needed
    } else {
        let lang = tree_sitter_cpp::language();
        let mut parser = {
            let _t = crate::tracer::span("search::src::parser_init");
            let mut p = Parser::new();
            if p.set_language(&lang).is_err() {
                return (vec![], None);
            }
            p
        };
        let _t = crate::tracer::span("search::src::parse");
        match parser.parse(src, None) {
            Some(t) => {
                let cache_copy = t.clone();
                (t, Some(cache_copy))
            }
            None => return (vec![], None),
        }
    };

    let mut cursor = QueryCursor::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut out = Vec::new();

    let _walk = crate::tracer::span("search::src::walk_captures");
    for (m, capture_idx) in cursor.captures(&prepared.ts_query, tree.root_node(), src.as_bytes()) {
        if out.len() >= MAX_RESULTS_PER_FILE {
            log::debug!("per-file cap ({}) reached in {}", MAX_RESULTS_PER_FILE, file_path);
            break;
        }

        let capture = m.captures[capture_idx];
        let node = capture.node;
        let byte_start = node.start_byte();

        if !seen.insert(byte_start) {
            continue;
        }

        if query.filter_nested_calls && call_is_nested(node) {
            continue;
        }

        if let Some(ref re) = prepared.capture_regex {
            let node_text = src.get(byte_start..node.end_byte()).unwrap_or("");
            if !re.is_match(node_text) {
                continue;
            }
        } else if !prepared.capture_filter_cmp.is_empty() {
            let node_text = src.get(byte_start..node.end_byte()).unwrap_or("");
            let cmp_text = if query.case_sensitive {
                node_text.to_string()
            } else {
                node_text.to_lowercase()
            };
            if !cmp_text.contains(&prepared.capture_filter_cmp) {
                continue;
            }
        }

        let line = node.start_position().row;
        let col = node.start_position().column;
        let line_text = src.lines().nth(line).unwrap_or("");
        let capture_name = prepared.ts_query.capture_names()[capture.index as usize].to_string();

        out.push(SearchResult {
            file_path: file_path.to_string(),
            line,
            col,
            end_line: node.end_position().row,
            end_col: node.end_position().column,
            snippet: make_snippet(line_text, col),
            node_kind: node.kind().to_string(),
            capture_name,
        });
    }

    (out, for_cache)
}

#[allow(dead_code)] // used by integration tests and bench binary
pub fn search_file(file_path: &str, query: &SearchQuery) -> Vec<SearchResult> {
    let _s = crate::tracer::span("search::file");
    let _io = crate::tracer::span("search::file::read");
    let Ok(src) = fs::read_to_string(file_path) else {
        log::warn!("cannot read {}: skipping", file_path);
        return vec![];
    };
    drop(_io);
    search_src(file_path, &src, query)
}

pub fn search_src(file_path: &str, src: &str, query: &SearchQuery) -> Vec<SearchResult> {
    let _s = crate::tracer::span("search::src");
    if query.grep_mode {
        let _g = crate::tracer::span("search::src::grep");
        return grep(file_path, src, &query.raw, query.use_regex, query.case_sensitive);
    }
    if query.ts_query_src.is_empty() {
        return vec![];
    }

    let lang = tree_sitter_cpp::language();

    let ts_query = {
        let _t = crate::tracer::span("search::src::query_compile");
        match Query::new(&lang, &query.ts_query_src) {
            Ok(q) => q,
            Err(e) => {
                log::warn!("query compile error in {}: {}", file_path, e);
                return vec![];
            }
        }
    };

    let mut parser = {
        let _t = crate::tracer::span("search::src::parser_init");
        let mut p = Parser::new();
        if p.set_language(&lang).is_err() {
            return vec![];
        }
        p
    };

    let tree = {
        let _t = crate::tracer::span("search::src::parse");
        match parser.parse(src, None) {
            Some(t) => t,
            None => return vec![],
        }
    };

    // Compile the capture filter — regex or plain substring.
    // We do this once per file rather than per-capture to avoid repeated compilation.
    let _fc = crate::tracer::span("search::src::filter_compile");
    let capture_filter_cmp = if query.case_sensitive {
        query.capture_filter.clone()
    } else {
        query.capture_filter.to_lowercase()
    };
    let capture_regex: Option<Regex> = if query.use_regex && !query.capture_filter.is_empty() {
        match regex::RegexBuilder::new(&query.capture_filter)
            .case_insensitive(!query.case_sensitive)
            .build()
        {
            Ok(re) => Some(re),
            Err(e) => {
                log::warn!("regex compile error for capture filter {:?}: {}", query.capture_filter, e);
                return vec![];
            }
        }
    } else {
        None
    };
    drop(_fc);

    let mut cursor = QueryCursor::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut out = Vec::new();

    let _walk = crate::tracer::span("search::src::walk_captures");
    for (m, capture_idx) in cursor.captures(&ts_query, tree.root_node(), src.as_bytes()) {
        if out.len() >= MAX_RESULTS_PER_FILE {
            log::debug!("per-file cap ({}) reached in {}", MAX_RESULTS_PER_FILE, file_path);
            break;
        }

        let capture = m.captures[capture_idx];
        let node = capture.node;
        let byte_start = node.start_byte();

        // Deduplicate: same byte position captured by multiple patterns
        if !seen.insert(byte_start) {
            continue;
        }

        // Skip calls nested inside the argument list of another call
        if query.filter_nested_calls && call_is_nested(node) {
            log::trace!(
                "nested call filtered: {:?} at {}:{}",
                src.get(byte_start..node.end_byte()).unwrap_or("?"),
                node.start_position().row + 1,
                node.start_position().column
            );
            continue;
        }

        // Text filter on the captured node's source text — regex or plain substring.
        if let Some(ref re) = capture_regex {
            let node_text = src.get(byte_start..node.end_byte()).unwrap_or("");
            if !re.is_match(node_text) {
                continue;
            }
        } else if !capture_filter_cmp.is_empty() {
            let node_text = src.get(byte_start..node.end_byte()).unwrap_or("");
            let cmp_text = if query.case_sensitive {
                node_text.to_string()
            } else {
                node_text.to_lowercase()
            };
            if !cmp_text.contains(&capture_filter_cmp) {
                continue;
            }
        }

        let line = node.start_position().row;
        let col = node.start_position().column;
        let line_text = src.lines().nth(line).unwrap_or("");
        let capture_name = ts_query.capture_names()[capture.index as usize].to_string();

        out.push(SearchResult {
            file_path: file_path.to_string(),
            line,
            col,
            end_line: node.end_position().row,
            end_col: node.end_position().column,
            snippet: make_snippet(line_text, col),
            node_kind: node.kind().to_string(),
            capture_name,
        });
    }

    out
}

/// Maximum results collected per file for any single search (ts-query or grep).
///
/// Learned from ripgrep: unbounded result accumulation in parallel workers can
/// cause runaway memory use on pathological inputs. A per-file cap keeps both
/// memory and result-list rendering bounded.
const MAX_RESULTS_PER_FILE: usize = 500;

// ── Helpers ──────────────────────────────────────────────────────────────

/// True if the captured name node belongs to a call_expression that is itself
/// inside the argument_list of another call.
///
/// Example: `foo(bar())` → `bar` is nested, `foo` is not.
/// Example: `foo(x * bar())` → `bar` is nested (inside binary_expression inside argument_list).
fn call_is_nested(captured_node: tree_sitter::Node) -> bool {
    // Step 1: walk up to find the call_expression that owns this capture.
    let mut n = captured_node;
    let call_expr = loop {
        match n.parent() {
            Some(p) if p.kind() == "call_expression" => break p,
            Some(p) => n = p,
            None => return false,
        }
    };

    // Step 2: keep climbing through expression nodes. If we reach argument_list
    // before any statement/declaration boundary, the call is nested in another
    // call's argument list.
    let mut ancestor = call_expr;
    while let Some(parent) = ancestor.parent() {
        match parent.kind() {
            "argument_list" => return true,
            // Any *_expression node: keep climbing (handles binary_expression,
            // unary_expression, cast_expression, parenthesized_expression, …)
            k if k.ends_with("_expression") => ancestor = parent,
            // Anything else (statement, declaration, block, …): top-level call.
            _ => return false,
        }
    }
    false
}

// ── Plain-text grep ───────────────────────────────────────────────────────

fn grep(file_path: &str, src: &str, text: &str, use_regex: bool, case_sensitive: bool) -> Vec<SearchResult> {
    if text.is_empty() {
        return vec![];
    }
    if use_regex {
        // Compile once, search every line. Honour the case toggle via RegexBuilder.
        let re = match regex::RegexBuilder::new(text)
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                log::warn!("grep regex compile error {:?}: {}", text, e);
                return vec![];
            }
        };
        return src.lines()
            .enumerate()
            .filter_map(|(line, content)| {
                let m = re.find(content)?;
                let col = m.start();
                Some(SearchResult {
                    file_path: file_path.to_string(),
                    line,
                    col,
                    end_line: line,
                    end_col: m.end(),
                    snippet: make_snippet(content, col),
                    node_kind: "text".into(),
                    capture_name: String::new(),
                })
            })
            .take(MAX_RESULTS_PER_FILE)
            .collect();
    }
    // Plain substring match — case-sensitive or insensitive.
    if case_sensitive {
        src.lines()
            .enumerate()
            .filter_map(|(line, content)| {
                let col = content.find(text)?;
                Some(SearchResult {
                    file_path: file_path.to_string(),
                    line,
                    col,
                    end_line: line,
                    end_col: col + text.len(),
                    snippet: make_snippet(content, col),
                    node_kind: "text".into(),
                    capture_name: String::new(),
                })
            })
            .take(MAX_RESULTS_PER_FILE)
            .collect()
    } else {
        let needle = text.to_lowercase();
        src.lines()
            .enumerate()
            .filter_map(|(line, content)| {
                let col = content.to_lowercase().find(&needle)?;
                Some(SearchResult {
                    file_path: file_path.to_string(),
                    line,
                    col,
                    end_line: line,
                    end_col: col + needle.len(),
                    snippet: make_snippet(content, col),
                    node_kind: "text".into(),
                    capture_name: String::new(),
                })
            })
            .take(MAX_RESULTS_PER_FILE)
            .collect()
    }
}

fn make_snippet(line: &str, col: usize) -> String {
    // col is a byte offset from tree-sitter; convert to char count safely.
    let col_char = line[..col.min(line.len())].chars().count();
    let start_char = col_char.saturating_sub(4);
    line.chars()
        .skip(start_char)
        .take(48)
        .collect::<String>()
        .trim_end()
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_query ──────────────────────────────────────────────────────

    #[test]
    fn parse_empty_is_grep() {
        let q = parse_query("");
        assert!(q.is_empty());
        assert!(q.grep_mode);
    }

    #[test]
    fn parse_plain_text_is_grep() {
        let q = parse_query("update");
        assert!(q.grep_mode);
        assert_eq!(q.raw, "update");
        assert!(q.ts_query_src.is_empty());
    }

    #[test]
    fn parse_shorthand_fn() {
        let q = parse_query("fn:update");
        assert!(!q.grep_mode);
        assert_eq!(q.capture_filter, "update");
        assert!(q.ts_query_src.contains("function_definition"));
    }

    #[test]
    fn parse_shorthand_call_no_filter() {
        let q = parse_query("call:");
        assert!(!q.grep_mode);
        assert!(q.capture_filter.is_empty());
        assert!(q.ts_query_src.contains("call_expression"));
    }

    #[test]
    fn parse_raw_ts_query() {
        let q = parse_query("(identifier) @id");
        assert!(!q.grep_mode);
        assert_eq!(q.ts_query_src, "(identifier) @id");
        assert!(q.capture_filter.is_empty());
    }

    #[test]
    fn parse_raw_alternation() {
        let q = parse_query("[(identifier) (type_identifier)] @name");
        assert!(!q.grep_mode);
        assert!(q.ts_query_src.starts_with('['));
    }

    // ── grep search ──────────────────────────────────────────────────────

    const SAMPLE: &str = r#"
void AudioManager::update(float dt) {
    activeSources_.erase(
        std::remove_if(activeSources_.begin(), activeSources_.end(),
            [](const AudioSource& s) { return !s.isPlaying(); }),
        activeSources_.end());
}
"#;

    #[test]
    fn grep_finds_matching_line() {
        let q = parse_query("update");
        let results = search_src("fake.cpp", SAMPLE, &q);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.snippet.contains("update")));
    }

    #[test]
    fn grep_case_insensitive() {
        let q = parse_query("AUDIOSOURCE");
        let results = search_src("fake.cpp", SAMPLE, &q);
        assert!(!results.is_empty());
    }

    #[test]
    fn grep_no_match_returns_empty() {
        let q = parse_query("zzznomatch");
        let results = search_src("fake.cpp", SAMPLE, &q);
        assert!(results.is_empty());
    }

    // ── ts-query: shorthands ─────────────────────────────────────────────

    const CPP_FUNCS: &str = r#"
#include <iostream>

int add(int a, int b) { return a + b; }

void greet(const char* name) {
    std::cout << name << "\n";
}

int main() {
    int result = add(1, 2);
    greet("hello");
    return 0;
}
"#;

    #[test]
    fn call_shorthand_finds_calls() {
        let q = parse_query("call:");
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        let names: Vec<&str> = results.iter().map(|r| r.snippet.as_str()).collect();
        assert!(results.iter().any(|r| r.snippet.contains("add")), "expected add() call, got: {:?}", names);
        assert!(results.iter().any(|r| r.snippet.contains("greet")), "expected greet() call, got: {:?}", names);
    }

    #[test]
    fn call_shorthand_with_filter() {
        let q = parse_query("call:greet");
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.snippet.contains("greet")));
        // Should not include "add"
        assert!(!results.iter().any(|r| r.snippet.contains("add") && !r.snippet.contains("greet")));
    }

    #[test]
    fn fn_shorthand_finds_definitions() {
        let q = parse_query("fn:");
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        assert!(results.iter().any(|r| r.snippet.contains("add")));
        assert!(results.iter().any(|r| r.snippet.contains("greet")));
        assert!(results.iter().any(|r| r.snippet.contains("main")));
    }

    #[test]
    fn fn_shorthand_with_filter() {
        let q = parse_query("fn:add");
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        assert_eq!(results.len(), 1);
        assert!(results[0].snippet.contains("add"));
    }

    #[test]
    fn param_shorthand_finds_parameters() {
        let q = parse_query("param:");
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        assert!(results.iter().any(|r| r.snippet.contains('a') || r.snippet.contains('b') || r.snippet.contains("name")));
    }

    // ── ts-query: raw queries ────────────────────────────────────────────

    #[test]
    fn raw_query_identifier() {
        let q = parse_query("(identifier) @id");
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.node_kind == "identifier"));
    }

    #[test]
    fn raw_query_invalid_node_type_returns_empty() {
        // "not_a_real_node" is not a valid tree-sitter-cpp node type
        let q = parse_query("(not_a_real_node) @x");
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        assert!(results.is_empty());
    }

    #[test]
    fn raw_query_no_capture_returns_empty() {
        // Valid node but no @capture — nothing to collect
        let q = parse_query("(function_definition)");
        // parse_query sees no leading '(' in... wait it does.
        // ts_query_src = "(function_definition)", no capture → 0 results
        let results = search_src("fake.cpp", CPP_FUNCS, &q);
        assert!(results.is_empty());
    }

    // ── deduplication ────────────────────────────────────────────────────

    #[test]
    fn no_duplicate_results_same_position() {
        // call: has two patterns; a simple identifier call should not appear twice
        let src = "void foo() { bar(); }";
        let q = parse_query("call:");
        let results = search_src("fake.cpp", src, &q);
        let bar_count = results.iter().filter(|r| r.snippet.contains("bar")).count();
        assert_eq!(bar_count, 1, "bar() should appear exactly once, got {}", bar_count);
    }

    // ── nested call filtering ────────────────────────────────────────────

    #[test]
    fn call_filter_excludes_nested_calls_in_args() {
        // applyForce(gravity_ * body->mass()) — mass() is nested, should not appear
        // when filtering by name containing 'ma'
        let src = r#"
void step() {
    body->applyForce(gravity_ * body->mass());
}
"#;
        let q = parse_query("call:ma");
        let results = search_src("fake.cpp", src, &q);
        // mass() is nested inside applyForce's argument list → should be excluded
        assert!(
            results.is_empty(),
            "expected no results: mass() is nested, got {:?}",
            results.iter().map(|r| &r.snippet).collect::<Vec<_>>()
        );
    }

    #[test]
    fn call_filter_includes_outer_call_matching_name() {
        let src = r#"
void step() {
    body->applyForce(gravity_ * body->mass());
    body->mass();
}
"#;
        let q = parse_query("call:mass");
        let results = search_src("fake.cpp", src, &q);
        // Only the standalone `body->mass()` on its own statement should match
        assert_eq!(results.len(), 1, "expected exactly 1 top-level mass() call, got {:?}", results);
        assert!(results[0].snippet.contains("mass"));
    }

    #[test]
    fn call_filter_does_not_affect_raw_queries() {
        // Raw ts-query has filter_nested_calls = false, so nested calls are included
        let src = "void step() { applyForce(mass()); }";
        let q = parse_query("(call_expression function: (identifier) @match)");
        let results = search_src("fake.cpp", src, &q);
        // Both applyForce and mass should appear
        assert!(results.iter().any(|r| r.snippet.contains("applyForce")));
        assert!(results.iter().any(|r| r.snippet.contains("mass")));
    }

    // ── regression: call:main on a project where main() is never called ──
    //
    // Previously this caused a panic:
    //   1. An earlier search loaded AST nodes and set search_ast_scroll to ~341.
    //   2. call:main returned 0 results so search_ast_nodes was cleared.
    //   3. The renderer did nodes[341..0] → panic (slice start > end).
    //
    // Fix: (a) reset all scroll fields when results are empty in run_search,
    //      (b) clamp scroll to nodes.len() in the render before slicing.

    #[test]
    fn call_main_returns_empty_when_main_is_never_called() {
        // main() is defined here but never called — call: should find 0 results.
        let src = r#"
void helper() { }

int main() {
    helper();
    return 0;
}
"#;
        let q = parse_query("call:main");
        let results = search_src("fake.cpp", src, &q);
        assert!(
            results.is_empty(),
            "call:main should return 0 results when main() is only defined, not called; got {:?}",
            results.iter().map(|r| (&r.snippet, r.line)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn call_main_finds_call_when_main_is_called() {
        // Unusual but valid — confirm the filter works when main IS called.
        let src = r#"
int main();

void bootstrap() {
    main();
}
"#;
        let q = parse_query("call:main");
        let results = search_src("fake.cpp", src, &q);
        assert_eq!(results.len(), 1, "expected exactly one call to main(), got {:?}", results);
        assert!(results[0].snippet.contains("main"));
    }

    // ── FileFilter / glob ────────────────────────────────────────────────

    #[test]
    fn filter_empty_matches_everything() {
        let f = FileFilter::parse("");
        assert!(f.is_empty());
        assert!(f.matches("src/foo.cpp"));
        assert!(f.matches("tests/bar.cpp"));
    }

    #[test]
    fn filter_star_star_matches_any_subdir() {
        let f = FileFilter::parse("src/**");
        assert!(f.matches("src/audio/Player.cpp"));
        assert!(f.matches("src/main.cpp"));
        assert!(!f.matches("tests/audio/Player.cpp"));
        assert!(!f.matches("vendor/lib.cpp"));
    }

    #[test]
    fn filter_extension_glob_no_slash() {
        let f = FileFilter::parse("*.cpp");
        assert!(f.matches("src/foo.cpp"));
        assert!(f.matches("main.cpp"));
        assert!(!f.matches("src/foo.h"));
    }

    #[test]
    fn filter_double_star_in_middle() {
        let f = FileFilter::parse("**/test/**");
        assert!(f.matches("src/test/foo.cpp"));
        assert!(f.matches("test/bar.cpp"));
        assert!(!f.matches("src/main.cpp"));
    }

    #[test]
    fn filter_comma_separates_patterns() {
        let f = FileFilter::parse("tests/**,vendor/**");
        assert!(f.matches("tests/foo.cpp"));
        assert!(f.matches("vendor/bar.cpp"));
        assert!(!f.matches("src/main.cpp"));
    }

    #[test]
    fn filter_semicolon_separates_patterns() {
        let f = FileFilter::parse("tests/**;vendor/**");
        assert!(f.matches("tests/foo.cpp"));
        assert!(!f.matches("src/main.cpp"));
    }

    #[test]
    fn filter_question_mark() {
        let f = FileFilter::parse("src/?.cpp");
        assert!(f.matches("src/a.cpp"));
        assert!(!f.matches("src/ab.cpp"));
        assert!(!f.matches("src/a/b.cpp"));
    }

    #[test]
    fn filter_exact_filename() {
        let f = FileFilter::parse("main.cpp");
        assert!(f.matches("src/main.cpp"));
        assert!(f.matches("main.cpp"));
        assert!(!f.matches("src/notmain.cpp"));
    }

    #[test]
    fn filter_header_extension() {
        let f = FileFilter::parse("*.h,*.hpp");
        assert!(f.matches("include/Audio.h"));
        assert!(f.matches("src/Player.hpp"));
        assert!(!f.matches("src/Player.cpp"));
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────


