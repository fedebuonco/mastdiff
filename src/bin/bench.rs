//! Search & indexing benchmark — outputs a Chrome Trace Event Format JSON file.
//!
//! Usage:
//!   cargo run --release --features bench --bin bench -- <dir> [--output trace.json] [--runs 3]
//!
//! For each query it reports a **cold** run (fresh, empty AST cache → every
//! file is parsed) and the **warm** average (cache populated → parses are
//! skipped), plus the speedup and cache hit-rate. This is the harness used to
//! prove a caching change actually pays off: the cold column is the baseline,
//! the warm column is what the cache buys you, and the per-span summary at the
//! end shows parse time collapsing.
//!
//! Open the trace in chrome://tracing or https://ui.perfetto.dev

use mastdiff::{
    ast_cache::AstCache,
    search::{parse_query, search_project_streaming, SearchQuery, SearchResult},
    tracer,
};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

fn collect_cpp_files(dir: &str) -> Vec<String> {
    let _s = tracer::span("bench::collect_files");
    WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("");
            matches!(ext, "cpp" | "cc" | "cxx" | "C" | "h" | "hpp" | "hxx" | "H")
        })
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect()
}

/// Run one streaming search to completion against `cache`, draining all hits.
/// Returns `(hit_count, cache_hits)` — `cache_hits` is the number of files
/// served from the warm AST cache (no re-parse).
fn run_search(files: &[String], query: &SearchQuery, cache: Arc<Mutex<AstCache>>) -> (usize, usize) {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel::<Vec<SearchResult>>(256);
    let files = files.to_vec();
    let query = query.clone();
    let handle = std::thread::spawn(move || search_project_streaming(&files, &query, tx, cancel, cache));
    let mut hits = 0;
    while let Ok(batch) = rx.recv() {
        hits += batch.len();
    }
    let cache_hits = handle.join().unwrap_or(0);
    (hits, cache_hits)
}

struct Row {
    label: &'static str,
    hits: usize,
    cold_ms: f64,
    warm_ms: f64,
    cache_hits: usize,
}

/// Cold run = fresh cache (everything parsed). Warm runs = same cache reused.
/// `runs` is the number of warm runs averaged after the single cold run.
fn bench_query(label: &'static str, query_str: &str, files: &[String], runs: usize, cap_mb: u64) -> Row {
    let query = parse_query(query_str);
    let cache = Arc::new(Mutex::new(AstCache::new(cap_mb)));

    // Cold: empty cache — measures the full-parse baseline.
    let t = Instant::now();
    let (hits, _) = run_search(files, &query, Arc::clone(&cache));
    let cold_ms = t.elapsed().as_secs_f64() * 1_000.0;

    // Warm: cache now populated — repeat and average.
    let mut warm_sum = 0.0_f64;
    let mut last_cache_hits = 0;
    let warm_runs = runs.max(1);
    for _ in 0..warm_runs {
        let _s = tracer::span(label);
        let t = Instant::now();
        let (_, ch) = run_search(files, &query, Arc::clone(&cache));
        warm_sum += t.elapsed().as_secs_f64() * 1_000.0;
        last_cache_hits = ch;
    }

    Row { label, hits, cold_ms, warm_ms: warm_sum / warm_runs as f64, cache_hits: last_cache_hits }
}

fn main() {
    tracer::init();

    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| ".".to_string());
    let mut output = "trace.json".to_string();
    let mut runs: usize = 3;
    let mut cap_mb: u64 = 128;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" | "-o" => { if let Some(v) = args.next() { output = v; } }
            "--runs"   | "-n" => { if let Some(v) = args.next() { runs = v.parse().unwrap_or(3); } }
            "--cache-mb"      => { if let Some(v) = args.next() { cap_mb = v.parse().unwrap_or(128); } }
            _ => {}
        }
    }

    let num_threads = rayon::current_num_threads();
    println!("\nmastdiff search benchmark");
    println!("  dir       : {dir}");
    println!("  warm runs : {runs}");
    println!("  threads   : {num_threads}");
    println!("  cache     : {cap_mb} MiB");
    println!();

    let files = collect_cpp_files(&dir);
    if files.is_empty() {
        eprintln!("No C++ files found in {dir}");
        std::process::exit(1);
    }
    println!("  {} C++ files\n", files.len());

    // Reset tracer so file-collection spans don't pollute the search trace.
    tracer::clear();

    let queries: &[(&'static str, &str)] = &[
        // Unfiltered — no pre-filter applies
        ("bench::fn",           "fn:"),
        ("bench::call",         "call:"),
        ("bench::var",          "var:"),
        // Filtered — pre-filter skips most files before tree-sitter parse
        ("bench::fn:paint",     "fn:paint"),
        ("bench::fn:update",    "fn:update"),
        ("bench::call:malloc",  "call:malloc"),
        ("bench::var:buffer",   "var:buffer"),
        ("bench::grep",         "TODO"),
    ];

    println!(
        "{:<14}  {:>7}  {:>9}  {:>9}  {:>8}  {:>9}",
        "query", "hits", "cold ms", "warm ms", "speedup", "cache_hit"
    );
    println!("{}", "-".repeat(66));

    for (label, query_str) in queries {
        let r = bench_query(label, query_str, &files, runs, cap_mb);
        let speedup = if r.warm_ms > 0.0 { r.cold_ms / r.warm_ms } else { 0.0 };
        println!(
            "{:<14}  {:>7}  {:>9.1}  {:>9.1}  {:>7.1}x  {:>4}/{:<4}",
            r.label.trim_start_matches("bench::"),
            r.hits,
            r.cold_ms,
            r.warm_ms,
            speedup,
            r.cache_hits,
            files.len(),
        );
    }

    // Per-span time rollup — shows where the warm-run time actually goes
    // (parse should be near-zero once the cache is warm).
    println!("\nspan totals (warm runs):");
    println!("{:<28}  {:>10}  {:>8}", "span", "total ms", "calls");
    println!("{}", "-".repeat(50));
    for (name, total_us, count) in tracer::summary().into_iter().take(12) {
        println!("{:<28}  {:>10.1}  {:>8}", name, total_us as f64 / 1000.0, count);
    }

    println!();
    tracer::save(&output).unwrap();
    println!("Open in chrome://tracing  or  https://ui.perfetto.dev");
}
