// Expose pure-logic modules for integration testing.
// UI and app modules (which depend on ratatui/crossterm) are not re-exported.
pub mod tracer;
pub mod ast_cache;
pub mod ast_diff;
pub mod config;
pub mod daemon;
pub mod input;
pub mod project;
pub mod search;
pub mod text_diff;
