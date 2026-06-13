//! Persistent search daemon — keeps the AST cache warm between queries.
//!
//! Start with `mastdiff --daemon <root>`.  The daemon:
//!   1. Discovers all C++ files in `<root>`.
//!   2. Listens on a random TCP port (127.0.0.1:0).
//!   3. Writes the port number to the port file (see [`port_file`]).
//!   4. Prints `"READY <port>"` to stderr so the spawner knows it's up.
//!   5. Handles one search request per TCP connection.
//!   6. Exits after 300 s of idle (no connections).
//!
//! # Protocol
//!
//! Client → server (one JSON line, then close write side):
//! ```json
//! {"query":"fn:update","include":"src/**","exclude":"","regex":false,"case_sensitive":false}
//! ```
//!
//! Server → client (N JSON lines, then closes the connection):
//! ```json
//! {"type":"hit","file":"/abs/path","line":1,"col":6,"end_line":1,"end_col":12,"text":"void update()","kind":"identifier","capture":"match","hl_start":5,"hl_end":11}
//! {"type":"done","total":42,"ms":86,"cache_hits":40,"files":42}
//! ```
//! On failure the server sends `{"type":"error","message":"..."}` instead of `done`.
//!
//! Cancellation: the client closes its socket; the daemon detects the EOF
//! on a background read and stops the in-flight search.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ast_cache::AstCache;
use crate::config::Config;
use crate::project;
use crate::search;
use crate::symcache::SymCache;

const IDLE_TIMEOUT_SECS: u64 = 300;
const FILE_REFRESH_SECS: u64 = 30;

// ── Port file ─────────────────────────────────────────────────────────────

fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x00000100000001b3);
    }
    hash
}

