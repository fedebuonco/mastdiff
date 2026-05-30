//! Project discovery: finds C++ translation units for the project browser.
//!
//! [`load`] prefers a `compile_commands.json` database (checked in several
//! common build-output directories) and falls back to a recursive directory
//! walk collecting `.cpp`, `.cc`, `.cxx`, `.C`, `.h`, `.hpp`, `.hxx`, and `.H` files.
//!
//! Header association uses **static analysis**: each source file is parsed
//! with tree-sitter to extract its `#include "..."` directives (local includes
//! only; `<system>` headers are ignored).  The raw include strings are resolved
//! against the project's known header paths to find the actual files.
//!
//! A **reverse index** (`included_by`) is also built on every header, listing
//! the source files that directly `#include` it.
//!
//! CMake targets are parsed from any `CMakeLists.txt` files in the tree and
//! returned alongside the file list in [`ProjectData`].

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tree_sitter::{Parser, Query, QueryCursor};
use serde::Deserialize;
use walkdir::WalkDir;

// ── Public types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TranslationUnit {
    pub file_path: String,
    pub file_size: u64,
    /// True for header files (.h/.hpp/.hxx/.H).
    pub is_header: bool,
    /// Headers directly `#include`d by this source file (sources only).
    /// Populated by static analysis of `#include "..."` directives.
    pub associated_headers: Vec<String>,
    /// Source files that directly `#include` this header (headers only).
    /// Empty for source files.
    pub included_by: Vec<String>,
}

impl TranslationUnit {
    pub fn short_name(&self) -> &str {
        Path::new(&self.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.file_path)
    }

    pub fn size_label(&self) -> String {
        if self.file_size < 1024 {
            format!("{}B", self.file_size)
        } else if self.file_size < 1024 * 1024 {
            format!("{:.1}KB", self.file_size as f64 / 1024.0)
        } else {
            format!("{:.1}MB", self.file_size as f64 / (1024.0 * 1024.0))
        }
    }
}

#[derive(Debug, Clone)]
pub struct CmakeTarget {
    pub name: String,
    /// "executable" or "library"
    pub kind: String,
    /// Source files explicitly listed in the CMake call (may be incomplete
    /// when variables are used instead of literal paths).
    pub sources: Vec<String>,
}

/// Everything discovered about a project directory.
pub struct ProjectData {
    /// All C++ files (sources + headers), sorted by path.
    pub files: Vec<TranslationUnit>,
    /// CMake targets found in CMakeLists.txt files (empty if none found).
    pub cmake_targets: Vec<CmakeTarget>,
}

// ── Entry point ───────────────────────────────────────────────────────────

