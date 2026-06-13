//! Persistent, content-addressed symbol-index cache — the ccache/sccache idea
//! applied to tree-sitter search.
//!
//! tree-sitter `Tree`s aren't serializable, so we can't persist the parse
//! itself. Instead we persist its **derived product**: for each file we parse
//! once with a *combined* query (the union of every shorthand bucket — `fn:`,
//! `call:`, `class:`, …) and store the resulting captures on disk, keyed by a
//! hash of the file's bytes. A later shorthand search then loads that index
//! and filters the relevant bucket's captures — **no parse, no query walk** —
//! and the entry survives across process restarts (where the in-memory
//! `AstCache` gives nothing).
//!
//! Raw S-expression queries and plain grep can't be served from a
//! pre-extracted index; callers fall back to the live parse path for those.
//!
//! # On-disk layout
//! ```text
//! $XDG_CACHE_HOME/mastdiff/v{SCHEMA}/<ab>/<hash>.json
//! ```
//! Content-addressed, so identical files (even across projects) share one
//! entry. Sharded by the first two hex chars to keep directories small.
//! Writes are temp-file + atomic-rename so concurrent CLI/daemon processes
//! never read a torn file.
//!
//! # Bounded size — it cannot run away
//! Content-addressed entries are never "stale", just *abandoned* when a file
//! changes (the new bytes hash elsewhere). Left alone the directory would grow
//! forever, so a hard byte cap is enforced: when a write pushes the running
//! total over the cap, [`SymCache::sweep`] deletes the oldest entries down to a
//! low-water mark (~90% of the cap). Set the cap to 0 to disable the cache
//! entirely (nothing is written or read).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tree_sitter::{Parser, Query, QueryCursor};

use crate::search::{
    self, bucket_filters_nested, bucket_of_query, call_is_nested, make_snippet, SearchQuery,
    SearchResult, INDEX_BUCKETS, MAX_RESULTS_PER_FILE,
};

/// Bump when the artifact format or any bucket's query/id changes — old
/// `v{N}` directories are then ignored and garbage-collected.
const SCHEMA: u32 = 1;

/// Files larger than this are searched live and never cached (bounds the size
/// of any single artifact).
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Low-water target after a sweep, as a fraction of the cap. Hysteresis so we
/// don't sweep on every write once near the cap.
const LOW_WATER: f64 = 0.9;

// ── Artifact ────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct CachedCapture {
    /// Shorthand bucket id (see `INDEX_BUCKETS`).
    pub bucket: u8,
    /// Actual tree-sitter node kind, e.g. "identifier".
    pub node_kind: String,
    /// Captured node's text — what the `fn:foo` text filter matches against.
    pub text: String,
    /// Pre-rendered source snippet for the result row.
    pub snippet: String,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// For the `call:` bucket: is this call nested inside another call's args?
    pub call_nested: bool,
}

#[derive(Serialize, Deserialize)]
pub struct FileIndex {
    pub schema: u32,
    pub captures: Vec<CachedCapture>,
}

// ── Content key ──────────────────────────────────────────────────────────────

/// FNV-1a over the file bytes, mixed with the length. Good enough for a local
/// content-addressed cache; a hash miss just costs a re-parse, never a wrong
/// result (the served data is whatever bytes produced this key).
pub fn content_key(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x00000100000001b3);
    }
    h ^= bytes.len() as u64;
    format!("{:016x}", h)
}

// ── Combined index query ─────────────────────────────────────────────────────

/// Build the union query that captures every bucket in a single walk. Each
/// bucket's `@match` capture is renamed to `@b<id>` so the capture name maps
/// back to its bucket.
///
/// A bucket whose query references node types absent from the bundled grammar
/// (e.g. `static_cast_expression` in some tree-sitter-cpp builds) won't compile
/// — including it would poison the whole union, so such buckets are skipped.
/// That's consistent with the live path, where the same query also fails to
/// compile and the shorthand returns nothing.
fn combined_query_src() -> String {
    let lang = tree_sitter_cpp::language();
    let mut s = String::new();
    for (id, _prefix, query_src, _nested) in INDEX_BUCKETS {
        let renamed = query_src.replace("@match", &format!("@b{id}"));
        if Query::new(&lang, &renamed).is_err() {
            log::debug!("symcache: bucket {id} query unsupported by grammar, skipping");
            continue;
        }
        s.push_str(&renamed);
        s.push('\n');
    }
    s
}

fn compile_combined_query() -> Option<Query> {
    let lang = tree_sitter_cpp::language();
    Query::new(&lang, &combined_query_src()).ok()
}

