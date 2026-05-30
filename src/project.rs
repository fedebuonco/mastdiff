//! Project discovery: finds C++ translation units for the project browser.
//!
//! [`load`] prefers a `compile_commands.json` database (checked in several
//! common build-output directories) and falls back to a recursive directory
//! walk collecting `.cpp`, `.cc`, `.cxx`, `.C`, `.h`, `.hpp`, `.hxx`, and `.H` files.
//!
//! The resulting [`TranslationUnit`] slice is always sorted by path and
//! deduplicated, so callers can rely on a stable, canonical ordering.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct TranslationUnit {
    pub file_path: String,
    pub file_size: u64,
}

impl TranslationUnit {
    pub fn short_name(&self) -> &str {
        Path::new(&self.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.file_path)
    }

    pub fn size_label(&self) -> String {
        if self.file_size < 1024 {
            format!("{}B", self.file_size)
        } else if self.file_size < 1024 * 1024 {
            format!("{:.1}KB", self.file_size as f64 / 1024.0)
        } else {
            format!("{:.1}MB", self.file_size as f64 / (1024.0 * 1024.0))
        }
    }
}

#[derive(Deserialize)]
struct CompileEntry {
    file: String,
}

/// Load all C++ translation units from a project directory.
/// Prefers `compile_commands.json` if present, otherwise walks for `.cpp`/`.cc`/`.cxx`.
pub fn load(dir: &Path) -> Result<Vec<TranslationUnit>> {
    if let Some(cc) = find_compile_commands(dir) {
        log::info!("project loader: compile_commands.json at {:?}", cc);
        let tus = load_from_compile_commands(&cc)?;
        log::info!("project loader: {} translation units loaded", tus.len());
        for tu in &tus {
            log::trace!("  tu: {} ({})", tu.file_path, tu.size_label());
        }
        Ok(tus)
    } else {
        log::info!("project loader: directory walk of {:?}", dir);
        let tus = load_from_walk(dir)?;
        log::info!("project loader: {} translation units found", tus.len());
        for tu in &tus {
            log::trace!("  tu: {} ({})", tu.file_path, tu.size_label());
        }
        Ok(tus)
    }
}

fn find_compile_commands(dir: &Path) -> Option<PathBuf> {
    let candidates = [
        dir.join("compile_commands.json"),
        dir.join("build").join("compile_commands.json"),
        dir.join("cmake-build-debug").join("compile_commands.json"),
        dir.join("cmake-build-release").join("compile_commands.json"),
        dir.join("out").join("compile_commands.json"),
        dir.join("_build").join("compile_commands.json"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn load_from_compile_commands(path: &Path) -> Result<Vec<TranslationUnit>> {
    let raw = fs::read_to_string(path)?;
    let entries: Vec<CompileEntry> = serde_json::from_str(&raw)?;

    let mut tus: Vec<TranslationUnit> = entries
        .into_iter()
        .filter(|e| is_cpp_source(&e.file))
        .map(|e| {
            let file_size = fs::metadata(&e.file).map(|m| m.len()).unwrap_or(0);
            TranslationUnit { file_path: e.file, file_size }
        })
        .filter(|tu| Path::new(&tu.file_path).exists())
        .collect();

    tus.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    tus.dedup_by(|a, b| a.file_path == b.file_path);
    Ok(tus)
}

fn load_from_walk(dir: &Path) -> Result<Vec<TranslationUnit>> {
    let mut tus: Vec<TranslationUnit> = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_cpp_source(&e.path().to_string_lossy()))
        .map(|e| {
            let file_size = e.metadata().map(|m| m.len()).unwrap_or(0);
            TranslationUnit {
                file_path: e.path().to_string_lossy().into_owned(),
                file_size,
            }
        })
        .collect();

    tus.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    Ok(tus)
}

pub fn is_cpp_source(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|e| e.to_str()),
        Some("cpp" | "cc" | "cxx" | "C" | "h" | "hpp" | "hxx" | "H")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_cpp_source ─────────────────────────────────────────────────────

    #[test]
    fn cpp_extensions_accepted() {
        assert!(is_cpp_source("foo.cpp"));
        assert!(is_cpp_source("foo.cc"));
        assert!(is_cpp_source("foo.cxx"));
        assert!(is_cpp_source("foo.C"));
        assert!(is_cpp_source("foo.h"));
        assert!(is_cpp_source("foo.hpp"));
        assert!(is_cpp_source("foo.hxx"));
        assert!(is_cpp_source("foo.H"));
    }

    #[test]
    fn non_cpp_extensions_rejected() {
        assert!(!is_cpp_source("foo.c"));
        assert!(!is_cpp_source("foo.rs"));
        assert!(!is_cpp_source("foo.py"));
        assert!(!is_cpp_source("foo"));
    }

    #[test]
    fn extension_check_is_case_sensitive() {
        // Only uppercase C is accepted; lowercase c is not
        assert!(!is_cpp_source("foo.c"));
        assert!(is_cpp_source("foo.C"));
    }

    // ── TranslationUnit helpers ───────────────────────────────────────────

    fn tu(path: &str, size: u64) -> TranslationUnit {
        TranslationUnit { file_path: path.to_string(), file_size: size }
    }

    #[test]
    fn short_name_extracts_filename() {
        assert_eq!(tu("/a/b/foo.cpp", 0).short_name(), "foo.cpp");
        assert_eq!(tu("bar.cpp", 0).short_name(), "bar.cpp");
    }

    #[test]
    fn size_label_bytes() {
        assert_eq!(tu("f.cpp", 500).size_label(), "500B");
    }

    #[test]
    fn size_label_kilobytes() {
        assert_eq!(tu("f.cpp", 2048).size_label(), "2.0KB");
    }

    #[test]
    fn size_label_megabytes() {
        assert_eq!(tu("f.cpp", 2 * 1024 * 1024).size_label(), "2.0MB");
    }

    #[test]
    fn size_label_zero() {
        assert_eq!(tu("f.cpp", 0).size_label(), "0B");
    }
}
