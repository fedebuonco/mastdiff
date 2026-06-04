# Performance — profiling, benchmarks, and optimisations

---

## Benchmark binary

A dedicated benchmark binary lives in `src/bin/bench.rs`.  It is compiled
only when the `bench` Cargo feature is enabled:

```bash
# Run against a project directory, write trace.json
cargo run --release --features bench --bin bench -- <dir> [--output trace.json] [--runs 3]

# Open the trace in the browser
# → chrome://tracing  (load file)
# → https://ui.perfetto.dev  (drag-and-drop)
```

The binary:
1. Collects C++ files via `walkdir` (same logic as the real app).
2. Runs one warmup pass to prime the OS file cache.
3. Times each query type for `--runs` iterations using the same
   `search_project` path the app uses.
4. Writes a **Chrome Trace Event Format** JSON file with per-phase spans.

---

## Tracer

`src/tracer.rs` is a lightweight Chrome-trace collector compiled in only with
`--features bench`.  In normal builds every call is a zero-cost inline no-op.

```rust
// In any function
let _s = crate::tracer::span("module::fn_name");   // records on drop (RAII)
```

The active implementation uses a global `Mutex<Vec<RawSpan>>` — acceptable
for profiling builds where some overhead is tolerable.  Each OS thread gets
a stable numeric TID (Rayon workers reuse their pool index; other threads get
sequential IDs).

**Instrumented call sites** (all behind `--features bench`):

| Module | Spans |
|--------|-------|
| `search` | `search::project_streaming`, `search::file`, `search::file::read`, `search::src`, `search::src::prefilter`, `search::src::query_compile`, `search::src::parser_init`, `search::src::parse`, `search::src::filter_compile`, `search::src::walk_captures`, `search::src::grep` |
| `project` | `project::load`, `project::load_streaming`, `project::analyze_includes`, `project::load_compile_commands`, `project::load_from_walk`, `project::walk_headers`, `project::extract_includes`, `project::find_cmake_targets` |
| `ast_diff` | `ast::compute_diff`, `ast::parse_single`, `ast::flatten_tree`, `ast::diff_flat_trees`, `ast::visible_rows`, `ast::filter_rows` |
| `syntax` | `syntax::highlight` |
| `text_diff` | `text_diff::compute_diff`, `text_diff::inline_spans` |
| `app` | `app::handle_key`, `app::run_search`, `app::tick_search`, `app::load_search_result`, `app::load_file_into_search_pane`, `app::rebuild_search_ast_visible`, `app::rebuild_ast_visible`, `app::launch_ast_diff`, `app::rebuild_project_display`, `app::handle_load_msg`, `app::finish_project_load`, `app::open_project_file` |

**Collecting a trace from the real app** (requires `--features bench` build):

```bash
cargo build --release --features bench
./target/release/mastdiff examples/ladybird/ --trace trace.json
# use the app normally, then press q
# → trace.json written on exit
```

---

## Baseline profiling results (Ladybird, 4 680 C++ files, 8 threads)

Measured with `--runs 3` after OS file cache was warm.

### Before optimisations

| Query | Time | ms/file |
|-------|------|---------|
| `fn:` | 15 988 ms | 3.42 |
| `var:` | 14 645 ms | 3.13 |
| `call:` | 5 971 ms | 1.28 |
| `grep` | 53 ms | 0.01 |

**Flame graph breakdown of `fn:` per file:**

```
search::src                 31.2 ms  100%
  search::src::query_compile  26.3 ms   84%   ← dominant cost
  search::src::parse           3.8 ms   12%
  search::src::walk_captures   0.7 ms    2%
  search::file::read           0.2 ms    1%
```

---

## Optimisation 1 — compile `Query` once per search, not per file

**Problem.**  `tree_sitter::Query::new()` compiled the same S-expression
once per file per search.  At ~15 ms/compile × 4 680 files it consumed
**93% of all search CPU time** while producing an identical result every time.

**Fix.**  `PreparedSearch` is constructed once in `search_project` /
`search_project_streaming` before the Rayon parallel iterator, then shared
across all workers as `&PreparedSearch`.  Both `tree_sitter::Query` and
`regex::Regex` implement `Send + Sync`, so the reference is safe to share.