/// Parse `src` and extract every bucket's captures into a `FileIndex`.
fn build_index(src: &str, query: &Query) -> Option<FileIndex> {
    let lang = tree_sitter_cpp::language();
    let mut parser = Parser::new();
    parser.set_language(&lang).ok()?;
    let tree = parser.parse(src, None)?;

    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut captures: Vec<CachedCapture> = Vec::new();
    // Dedup by (bucket, start byte), mirroring the live path's per-query dedup.
    let mut seen: std::collections::HashSet<(u8, usize)> = std::collections::HashSet::new();

    for (m, capture_idx) in cursor.captures(query, tree.root_node(), src.as_bytes()) {
        let capture = m.captures[capture_idx];
        let name = names[capture.index as usize];
        let bucket: u8 = match name.strip_prefix('b').and_then(|n| n.parse().ok()) {
            Some(b) => b,
            None => continue,
        };
        let node = capture.node;
        let start = node.start_byte();
        if !seen.insert((bucket, start)) {
            continue;
        }
        let text = src.get(start..node.end_byte()).unwrap_or("").to_string();
        let line = node.start_position().row;
        let col = node.start_position().column;
        let line_text = src.lines().nth(line).unwrap_or("");
        captures.push(CachedCapture {
            bucket,
            node_kind: node.kind().to_string(),
            text,
            snippet: make_snippet(line_text, col),
            line: line as u32,
            col: col as u32,
            end_line: node.end_position().row as u32,
            end_col: node.end_position().column as u32,
            call_nested: if bucket == 1 { call_is_nested(node) } else { false },
        });
    }

    Some(FileIndex { schema: SCHEMA, captures })
}

/// Turn a file's cached captures into `SearchResult`s for `query`, reproducing
/// the live path's filter / nested-call / per-file-cap semantics exactly.
fn serve(index: &FileIndex, file_path: &str, query: &SearchQuery, bucket: u8) -> Vec<SearchResult> {
    let filter_nested = bucket_filters_nested(bucket) && query.filter_nested_calls;
    let regex = if query.use_regex && !query.capture_filter.is_empty() {
        regex::Regex::new(&query.capture_filter).ok()
    } else {
        None
    };
    let filter_cmp = if query.case_sensitive {
        query.capture_filter.clone()
    } else {
        query.capture_filter.to_lowercase()
    };

    let mut out = Vec::new();
    for c in &index.captures {
        if c.bucket != bucket {
            continue;
        }
        if filter_nested && c.call_nested {
            continue;
        }
        let pass = if let Some(ref re) = regex {
            re.is_match(&c.text)
        } else if filter_cmp.is_empty() {
            true
        } else {
            let hay = if query.case_sensitive { c.text.clone() } else { c.text.to_lowercase() };
            hay.contains(&filter_cmp)
        };
        if !pass {
            continue;
        }
        out.push(SearchResult {
            file_path: file_path.to_string(),
            line: c.line as usize,
            col: c.col as usize,
            end_line: c.end_line as usize,
            end_col: c.end_col as usize,
            snippet: c.snippet.clone(),
            node_kind: c.node_kind.clone(),
            capture_name: "match".to_string(),
        });
        if out.len() >= MAX_RESULTS_PER_FILE {
            break;
        }
    }
    out
}

// ── Store ────────────────────────────────────────────────────────────────────

pub struct SymCache {
    /// `.../mastdiff/v{SCHEMA}` — None when disabled (cap == 0).
    dir: Option<PathBuf>,
    cap_bytes: u64,
    /// Running total of on-disk artifact bytes, seeded by an initial scan.
    cur_bytes: AtomicU64,
    /// Serializes sweeps so concurrent writers in one process don't double-evict.
    sweep_lock: Mutex<()>,
    query: Option<Query>,
}

impl SymCache {
    /// Open (and create) the cache rooted at the default cache directory.
    /// `cap_mb == 0` disables it: every call becomes a no-op miss.
    pub fn open(cap_mb: u64) -> Self {
        Self::open_in(default_cache_root(), cap_mb)
    }