/// Returns `$TMPDIR/mastdiff-<fnv1a(canonical_root)>.port`.
///
/// Both the daemon and the CLI client must agree on this path so they can
/// find each other.  JavaScript callers must implement the same FNV-1a hash
/// over the UTF-8 bytes of the canonical root path.
pub fn port_file(root: &Path) -> PathBuf {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let hash = fnv1a(&canonical.to_string_lossy());
    std::env::temp_dir().join(format!("mastdiff-{:016x}.port", hash))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Wire types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct DaemonRequest {
    pub query: String,
    #[serde(default)]
    pub include: String,
    #[serde(default)]
    pub exclude: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum DaemonMsg {
    Hit {
        file: String,
        line: u32,
        col: u32,
        end_line: u32,
        end_col: u32,
        text: String,
        kind: String,
        capture: String,
        /// Byte range of the match within `text` (trimmed line), for
        /// in-row highlighting. `hl_start == hl_end` means no usable span.
        #[serde(default)]
        hl_start: u32,
        #[serde(default)]
        hl_end: u32,
    },
    Done {
        total: usize,
        /// Search wall time in milliseconds.
        #[serde(default)]
        ms: u64,
        /// Files whose parse tree came from the warm AST cache.
        #[serde(default)]
        cache_hits: usize,
        /// Total files searched (after include/exclude filtering).
        #[serde(default)]
        files: usize,
    },
    Error {
        message: String,
    },
}

pub struct DaemonHit {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub text: String,
    pub kind: String,
    pub capture: String,
    pub hl_start: u32,
    pub hl_end: u32,
}

/// Byte range of a match within the trimmed snippet line, clamped to char
/// boundaries so the range is always safe to slice with.
///
/// `col`/`end_col` are 0-based byte columns in the original (untrimmed)
/// line; `same_line` is false when the match continues past this line, in
/// which case the highlight extends to the end of the snippet.
pub fn highlight_span(line: &str, text: &str, col: usize, end_col: usize, same_line: bool) -> (u32, u32) {
    let lead = line.len() - line.trim_start().len();
    let clamp = |b: usize| -> usize {
        let mut b = b.min(text.len());
        while b > 0 && !text.is_char_boundary(b) {
            b -= 1;
        }
        b
    };
    let start = clamp(col.saturating_sub(lead));
    let end = if same_line { clamp(end_col.saturating_sub(lead)).max(start) } else { text.len() };
    (start as u32, end as u32)
}

// ── Daemon server ─────────────────────────────────────────────────────────

struct DaemonState {
    root: PathBuf,
    files: Vec<String>,
    last_refresh: Instant,
    ast_cache: Arc<Mutex<AstCache>>,
    /// Persistent on-disk symbol cache — shorthand queries are served from
    /// here, so a restarted daemon comes up warm from disk.
    symcache: Arc<SymCache>,
}

impl DaemonState {
    fn new(root: PathBuf, files: Vec<String>, cap_mb: u64, sym_cache_mb: u64) -> Self {
        Self {
            root,
            files,
            last_refresh: Instant::now(),
            ast_cache: Arc::new(Mutex::new(AstCache::new(cap_mb))),
            symcache: Arc::new(SymCache::open(sym_cache_mb)),
        }
    }

    fn refresh_if_stale(&mut self) {
        if self.last_refresh.elapsed().as_secs() < FILE_REFRESH_SECS {
            return;
        }
        match project::load(&self.root) {
            Ok(data) => {
                self.files = data.files.iter().map(|tu| tu.file_path.clone()).collect();
                self.last_refresh = Instant::now();
                log::info!("daemon: refreshed file list ({} files)", self.files.len());
            }
            Err(e) => {
                log::warn!("daemon: file list refresh failed: {}", e);
                self.last_refresh = Instant::now(); // avoid spam
            }
        }
    }
}

pub fn run_daemon(root: &Path, cfg: &Config) -> Result<()> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    log::info!("daemon: root={:?}", root);

    let data = project::load(&root)?;
    let files: Vec<String> = data.files.iter().map(|tu| tu.file_path.clone()).collect();
    log::info!("daemon: {} files discovered", files.len());

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let pf = port_file(&root);
    std::fs::write(&pf, port.to_string())?;
    log::info!("daemon: port file {:?} → {}", pf, port);

    eprintln!("READY {}", port);

    let state = Arc::new(Mutex::new(DaemonState::new(
        root,
        files,
        cfg.ast_cache_mb,
        cfg.sym_cache_mb,
    )));

    // Idle watchdog — exits after IDLE_TIMEOUT_SECS of no connections.
    let last_activity = Arc::new(AtomicU64::new(unix_now()));
    {
        let la = Arc::clone(&last_activity);
        let pf2 = pf.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(10));
            let idle = unix_now().saturating_sub(la.load(Ordering::Relaxed));
            if idle >= IDLE_TIMEOUT_SECS {
                log::info!("daemon: idle timeout, exiting");
                let _ = std::fs::remove_file(&pf2);
                std::process::exit(0);
            }
        });
    }

    // Background pre-warm: run a `fn:` search through the symbol cache. Because
    // one content-keyed index covers every bucket, this builds (and persists)
    // the full per-file index for the whole tree — so the daemon, and any later
    // process sharing the disk cache, start warm.
    {
        let state2 = Arc::clone(&state);
        std::thread::spawn(move || {
            let (files, symcache) = {
                let st = state2.lock().unwrap();
                (st.files.clone(), Arc::clone(&st.symcache))
            };
            log::info!("daemon: pre-warming symbol cache ({} files)", files.len());
            let query = search::parse_query("fn:");
            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, rx) = std::sync::mpsc::sync_channel(512);
            std::thread::spawn(move || {
                let _ = symcache.search_streaming(&files, &query, tx, cancel);
            });
            let mut n = 0usize;
            while let Ok(batch) = rx.recv() {
                n += batch.len();
            }
            log::info!("daemon: pre-warm done ({} fn: results)", n);
        });
    }

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                log::warn!("daemon: accept error: {}", e);
                continue;
            }
        };
        last_activity.store(unix_now(), Ordering::Relaxed);
        let state = Arc::clone(&state);
        let la2 = Arc::clone(&last_activity);
        std::thread::spawn(move || {
            if let Err(e) = handle_conn(stream, state) {
                log::warn!("daemon: connection error: {}", e);
            }
            la2.store(unix_now(), Ordering::Relaxed);
        });
    }

    Ok(())
}

