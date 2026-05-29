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
        load_from_compile_commands(&cc)
    } else {
        load_from_walk(dir)
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
        Some("cpp" | "cc" | "cxx" | "C")
    )
}
