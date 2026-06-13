//! User configuration loaded from `~/.config/mastdiff/config.toml`.
//!
//! Minimal example config:
//! ```toml
//! open_in = "vim"   # or "vscode"
//! ```
//!
//! If the file is missing or unparseable the app falls back to defaults silently.

use serde::Deserialize;
use std::path::PathBuf;

// ── LogLevel ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

impl LogLevel {
    pub fn to_level_filter(&self) -> log::LevelFilter {
        match self {
            Self::Off   => log::LevelFilter::Off,
            Self::Error => log::LevelFilter::Error,
            Self::Warn  => log::LevelFilter::Warn,
            Self::Info  => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Off   => "off",
            Self::Error => "error",
            Self::Warn  => "warn",
            Self::Info  => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

// ── OpenIn ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenIn {
    Vim,
    #[serde(rename = "vscode")]
    VSCode,
}

impl Default for OpenIn {
    fn default() -> Self {
        Self::Vim
    }
}

impl OpenIn {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Vim => "vim",
            Self::VSCode => "vscode",
        }
    }
}

// ── Config ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Which external editor to open files in when the user presses Ctrl+o.
    #[serde(default)]
    pub open_in: OpenIn,

    /// Minimum log level written to `mastdiff.log`.
    /// One of: off, error, warn, info, debug, trace  (default: info)
    #[serde(default)]
    pub log_level: LogLevel,

    /// Maximum source bytes (in MiB) kept in the in-memory AST parse cache.
    /// Set to 0 to disable caching entirely.  Default: 128.
    #[serde(default = "default_ast_cache_mb")]
    pub ast_cache_mb: u64,

    /// Hard size cap (in MiB) for the persistent, content-addressed symbol
    /// cache on disk (`$XDG_CACHE_HOME/mastdiff/`). Precomputed shorthand-query
    /// captures are stored there so cold searches skip re-parsing across
    /// process restarts. When a write crosses the cap the oldest entries are
    /// evicted back under it, so the directory can't grow without bound.
    /// Set to 0 to disable the persistent cache entirely.  Default: 256.
    #[serde(default = "default_sym_cache_mb")]
    pub sym_cache_mb: u64,
}

fn default_ast_cache_mb() -> u64 { 128 }
fn default_sym_cache_mb() -> u64 { 256 }

impl Default for Config {
    fn default() -> Self {
        Self {
            open_in: OpenIn::default(),
            log_level: LogLevel::default(),
            ast_cache_mb: default_ast_cache_mb(),
            sym_cache_mb: default_sym_cache_mb(),
        }
    }
}

impl Config {
    /// Load from `~/.config/mastdiff/config.toml` (XDG-aware).
    /// Falls back to [`Config::default`] on any error.
    pub fn load() -> Self {
        let path = Self::path();
        log::debug!("loading config from {:?}", path);
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                log::debug!("config file not found or unreadable: {}", e);
                return Self::default();
            }
        };
        match toml::from_str::<Config>(&raw) {
            Ok(cfg) => {
                log::info!(
                    "config loaded: open_in={} log_level={}",
                    cfg.open_in.label(),
                    cfg.log_level.label()
                );
                cfg
            }
            Err(e) => {
                log::warn!("config parse error (using defaults): {}", e);
                Self::default()
            }
        }
    }

    /// Canonical path: `$XDG_CONFIG_HOME/mastdiff/config.toml`
    /// or `~/.config/mastdiff/config.toml` if XDG is unset.
    pub fn path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(".config"))
                    .unwrap_or_else(|_| PathBuf::from("."))
            });
        base.join("mastdiff").join("config.toml")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_open_in_is_vim() {
        let cfg = Config::default();
        assert_eq!(cfg.open_in, OpenIn::Vim);
    }

    #[test]
    fn parse_open_in_vim() {
        let cfg: Config = toml::from_str("open_in = \"vim\"").unwrap();
        assert_eq!(cfg.open_in, OpenIn::Vim);
    }

    #[test]
    fn parse_open_in_vscode() {
        let cfg: Config = toml::from_str("open_in = \"vscode\"").unwrap();
        assert_eq!(cfg.open_in, OpenIn::VSCode);
    }

    #[test]
    fn parse_empty_toml_uses_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.open_in, OpenIn::Vim);
    }

    #[test]
    fn label_vim() {
        assert_eq!(OpenIn::Vim.label(), "vim");
    }

    #[test]
    fn label_vscode() {
        assert_eq!(OpenIn::VSCode.label(), "vscode");
    }

    #[test]
    fn config_path_ends_with_expected_suffix() {
        let p = Config::path();
        assert!(p.ends_with("mastdiff/config.toml"));
    }

    #[test]
    fn parse_log_level_debug() {
        let cfg: Config = toml::from_str("log_level = \"debug\"").unwrap();
        assert_eq!(cfg.log_level, LogLevel::Debug);
    }

    #[test]
    fn parse_log_level_trace() {
        let cfg: Config = toml::from_str("log_level = \"trace\"").unwrap();
        assert_eq!(cfg.log_level, LogLevel::Trace);
    }

    #[test]
    fn parse_log_level_off() {
        let cfg: Config = toml::from_str("log_level = \"off\"").unwrap();
        assert_eq!(cfg.log_level, LogLevel::Off);
    }

    #[test]
    fn default_log_level_is_info() {
        let cfg = Config::default();
        assert_eq!(cfg.log_level, LogLevel::Info);
    }

    #[test]
    fn log_level_to_level_filter() {
        assert_eq!(LogLevel::Debug.to_level_filter(), log::LevelFilter::Debug);
        assert_eq!(LogLevel::Off.to_level_filter(),   log::LevelFilter::Off);
        assert_eq!(LogLevel::Trace.to_level_filter(), log::LevelFilter::Trace);
    }

    #[test]
    fn default_ast_cache_mb_is_128() {
        assert_eq!(Config::default().ast_cache_mb, 128);
    }

    #[test]
    fn parse_ast_cache_mb() {
        let cfg: Config = toml::from_str("ast_cache_mb = 256").unwrap();
        assert_eq!(cfg.ast_cache_mb, 256);
    }

    #[test]
    fn ast_cache_mb_zero_disables() {
        let cfg: Config = toml::from_str("ast_cache_mb = 0").unwrap();
        assert_eq!(cfg.ast_cache_mb, 0);
    }

    #[test]
    fn default_sym_cache_mb_is_256() {
        assert_eq!(Config::default().sym_cache_mb, 256);
    }

    #[test]
    fn parse_sym_cache_mb() {
        let cfg: Config = toml::from_str("sym_cache_mb = 512").unwrap();
        assert_eq!(cfg.sym_cache_mb, 512);
    }

    #[test]
    fn sym_cache_mb_zero_disables() {
        let cfg: Config = toml::from_str("sym_cache_mb = 0").unwrap();
        assert_eq!(cfg.sym_cache_mb, 0);
    }
}