pub fn load(dir: &Path) -> Result<ProjectData> {
    let mut files = if let Some(cc) = find_compile_commands(dir) {
        log::info!("project loader: compile_commands.json at {:?}", cc);
        let tus = load_from_compile_commands(&cc, dir)?;
        log::info!("project loader: {} translation units loaded", tus.len());
        tus
    } else {
        log::info!("project loader: directory walk of {:?}", dir);
        let tus = load_from_walk(dir)?;
        log::info!("project loader: {} files found", tus.len());
        tus
    };

    // ── Static include analysis ───────────────────────────────────────────
    // Build lookup structures for resolving raw include strings to file paths.
    let header_paths: HashSet<String> = files
        .iter()
        .filter(|tu| tu.is_header)
        .map(|tu| tu.file_path.clone())
        .collect();

    // filename → [full_path] for headers (e.g. "audio.h" → ["/…/include/audio.h"])
    let mut header_by_name: HashMap<String, Vec<String>> = HashMap::new();
    for p in &header_paths {
        let name = Path::new(p)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        header_by_name.entry(name).or_default().push(p.clone());
    }

    // path_suffix → full_path for partial-path matching
    // e.g. "spdlog/spdlog.h" can match from inside include/
    let mut header_by_suffix: HashMap<String, String> = HashMap::new();
    for p in &header_paths {
        // Index every trailing sub-path of depth 1..4
        let components: Vec<&str> = p.split('/').collect();
        for depth in 1..=components.len().min(4) {
            let suffix = components[components.len() - depth..].join("/");
            header_by_suffix.entry(suffix).or_insert_with(|| p.clone());
        }
    }

    // Collect all unique source directory paths as additional include-path hints.
    let include_dirs: Vec<String> = files
        .iter()
        .filter_map(|tu| Path::new(&tu.file_path).parent()?.to_str().map(str::to_owned))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // Parse #include directives from every source file using tree-sitter,
    // resolve to actual paths in the project, and record associations both
    // ways: source → headers (associated_headers) and header → sources (included_by).
    let mut included_by: HashMap<String, Vec<String>> = HashMap::new(); // header → sources

    // We need to iterate by index to later mutate `files`.
    let source_indices: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, tu)| !tu.is_header)
        .map(|(i, _)| i)
        .collect();

    for &si in &source_indices {
        let src_path = files[si].file_path.clone();
        let raw_includes = extract_local_includes(&src_path);
        let inc_dirs: Vec<&str> = include_dirs.iter().map(|s| s.as_str()).collect();
        let resolved = resolve_includes(
            &src_path,
            &raw_includes,
            &header_by_name,
            &header_by_suffix,
            &inc_dirs,
        );
        for h in &resolved {
            included_by.entry(h.clone()).or_default().push(src_path.clone());
        }
        files[si].associated_headers = resolved;
        log::debug!(
            "includes: {} → {} headers",
            Path::new(&src_path).file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            files[si].associated_headers.len()
        );
    }

    // Populate included_by on header entries.
    for tu in &mut files {
        if !tu.is_header { continue; }
        if let Some(sources) = included_by.get(&tu.file_path) {
            tu.included_by = sources.clone();
        }
    }

    for tu in &files {
        log::trace!(
            "  {}{} ({}) hdrs={} iby={}",
            if tu.is_header { "[H] " } else { "    " },
            tu.file_path, tu.size_label(),
            tu.associated_headers.len(),
            tu.included_by.len()
        );
    }

    let cmake_targets = find_cmake_targets(dir);
    log::info!("project loader: {} cmake targets", cmake_targets.len());

    Ok(ProjectData { files, cmake_targets })
}

// ── File discovery ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CompileEntry {
    file: String,
}