fn handle_conn(stream: TcpStream, state: Arc<Mutex<DaemonState>>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    let mut raw = String::new();
    reader.read_line(&mut raw)?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(());
    }

    let req: DaemonRequest = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            let msg = serde_json::to_string(&DaemonMsg::Error { message: format!("bad request: {e}") })?;
            let _ = writeln!(writer, "{msg}");
            return Ok(());
        }
    };
    log::debug!("daemon: search {:?}", req.query);

    let (files, ast_cache, symcache) = {
        let mut st = state.lock().unwrap();
        st.refresh_if_stale();
        let root_str = st.root.to_string_lossy().to_string();
        let include = search::FileFilter::parse(&req.include);
        let exclude = search::FileFilter::parse(&req.exclude);
        let files: Vec<String> = st
            .files
            .iter()
            .filter(|p| {
                let rel = p
                    .strip_prefix(&format!("{root_str}/"))
                    .or_else(|| p.strip_prefix(root_str.as_str()))
                    .unwrap_or(p.as_str());
                let ok_inc = include.is_empty() || include.matches(rel) || include.matches(p);
                let ok_exc = exclude.is_empty() || (!exclude.matches(rel) && !exclude.matches(p));
                ok_inc && ok_exc
            })
            .cloned()
            .collect();
        (files, Arc::clone(&st.ast_cache), Arc::clone(&st.symcache))
    };

    let mut query = search::parse_query(&req.query);
    query.use_regex = req.regex;
    query.case_sensitive = req.case_sensitive;

    let files_count = files.len();
    let started = Instant::now();
    let cancel = Arc::new(AtomicBool::new(false));

    // Cancellation: the client signals "stop" by closing its socket.  Keep
    // reading the connection in the background — EOF or error means the
    // client is gone, so flip the cancel flag and let the workers wind down.
    {
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) => {
                        cancel.store(true, Ordering::Relaxed);
                        return;
                    }
                    Ok(_) => {} // ignore any further lines
                    // The socket has a read timeout — that just means the
                    // client is quiet, not gone.
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(_) => {
                        cancel.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            }
        });
    }

    // Shorthand queries (fn:, call:, …) go through the persistent symbol cache
    // — served from disk, no re-parse. Raw S-expr and grep queries can't be
    // pre-extracted, so they take the live streaming path with the warm
    // in-memory AST cache.
    let use_symcache =
        symcache.is_enabled() && search::bucket_of_query(&query.ts_query_src).is_some();
    let search_cancel = Arc::clone(&cancel);
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<search::SearchResult>>(256);
    let (stats_tx, stats_rx) = std::sync::mpsc::channel::<usize>();
    std::thread::spawn(move || {
        let cache_hits = if use_symcache {
            symcache.search_streaming(&files, &query, tx, search_cancel)
        } else {
            search::search_project_streaming(&files, &query, tx, search_cancel, ast_cache)
        };
        let _ = stats_tx.send(cache_hits);
    });

    let mut file_lines: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut total = 0usize;
    let mut write_ok = true;

    'recv: while let Ok(batch) = rx.recv() {
        for r in batch {
            total += 1;
            let lines = file_lines.entry(r.file_path.clone()).or_insert_with(|| {
                std::fs::read_to_string(&r.file_path)
                    .map(|s| s.lines().map(str::to_string).collect())
                    .unwrap_or_default()
            });
            let (text, hl_start, hl_end) = match lines.get(r.line) {
                Some(l) => {
                    let text = l.trim().to_string();
                    let (s, e) = highlight_span(l, &text, r.col, r.end_col, r.end_line == r.line);
                    (text, s, e)
                }
                None => (r.snippet.clone(), 0, 0),
            };
            let msg = serde_json::to_string(&DaemonMsg::Hit {
                file: r.file_path,
                line: (r.line + 1) as u32,
                col: (r.col + 1) as u32,
                end_line: (r.end_line + 1) as u32,
                end_col: (r.end_col + 1) as u32,
                text,
                kind: r.node_kind,
                capture: r.capture_name,
                hl_start,
                hl_end,
            })?;
            if writeln!(writer, "{msg}").is_err() {
                write_ok = false;
                cancel.store(true, Ordering::Relaxed);
                break 'recv;
            }
        }
    }

    if write_ok {
        let cache_hits = stats_rx.recv().unwrap_or(0);
        let done = serde_json::to_string(&DaemonMsg::Done {
            total,
            ms: started.elapsed().as_millis() as u64,
            cache_hits,
            files: files_count,
        })?;
        let _ = writeln!(writer, "{done}");
    }

    Ok(())
}

