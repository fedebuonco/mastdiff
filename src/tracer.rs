//! Chrome Trace Event Format collector.
//!
//! Compiled in only with `--features bench`. In normal builds every call is a
//! zero-cost no-op.  Usage:
//!
//! ```ignore
//! // at process start
//! crate::tracer::init();
//!
//! // inside any function — guard records on drop
//! let _s = crate::tracer::span("module::fn_name");
//!
//! // on exit
//! crate::tracer::save("trace.json").ok();
//! ```

// ── Active implementation (--features bench) ──────────────────────────────

#[cfg(feature = "bench")]
mod inner {
    use serde_json::{json, Value};
    use std::{
        fs,
        io::Write,
        sync::{
            atomic::{AtomicU64, Ordering},
            Mutex, OnceLock,
        },
        time::{Duration, Instant},
    };

    struct RawSpan {
        name: &'static str,
        tid: u64,
        ts: Instant,
        dur: Duration,
    }

    static EPOCH: OnceLock<Instant> = OnceLock::new();
    static SPANS: OnceLock<Mutex<Vec<RawSpan>>> = OnceLock::new();
    static NEXT_TID: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        static TID: u64 = {
            // Rayon workers expose their pool index; every other thread (main,
            // project-loader, …) gets the next available ID from the counter.
            // We add rayon::current_num_threads() as an offset so that rayon
            // workers occupy contiguous IDs starting right after thread 0.
            if let Some(i) = rayon::current_thread_index() {
                // Reserve 0 for the main / non-rayon threads, push rayon to 1..=N
                (i as u64) + 1
            } else {
                // Non-rayon threads: grab a unique ID ≥ rayon pool size + 1
                let n = rayon::current_num_threads() as u64 + 1;
                let raw = NEXT_TID.fetch_add(1, Ordering::Relaxed);
                n + raw
            }
        };
    }

    fn tid() -> u64 { TID.with(|&t| t) }
    fn epoch() -> Instant { *EPOCH.get_or_init(Instant::now) }
    fn store() -> &'static Mutex<Vec<RawSpan>> {
        SPANS.get_or_init(|| Mutex::new(Vec::with_capacity(65536)))
    }

    pub fn init() {
        EPOCH.get_or_init(Instant::now);
        SPANS.get_or_init(|| Mutex::new(Vec::with_capacity(65536)));
    }

    pub fn push(name: &'static str, start: Instant, dur: Duration) {
        store().lock().unwrap().push(RawSpan { name, tid: tid(), ts: start, dur });
    }

    pub fn clear() { store().lock().unwrap().clear(); }

    pub fn count() -> usize { store().lock().unwrap().len() }

    pub fn save(path: &str) -> std::io::Result<()> {
        let epoch = epoch();
        let spans = store().lock().unwrap();

        // Thread-name metadata events — one per unique tid seen.
        let mut seen_tids: Vec<u64> = spans.iter().map(|s| s.tid).collect();
        seen_tids.sort_unstable();
        seen_tids.dedup();

        let n_rayon = rayon::current_num_threads() as u64;
        let mut events: Vec<Value> = seen_tids
            .into_iter()
            .map(|tid| {
                let name = if tid == 0 {
                    "main".to_string()
                } else if tid <= n_rayon {
                    format!("rayon-{}", tid - 1)
                } else {
                    format!("thread-{}", tid)
                };
                json!({"name":"thread_name","ph":"M","pid":1,"tid":tid,"args":{"name":name}})
            })
            .collect();

        // Span events.
        for s in spans.iter() {
            let ts = s.ts.saturating_duration_since(epoch).as_micros() as u64;
            let dur = s.dur.as_micros() as u64;
            events.push(json!({
                "name": s.name,
                "ph": "X",
                "ts": ts,
                "dur": dur.max(1),
                "pid": 1,
                "tid": s.tid,
            }));
        }

        let trace = json!({"traceEvents": events, "displayTimeUnit": "ms"});
        let mut f = fs::File::create(path)?;
        write!(f, "{}", serde_json::to_string(&trace)?)?;
        eprintln!("[tracer] {} spans → {}", spans.len(), path);
        Ok(())
    }

    pub struct Span {
        name: &'static str,
        start: Instant,
    }

    impl Span {
        #[inline]
        pub fn new(name: &'static str) -> Self { Self { name, start: Instant::now() } }
    }

    impl Drop for Span {
        #[inline]
        fn drop(&mut self) { push(self.name, self.start, self.start.elapsed()); }
    }

    #[inline]
    pub fn span(name: &'static str) -> Span { Span::new(name) }
}

// ── No-op stubs (default builds) ─────────────────────────────────────────

#[cfg(not(feature = "bench"))]
mod inner {
    pub struct Span;
    impl Span { #[allow(dead_code)] #[inline(always)] pub fn new(_: &'static str) -> Self { Span } }
    #[inline(always)] pub fn init() {}
    #[inline(always)] pub fn span(_: &'static str) -> Span { Span }
    #[inline(always)] pub fn clear() {}
    #[inline(always)] pub fn count() -> usize { 0 }
    #[inline(always)] pub fn save(_: &str) -> std::io::Result<()> { Ok(()) }
}

pub use inner::{init, span, save};
#[allow(unused)] pub use inner::{clear, count, Span};