fn find_compile_commands(dir: &Path) -> Option<PathBuf> {
    let candidates = [
        dir.join("compile_commands.json"),
        dir.join("build").join("compile_commands.json"),
        dir.join("cmake-build-debug").join("compile_commands.json"),
        dir.join("cmake-build-release").join("compile_commands.json"),
        dir.join("out").join("compile_commands.json"),
        dir.join("_build").join("compile_commands.json"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn load_from_compile_commands(path: &Path, project_root: &Path) -> Result<Vec<TranslationUnit>> {
    let raw = fs::read_to_string(path)?;
    let entries: Vec<CompileEntry> = serde_json::from_str(&raw)?;

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut tus: Vec<TranslationUnit> = entries
        .into_iter()
        .filter(|e| is_source(&e.file))
        .filter_map(|e| {
            if !Path::new(&e.file).exists() { return None; }
            if !seen.insert(e.file.clone()) { return None; }
            let file_size = fs::metadata(&e.file).map(|m| m.len()).unwrap_or(0);
            Some(TranslationUnit {
                file_path: e.file,
                file_size,
                is_header: false,
                associated_headers: vec![],
                included_by: vec![],
            })
        })
        .collect();

    // Add headers from a directory walk (compile_commands doesn't list them).
    let headers = walk_headers(project_root);
    tus.extend(headers);

    tus.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    tus.dedup_by(|a, b| a.file_path == b.file_path);
    Ok(tus)
}

fn load_from_walk(dir: &Path) -> Result<Vec<TranslationUnit>> {
    let mut tus: Vec<TranslationUnit> = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_cpp_source(&e.path().to_string_lossy()))
        .map(|e| {
            let path = e.path().to_string_lossy().into_owned();
            let file_size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let hdr = is_header(&path);
            TranslationUnit {
                file_path: path,
                file_size,
                is_header: hdr,
                associated_headers: vec![],
                included_by: vec![],
            }
        })
        .collect();

    tus.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    Ok(tus)
}

fn walk_headers(dir: &Path) -> Vec<TranslationUnit> {
    WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_header(&e.path().to_string_lossy()))
        .map(|e| {
            let file_size = e.metadata().map(|m| m.len()).unwrap_or(0);
            TranslationUnit {
                file_path: e.path().to_string_lossy().into_owned(),
                file_size,
                is_header: true,
                associated_headers: vec![],
                included_by: vec![],
            }
        })
        .collect()
}

// ── Include analysis ──────────────────────────────────────────────────────

/// Parse `#include "..."` directives from a source file using tree-sitter.
/// Only quoted (local) includes are returned; `<system>` headers are skipped.
fn extract_local_includes(path: &str) -> Vec<String> {
    let Ok(src) = fs::read_to_string(path) else { return vec![] };
    let mut parser = Parser::new();
    let lang = tree_sitter_cpp::language();
    if parser.set_language(&lang).is_err() { return vec![] }
    let Some(tree) = parser.parse(&src, None) else { return vec![] };

    let query_src = r#"(preproc_include path: (string_literal) @path)"#;
    let Ok(query) = Query::new(&lang, query_src) else { return vec![] };
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();

    for (m, _) in cursor.captures(&query, tree.root_node(), src.as_bytes()) {
        let text = m.captures[0].node.utf8_text(src.as_bytes()).unwrap_or("");
        // Strip surrounding quotes: "foo.h" → foo.h
        let inner = text.trim_matches('"');
        if !inner.is_empty() {
            out.push(inner.to_string());
        }
    }
    out
}

/// Resolve raw include strings (e.g. `"audio.h"`, `"spdlog/spdlog.h"`) to
/// actual absolute file paths present in the project.
///
/// Resolution order (first match wins):
/// 1. Relative to the source file's own directory.
/// 2. Suffix match against all known header paths (handles `subdir/foo.h`).
/// 3. Bare-filename match against all headers with that name.
fn resolve_includes(
    src_path: &str,
    raw: &[String],
    by_name: &HashMap<String, Vec<String>>,
    by_suffix: &HashMap<String, String>,
    _include_dirs: &[&str],
) -> Vec<String> {
    let src_dir = Path::new(src_path).parent().unwrap_or(Path::new(""));
    let mut resolved: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for inc in raw {
        // 1. Relative to source directory
        let rel = src_dir.join(inc);
        if let Ok(canon) = rel.canonicalize() {
            let s = canon.to_string_lossy().into_owned();
            if seen.insert(s.clone()) { resolved.push(s); }
            continue;
        }

        // 2. Suffix match (handles include paths like "spdlog/fmt/fmt.h")
        if let Some(full) = by_suffix.get(inc.as_str()) {
            if seen.insert(full.clone()) { resolved.push(full.clone()); }
            continue;
        }

        // 3. Bare filename fallback
        let fname = Path::new(inc).file_name().and_then(|n| n.to_str()).unwrap_or(inc);
        if let Some(candidates) = by_name.get(fname) {
            if let Some(best) = pick_best(inc, candidates) {
                if seen.insert(best.clone()) { resolved.push(best); }
            }
        }
    }

    resolved
}

/// Among multiple headers with the same filename, prefer the one whose path
/// shares the longest common suffix with the raw include string.
fn pick_best(raw_include: &str, candidates: &[String]) -> Option<String> {
    candidates.iter()
        .max_by_key(|c| {
            // Count matching path components from the right
            let raw_parts: Vec<&str> = raw_include.split('/').collect();
            let cand_parts: Vec<&str> = c.split('/').collect();
            raw_parts.iter().rev()
                .zip(cand_parts.iter().rev())
                .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
                .count()
        })
        .cloned()
}

// ── CMake target discovery ────────────────────────────────────────────────

fn find_cmake_targets(dir: &Path) -> Vec<CmakeTarget> {
    let mut targets: Vec<CmakeTarget> = Vec::new();
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == "CMakeLists.txt")
    {
        if let Ok(src) = fs::read_to_string(entry.path()) {
            parse_cmake(&src, &mut targets, &mut seen_names);
        }
    }

    targets.sort_by(|a, b| a.name.cmp(&b.name));
    targets
}