// ── Client helper (used by headless --search mode) ────────────────────────

/// Try to forward a search to an already-running daemon for `root`.
/// Returns `None` if no daemon is available (port file missing, connection
/// refused, etc.) so the caller can fall back to spawning a new subprocess.
pub fn try_client_search(root: &Path, req: &DaemonRequest) -> Option<Vec<DaemonHit>> {
    let pf = port_file(root);
    let port_str = std::fs::read_to_string(&pf).ok()?;
    let port: u16 = port_str.trim().parse().ok()?;

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().ok()?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok()?;

    {
        let mut w = &stream;
        let payload = serde_json::to_string(req).ok()? + "\n";
        w.write_all(payload.as_bytes()).ok()?;
    }

    let mut hits = Vec::new();
    for line in BufReader::new(&stream).lines() {
        let line = line.ok()?;
        match serde_json::from_str::<DaemonMsg>(&line).ok()? {
            DaemonMsg::Hit { file, line: ln, col, end_line, end_col, text, kind, capture, hl_start, hl_end } => {
                hits.push(DaemonHit { file, line: ln, col, end_line, end_col, text, kind, capture, hl_start, hl_end });
            }
            DaemonMsg::Done { .. } => break,
            DaemonMsg::Error { message } => {
                log::warn!("daemon reported error: {}", message);
                return None;
            }
        }
    }
    Some(hits)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::highlight_span;

    #[test]
    fn span_on_unindented_line() {
        let line = "void paint(int x);";
        let text = line.trim().to_string();
        // match on "paint": byte cols 5..10
        assert_eq!(highlight_span(line, &text, 5, 10, true), (5, 10));
        assert_eq!(&text[5..10], "paint");
    }

    #[test]
    fn span_shifts_for_leading_indentation() {
        let line = "        obj.paint();";
        let text = line.trim().to_string();
        // "paint" starts at byte 12 in the raw line, 4 in the trimmed text
        let (s, e) = highlight_span(line, &text, 12, 17, true);
        assert_eq!(&text[s as usize..e as usize], "paint");
    }

    #[test]
    fn multiline_match_extends_to_end_of_snippet() {
        let line = "    auto f = [&](int a,";
        let text = line.trim().to_string();
        let (s, e) = highlight_span(line, &text, 9, 3, false);
        assert_eq!(s, 5); // "f = ..." region start
        assert_eq!(e as usize, text.len());
    }

    #[test]
    fn out_of_range_columns_are_clamped() {
        let line = "int x;";
        let text = line.trim().to_string();
        let (s, e) = highlight_span(line, &text, 100, 200, true);
        assert_eq!(s as usize, text.len());
        assert_eq!(e as usize, text.len());
    }

    #[test]
    fn span_clamps_to_char_boundaries() {
        let line = "int café_count;"; // é is 2 bytes (offsets 7-8)
        let text = line.trim().to_string();
        // end col lands mid-é — must back off to a char boundary, not panic
        let (s, e) = highlight_span(line, &text, 4, 8, true);
        assert!(text.is_char_boundary(s as usize));
        assert!(text.is_char_boundary(e as usize));
        let _ = &text[s as usize..e as usize]; // must not panic
    }
}
