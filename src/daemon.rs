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
//! {"type":"hit","file":"/abs/path","line":1,"col":1,"text":"void update()","kind":"identifier","capture":"match"}
//! {"type":"done","total":42}
//! ```
//! On failure the server sends `{"type":"error","message":"..."}` instead of `done`.

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
        text: String,
        kind: String,
        capture: String,
    },
    Done {
        total: usize,
    },
    Error {
        message: String,
    },
}

pub struct DaemonHit {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub text: String,
    pub kind: String,
    pub capture: String,
}

// ── Daemon server ─────────────────────────────────────────────────────────

struct DaemonState {
    root: PathBuf,
    files: Vec<String>,
    last_refresh: Instant,
    ast_cache: Arc<Mutex<AstCache>>,
}

impl DaemonState {
    fn new(root: PathBuf, files: Vec<String>, cap_mb: u64) -> Self {
        Self {
            root,
            files,
            last_refresh: Instant::now(),
            ast_cache: Arc::new(Mutex::new(AstCache::new(cap_mb))),
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

    let state = Arc::new(Mutex::new(DaemonState::new(root, files, cfg.ast_cache_mb)));

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

    // Background pre-warm: run `fn:` search to parse and cache every source file.
    {
        let state2 = Arc::clone(&state);
        std::thread::spawn(move || {
            let (files, ast_cache) = {
                let st = state2.lock().unwrap();
                (st.files.clone(), Arc::clone(&st.ast_cache))
            };
            log::info!("daemon: pre-warming AST cache ({} files)", files.len());
            let query = search::parse_query("fn:");
            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, rx) = std::sync::mpsc::sync_channel(512);
            std::thread::spawn(move || {
                search::search_project_streaming(&files, &query, tx, cancel, ast_cache);
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

    let (files, ast_cache) = {
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
        (files, Arc::clone(&st.ast_cache))
    };

    let mut query = search::parse_query(&req.query);
    query.use_regex = req.regex;
    query.case_sensitive = req.case_sensitive;

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<search::SearchResult>>(256);
    std::thread::spawn(move || {
        search::search_project_streaming(&files, &query, tx, cancel, ast_cache);
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
            let text = lines
                .get(r.line)
                .map(|l| l.trim().to_string())
                .unwrap_or_else(|| r.snippet.clone());
            let msg = serde_json::to_string(&DaemonMsg::Hit {
                file: r.file_path,
                line: (r.line + 1) as u32,
                col: (r.col + 1) as u32,
                text,
                kind: r.node_kind,
                capture: r.capture_name,
            })?;
            if writeln!(writer, "{msg}").is_err() {
                write_ok = false;
                break 'recv;
            }
        }
    }

    if write_ok {
        let done = serde_json::to_string(&DaemonMsg::Done { total })?;
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
            DaemonMsg::Hit { file, line: ln, col, text, kind, capture } => {
                hits.push(DaemonHit { file, line: ln, col, text, kind, capture });
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