/// Very lightweight CMake parser — handles multi-line calls by joining
/// continuation tokens.  Only `add_executable` and `add_library` are extracted.
fn parse_cmake(
    src: &str,
    targets: &mut Vec<CmakeTarget>,
    seen: &mut std::collections::HashSet<String>,
) {
    // Collapse the whole file into one "token stream" (newlines → spaces,
    // strip line comments).
    let mut flat = String::with_capacity(src.len());
    for line in src.lines() {
        let stripped = match line.find('#') {
            Some(i) => &line[..i],
            None    => line,
        };
        flat.push_str(stripped);
        flat.push(' ');
    }

    // Simple extraction: find balanced parentheses after the keyword.
    for keyword in &[("add_executable", "executable"), ("add_library", "library")] {
        let (kw, kind) = keyword;
        let mut search = flat.as_str();
        while let Some(pos) = search.to_lowercase().find(kw) {
            search = &search[pos + kw.len()..];
            // Find opening paren
            let Some(open) = search.find('(') else { break };
            // Only skip whitespace between keyword and '('
            if search[..open].trim().is_empty() {
                search = &search[open + 1..];
                // Find matching closing paren (no nesting in our simple case)
                let Some(close) = search.find(')') else { break };
                let args = &search[..close];
                search = &search[close + 1..];

                let tokens: Vec<&str> = args.split_whitespace().collect();
                if tokens.is_empty() { continue; }

                let name = tokens[0].to_string();
                // Skip IMPORTED, ALIAS, INTERFACE-only pseudo-targets
                if name.starts_with('$') || seen.contains(&name) { continue; }
                seen.insert(name.clone());

                // Skip the optional STATIC/SHARED/MODULE/INTERFACE keyword
                let src_start = if tokens.len() > 1
                    && matches!(tokens[1], "STATIC"|"SHARED"|"MODULE"|"INTERFACE"|"OBJECT")
                {
                    2
                } else {
                    1
                };

                let sources: Vec<String> = tokens[src_start..]
                    .iter()
                    .filter(|t| !t.starts_with('$') && (t.contains('.') || t.contains('/')))
                    .map(|t| t.to_string())
                    .collect();

                targets.push(CmakeTarget { name, kind: kind.to_string(), sources });
            } else {
                break;
            }
        }
    }
}

// ── Extension helpers ─────────────────────────────────────────────────────

pub fn is_cpp_source(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("cpp" | "cc" | "cxx" | "C" | "h" | "hpp" | "hxx" | "H")
    )
}

pub fn is_source(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("cpp" | "cc" | "cxx" | "C")
    )
}