    /// Open a cache under an explicit root (used by tests and the bench so runs
    /// are isolated and reproducible).
    pub fn open_in(root: PathBuf, cap_mb: u64) -> Self {
        if cap_mb == 0 {
            return Self {
                dir: None,
                cap_bytes: 0,
                cur_bytes: AtomicU64::new(0),
                sweep_lock: Mutex::new(()),
                query: None,
            };
        }
        // Drop stale schema dirs so an old format can't accumulate forever.
        gc_old_schemas(&root);
        let dir = root.join(format!("v{SCHEMA}"));
        let _ = fs::create_dir_all(&dir);
        let cur = dir_size(&dir);
        SymCache {
            dir: Some(dir),
            cap_bytes: cap_mb.saturating_mul(1024 * 1024),
            cur_bytes: AtomicU64::new(cur),
            sweep_lock: Mutex::new(()),
            query: compile_combined_query(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.dir.is_some() && self.query.is_some()
    }

    fn path_for(&self, key: &str) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        let shard = &key[..2.min(key.len())];
        Some(dir.join(shard).join(format!("{key}.json")))
    }

    fn load(&self, key: &str) -> Option<FileIndex> {
        let path = self.path_for(key)?;
        let raw = fs::read(&path).ok()?;
        let idx: FileIndex = serde_json::from_slice(&raw).ok()?;
        if idx.schema != SCHEMA {
            return None;
        }
        Some(idx)
    }

    fn store(&self, key: &str, idx: &FileIndex) {
        let Some(path) = self.path_for(key) else { return };
        let Ok(bytes) = serde_json::to_vec(idx) else { return };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Atomic publish: write a unique temp file, then rename into place.
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        if fs::write(&tmp, &bytes).is_err() {
            return;
        }
        if fs::rename(&tmp, &path).is_err() {
            let _ = fs::remove_file(&tmp);
            return;
        }
        let total = self.cur_bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed) + bytes.len() as u64;
        if total > self.cap_bytes {
            self.sweep();
        }
    }

    /// Evict oldest-first until back under the low-water mark. Bounded work:
    /// one directory scan, one sort, delete the tail.
    fn sweep(&self) {
        let Some(dir) = self.dir.as_ref() else { return };
        let _guard = self.sweep_lock.lock().unwrap();
        let target = (self.cap_bytes as f64 * LOW_WATER) as u64;

        let mut entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        let mut total: u64 = 0;
        for shard in read_dir(dir) {
            for f in read_dir(&shard) {
                if f.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(meta) = fs::metadata(&f) {
                    let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    total += meta.len();
                    entries.push((f, meta.len(), mtime));
                }
            }
        }
        // Oldest first.
        entries.sort_by_key(|(_, _, mtime)| *mtime);
        for (path, len, _) in entries {
            if total <= target {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
        self.cur_bytes.store(total, Ordering::Relaxed);
        log::debug!("symcache: swept to {} bytes (cap {})", total, self.cap_bytes);
    }

    /// Search one file: serve from the cache if the content is indexed,
    /// otherwise parse, store the fresh index, and serve. Returns the results
    /// plus whether it was a cache hit (no parse). Files that can't be read,
    /// are too large, or use a non-shorthand query yield `(live_results, false)`.
    pub fn search_file(&self, file_path: &str, query: &SearchQuery) -> (Vec<SearchResult>, bool) {
        let Some(bucket) = bucket_of_query(&query.ts_query_src) else {
            // Raw S-expr / grep: not indexable — fall back to the live path.
            return (search::search_file(file_path, query), false);
        };
        if !self.is_enabled() {
            return (search::search_file(file_path, query), false);
        }
        let Ok(bytes) = fs::read(file_path) else {
            return (Vec::new(), false);
        };
        if bytes.len() > MAX_FILE_BYTES {
            return (search::search_file(file_path, query), false);
        }
        let key = content_key(&bytes);
        if let Some(idx) = self.load(&key) {
            return (serve(&idx, file_path, query, bucket), true);
        }
        // Miss: parse once, extract all buckets, store, then serve this bucket.
        let Ok(src) = String::from_utf8(bytes) else {
            return (search::search_file(file_path, query), false);
        };
        let query_ts = self.query.as_ref().unwrap();
        match build_index(&src, query_ts) {
            Some(idx) => {
                let results = serve(&idx, file_path, query, bucket);
                self.store(&key, &idx);
                (results, false)
            }
            None => (Vec::new(), false),
        }
    }

    /// Search every file in parallel, serving from the cache where possible.
    /// Returns `(sorted_results, index_hits)` — `index_hits` is the number of
    /// files answered from disk without a re-parse.
    pub fn search_project(&self, files: &[String], query: &SearchQuery) -> (Vec<SearchResult>, usize) {
        use rayon::prelude::*;
        use std::sync::atomic::AtomicUsize;

        if query.is_empty() {
            return (Vec::new(), 0);
        }
        let hits = AtomicUsize::new(0);
        let mut results: Vec<SearchResult> = files
            .par_iter()
            .flat_map(|f| {
                let (r, hit) = self.search_file(f, query);
                if hit {
                    hits.fetch_add(1, Ordering::Relaxed);
                }
                r
            })
            .collect();
        results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
        (results, hits.into_inner())
    }

    /// Streaming variant for the daemon: sends batches of hits through `tx` as
    /// files are searched and bails out when `cancel` is set, preserving the
    /// daemon's incremental + cancellable behavior. Returns the number of files
    /// served from disk without a re-parse.
    pub fn search_streaming(
        &self,
        files: &[String],
        query: &SearchQuery,
        tx: std::sync::mpsc::SyncSender<Vec<SearchResult>>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> usize {
        use rayon::prelude::*;
        use std::sync::atomic::AtomicUsize;

        if query.is_empty() {
            return 0;
        }
        let hits = AtomicUsize::new(0);
        files.par_iter().for_each_with(tx, |tx, f| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let (r, hit) = self.search_file(f, query);
            if hit {
                hits.fetch_add(1, Ordering::Relaxed);
            }
            if !r.is_empty() {
                let _ = tx.send(r); // receiver gone (cancelled) → drop silently
            }
        });
        hits.into_inner()
    }

    /// On-disk byte total and entry count, for `--cache stats`.
    pub fn stats(&self) -> (u64, usize) {
        let Some(dir) = self.dir.as_ref() else { return (0, 0) };
        let mut bytes = 0;
        let mut count = 0;
        for shard in read_dir(dir) {
            for f in read_dir(&shard) {
                if f.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(meta) = fs::metadata(&f) {
                        bytes += meta.len();
                        count += 1;
                    }
                }
            }
        }
        (bytes, count)
    }
}

// ── Filesystem helpers ───────────────────────────────────────────────────────

fn default_cache_root() -> PathBuf {
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".cache"))
                .unwrap_or_else(|_| std::env::temp_dir())
        });
    base.join("mastdiff")
}

