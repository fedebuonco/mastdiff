//! Search & indexing benchmark — outputs a Chrome Trace Event Format JSON file.
//!
//! Usage:
//!   cargo run --release --features bench --bin bench -- <dir> [--output trace.json] [--runs 3]
//!
//! Open the result in chrome://tracing or https://ui.perfetto.dev

use mastdiff::{search::{parse_query, search_project}, tracer};
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

fn bench_query(label: &'static str, query_str: &str, files: &[String], runs: usize) -> (usize, f64) {
    let query = parse_query(query_str);
    let mut sum_ms = 0.0_f64;
    let mut last_hits = 0;

    for _run in 0..runs {
        let _s = tracer::span(label);
        let t = Instant::now();
        // search_project compiles the Query once then fans out across workers
        let results = search_project(files, &query);
        sum_ms += t.elapsed().as_secs_f64() * 1_000.0;
        last_hits = results.len();
    }

    (last_hits, sum_ms / runs as f64)
}

fn main() {
    tracer::init();

    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| ".".to_string());
    let mut output = "trace.json".to_string();
    let mut runs: usize = 3;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" | "-o" => { if let Some(v) = args.next() { output = v; } }
            "--runs"   | "-n" => { if let Some(v) = args.next() { runs = v.parse().unwrap_or(3); } }
            _ => {}
        }
    }

    let num_threads = rayon::current_num_threads();
    println!("\nmastdiff search benchmark");
    println!("  dir      : {dir}");
    println!("  runs     : {runs}");
    println!("  threads  : {num_threads}");
    println!();

    // Warmup (not recorded — tracer is reset after)
    let files_warm = collect_cpp_files(&dir);
    print!("warming up ({} files)... ", files_warm.len());
    let _ = std::io::Write::flush(&mut std::io::stdout());
    for q in &["fn:", "call:", "TODO"] {
        let _ = search_project(&files_warm, &parse_query(q));
    }
    println!("done\n");

    // Reset tracer so warmup spans don't appear in the trace.
    tracer::clear();

    let files = collect_cpp_files(&dir);
    if files.is_empty() {
        eprintln!("No C++ files found in {dir}");
        std::process::exit(1);
    }
    println!("  {} C++ files\n", files.len());

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

    println!("{:<16}  {:>8}  {:>10}  {:>12}", "query", "hits", "avg ms", "ms/file");
    println!("{}", "-".repeat(55));

    for (label, query_str) in queries {
        let (hits, avg_ms) = bench_query(label, query_str, &files, runs);
        println!(
            "{:<16}  {:>8}  {:>10.1}  {:>12.4}",
            label.trim_start_matches("bench::"),
            hits,
            avg_ms,
            avg_ms / files.len() as f64,
        );
    }

    println!();
    tracer::save(&output).unwrap();
    println!("Open in chrome://tracing  or  https://ui.perfetto.dev");
}