```rust
// search.rs — compiled once per search call
struct PreparedSearch {
    ts_query:             Query,
    capture_regex:        Option<Regex>,
    capture_filter_cmp:   String,
}

// search_project_streaming
let prepared = PreparedSearch::new(query);   // one Query::new() here …
files.par_iter().for_each_with(tx, |tx, f| {
    search_file_prepared(f, query, prepared.as_ref())  // … reused here
});
```

The same fix was applied to `project::analyze_includes`, which previously
compiled a `Query` for `#include` extraction inside `extract_local_includes`
— once per source file on every project load (~2 250 calls, ~8.8 s).

**Result.**

| Query | Before | After | Speedup |
|-------|--------|-------|---------|
| `fn:` | 15 988 ms | 1 092 ms | **14.7×** |
| `var:` | 14 645 ms | 1 020 ms | **14.4×** |
| `class:` | 5 106 ms | 877 ms | **5.8×** |
| `call:` | 5 971 ms | 1 193 ms | **5.0×** |
| `include:` | 3 633 ms | 874 ms | **4.2×** |

---

## Optimisation 2 — text pre-filter before tree-sitter parse

**Problem.**  After optimisation 1, `parser.parse()` (tree-sitter AST
construction) became the dominant cost at **71% of search CPU time**
(mean 1.15 ms/file, p99 12.6 ms).  For a query like `fn:update`, 99% of
files produce zero captures but still pay the full parse cost.

**Fix.**  When a capture filter is present (e.g. `update` for `fn:update`),
scan the raw source bytes for the filter string before invoking tree-sitter.
If the identifier cannot appear in the file at all, skip parsing entirely.
The check costs ~8 µs/file — two orders of magnitude cheaper than a parse.

```rust
// search_src_prepared — runs before Parser::new()
if !prepared.capture_filter_cmp.is_empty() {
    let hit = if let Some(ref re) = prepared.capture_regex {
        re.is_match(src)
    } else if query.case_sensitive {
        src.contains(prepared.capture_filter_cmp.as_str())
    } else {
        src.as_bytes()
            .windows(prepared.capture_filter_cmp.len())
            .any(|w| w.eq_ignore_ascii_case(prepared.capture_filter_cmp.as_bytes()))
    };
    if !hit { return vec![]; }
}
```

This is a **safe pre-filter**: it has no false negatives (if the identifier
doesn't exist as bytes in the file it cannot be a tree-sitter capture), only
false positives (word appears in a comment — file is parsed and produces no
captures, which is correct).

**Skip rates and speedups measured on Ladybird:**

| Query | Files skipped | Skip rate | Wall time | Speedup vs unfiltered |
|-------|--------------|-----------|-----------|----------------------|
| `fn:paint` | 4 289 / 4 680 | 91.6% | 214 ms | **4.7×** |
| `fn:update` | 4 225 / 4 680 | 90.3% | 274 ms | **3.7×** |
| `var:buffer` | 3 951 / 4 680 | 84.4% | 352 ms | **2.9×** |
| `call:malloc` | 4 651 / 4 680 | 99.4% | 55 ms | **20×** |

Unfiltered queries (`fn:`, `call:`, `var:`) are unaffected — the guard is a
no-op when `capture_filter_cmp` is empty.

---

## Current bottleneck (after both optimisations)

For unfiltered queries, `parser.parse()` is the floor:

```
per file CPU time — unfiltered query (e.g. fn:)
  parse             ~1.15 ms/file   71%   unavoidable
  walk_captures     ~0.41 ms/file   26%
  file read         ~0.05 ms/file    3%
```

Parse cost has high variance (p50: 0.33 ms, p99: 12.6 ms, max: 136 ms)
driven by file size.  The next meaningful optimisation would be an **AST
cache** — storing `(mtime, Tree)` per file and skipping re-parse when the
file has not changed.  This would reduce repeated searches over the same
codebase to essentially the `walk_captures` cost alone.