fn read_dir(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default()
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    for shard in read_dir(dir) {
        for f in read_dir(&shard) {
            if let Ok(meta) = fs::metadata(&f) {
                total += meta.len();
            }
        }
    }
    total
}

/// Remove any `v{N}` directory whose N != current SCHEMA.
fn gc_old_schemas(root: &Path) {
    let current = format!("v{SCHEMA}");
    for entry in read_dir(root) {
        if let Some(name) = entry.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('v') && name != current && name[1..].parse::<u32>().is_ok() {
                let _ = fs::remove_dir_all(&entry);
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::parse_query;

    fn temp_root() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("mastdiff-symcache-test-{}-{}", std::process::id(), rand_suffix()));
        p
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }

    #[test]
    fn content_key_is_stable_and_length_sensitive() {
        assert_eq!(content_key(b"hello"), content_key(b"hello"));
        assert_ne!(content_key(b"hello"), content_key(b"helloo"));
    }

    #[test]
    fn combined_query_compiles() {
        assert!(compile_combined_query().is_some(), "combined bucket query must compile");
    }

    #[test]
    fn round_trip_matches_live_search() {
        let root = temp_root();
        let cache = SymCache::open_in(root.clone(), 64);

        let src = "examples/sample_project/src/entity.cpp";
        for (i, q) in ["fn:", "call:", "var:", "class:", "fn:update", "call:render", "param:dt"]
            .into_iter()
            .enumerate()
        {
            // Fresh cache dir per query so each starts cold and we can assert
            // the miss→hit transition cleanly.
            let cache = SymCache::open_in(root.join(format!("q{i}")), 64);
            let query = parse_query(q);
            let live = search::search_file(src, &query);

            // First pass: cold (miss → parse + store). Second pass: warm (hit).
            let (cold, hit1) = cache.search_file(src, &query);
            let (warm, hit2) = cache.search_file(src, &query);
            assert!(!hit1, "{q}: first lookup should be a miss");
            assert!(hit2, "{q}: second lookup should hit the cache");

            assert_eq!(cold, live, "{q}: cold cache results must match live search");
            assert_eq!(warm, live, "{q}: warm cache results must match live search");
        }

        // One index serves every bucket: after any query warms a file, a
        // different bucket on the same file is an immediate hit (no re-parse).
        let shared = SymCache::open_in(root.join("shared"), 64);
        let (_, miss) = shared.search_file(src, &parse_query("fn:"));
        let (_, hit) = shared.search_file(src, &parse_query("call:"));
        assert!(!miss, "first bucket builds the index");
        assert!(hit, "second bucket reuses the same content-keyed index");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn disabled_cache_is_a_passthrough() {
        let cache = SymCache::open_in(temp_root(), 0);
        assert!(!cache.is_enabled());
        let query = parse_query("fn:");
        let src = "examples/sample_project/src/entity.cpp";
        let (results, hit) = cache.search_file(src, &query);
        assert!(!hit);
        assert_eq!(results, search::search_file(src, &query));
    }
}