pub fn is_header(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("h" | "hpp" | "hxx" | "H")
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tu(path: &str, size: u64) -> TranslationUnit {
        TranslationUnit {
            file_path: path.to_string(),
            file_size: size,
            is_header: is_header(path),
            associated_headers: vec![],
            included_by: vec![],
        }
    }

    // ── is_cpp_source ─────────────────────────────────────────────────────

    #[test]
    fn cpp_extensions_accepted() {
        assert!(is_cpp_source("foo.cpp"));
        assert!(is_cpp_source("foo.cc"));
        assert!(is_cpp_source("foo.cxx"));
        assert!(is_cpp_source("foo.C"));
        assert!(is_cpp_source("foo.h"));
        assert!(is_cpp_source("foo.hpp"));
        assert!(is_cpp_source("foo.hxx"));
        assert!(is_cpp_source("foo.H"));
    }

    #[test]
    fn non_cpp_extensions_rejected() {
        assert!(!is_cpp_source("foo.c"));
        assert!(!is_cpp_source("foo.rs"));
        assert!(!is_cpp_source("foo.py"));
        assert!(!is_cpp_source("foo"));
    }

    #[test]
    fn extension_check_is_case_sensitive() {
        assert!(!is_cpp_source("foo.c"));
        assert!(is_cpp_source("foo.C"));
    }

    #[test]
    fn is_header_recognises_headers() {
        assert!(is_header("foo.h"));
        assert!(is_header("foo.hpp"));
        assert!(is_header("foo.hxx"));
        assert!(is_header("foo.H"));
        assert!(!is_header("foo.cpp"));
        assert!(!is_header("foo.c"));
    }

    #[test]
    fn is_source_excludes_headers() {
        assert!(is_source("foo.cpp"));
        assert!(!is_source("foo.h"));
        assert!(!is_source("foo.hpp"));
    }

    // ── TranslationUnit helpers ───────────────────────────────────────────

    #[test]
    fn short_name_extracts_filename() {
        assert_eq!(tu("/a/b/foo.cpp", 0).short_name(), "foo.cpp");
        assert_eq!(tu("bar.cpp", 0).short_name(), "bar.cpp");
    }

    #[test]
    fn size_label_bytes()     { assert_eq!(tu("f.cpp", 500).size_label(), "500B"); }
    #[test]
    fn size_label_kilobytes() { assert_eq!(tu("f.cpp", 2048).size_label(), "2.0KB"); }
    #[test]
    fn size_label_megabytes() { assert_eq!(tu("f.cpp", 2 * 1024 * 1024).size_label(), "2.0MB"); }
    #[test]
    fn size_label_zero()      { assert_eq!(tu("f.cpp", 0).size_label(), "0B"); }

    // ── CMake parsing ─────────────────────────────────────────────────────

    #[test]
    fn cmake_parses_executable() {
        let cmake = "add_executable(MyApp main.cpp foo.cpp bar.cpp)\n";
        let mut targets = vec![];
        let mut seen = std::collections::HashSet::new();
        parse_cmake(cmake, &mut targets, &mut seen);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "MyApp");
        assert_eq!(targets[0].kind, "executable");
        assert!(targets[0].sources.contains(&"main.cpp".to_string()));
    }

    #[test]
    fn cmake_parses_library() {
        let cmake = "add_library(MyLib STATIC alpha.cpp beta.cpp)\n";
        let mut targets = vec![];
        let mut seen = std::collections::HashSet::new();
        parse_cmake(cmake, &mut targets, &mut seen);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "MyLib");
        assert_eq!(targets[0].kind, "library");
    }

    #[test]
    fn cmake_parses_multiline() {
        let cmake = "add_executable(MyApp\n    main.cpp\n    utils.cpp\n)\n";
        let mut targets = vec![];
        let mut seen = std::collections::HashSet::new();
        parse_cmake(cmake, &mut targets, &mut seen);
        assert_eq!(targets.len(), 1);
        assert!(targets[0].sources.contains(&"main.cpp".to_string()));
    }

    #[test]
    fn cmake_skips_variable_targets() {
        let cmake = "add_executable(${MY_TARGET} main.cpp)\n";
        let mut targets = vec![];
        let mut seen = std::collections::HashSet::new();
        parse_cmake(cmake, &mut targets, &mut seen);
        assert_eq!(targets.len(), 0, "variable-named targets should be skipped");
    }

    #[test]
    fn cmake_deduplicates_targets() {
        let cmake = "add_executable(App main.cpp)\nadd_executable(App other.cpp)\n";
        let mut targets = vec![];
        let mut seen = std::collections::HashSet::new();
        parse_cmake(cmake, &mut targets, &mut seen);
        assert_eq!(targets.len(), 1);
    }
}
