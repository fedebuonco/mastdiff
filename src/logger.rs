//! File-based logger for the `log` crate.
//!
//! Appends structured log lines to `mastdiff.log` in the working directory.
//! Each line includes an elapsed timestamp, level, shortened module name,
//! and — when available — the source file and line number of the call site
//! (learned from ripgrep's logger which includes this for easy debugging).
//!
//! Silently does nothing if the log file cannot be opened, so the app works
//! correctly in read-only environments.

use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;

struct FileLogger {
    file: Mutex<File>,
    level: LevelFilter,
    /// Wall-clock start used to produce elapsed timestamps (no external dep needed).
    start: Instant,
}

impl Log for FileLogger {
    fn enabled(&self, meta: &Metadata) -> bool {
        meta.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let elapsed = self.start.elapsed();
        let secs = elapsed.as_secs();
        let ms   = elapsed.subsec_millis();
        // Shorten the module target to the last segment for readability.
        let target = record.target().rsplit("::").next().unwrap_or(record.target());
        // Include source file:line when available — learned from ripgrep's logger.
        // This makes it possible to grep the log and jump straight to the call site.
        let location = match (record.file(), record.line()) {
            (Some(f), Some(l)) => format!(" ({}:{})", f, l),
            (Some(f), None)    => format!(" ({})", f),
            _                  => String::new(),
        };
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(
                f,
                "[{:>6}.{:03}s] [{:<5}] {:<20}{} {}",
                secs,
                ms,
                record.level(),
                target,
                location,
                record.args()
            );
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

/// Initialise file logging. Appends to `path`; silently does nothing if the
/// file cannot be opened (so the app still starts in read-only environments).
pub fn init(path: &str, level: LevelFilter) -> Result<(), SetLoggerError> {
    let Ok(file) = OpenOptions::new().create(true).append(true).open(path) else {
        return Ok(()); // can't open log file — log nothing, don't crash
    };
    let logger = Box::new(FileLogger {
        file: Mutex::new(file),
        level,
        start: Instant::now(),
    });
    log::set_boxed_logger(logger)?;
    log::set_max_level(level);
    Ok(())
}
