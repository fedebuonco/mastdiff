// Expose pure-logic modules for integration testing.
// UI and app modules (which depend on ratatui/crossterm) are not re-exported.
pub mod ast_diff;
pub mod config;
pub mod input;
pub mod project;
pub mod search;
pub mod text_diff;
