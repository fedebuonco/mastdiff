use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

struct FileLogger {
    file: Mutex<File>,
    level: LevelFilter,
}

impl Log for FileLogger {
    fn enabled(&self, meta: &Metadata) -> bool {
        meta.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "[{:<5}] {} — {}", record.level(), record.target(), record.args());
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
    });
    log::set_boxed_logger(logger)?;
    log::set_max_level(level);
    Ok(())
}
