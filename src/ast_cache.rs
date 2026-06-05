use indexmap::IndexMap;
use std::{path::PathBuf, time::SystemTime};
use tree_sitter::Tree;

pub struct AstCache {
    entries: IndexMap<PathBuf, Entry>,
    total_source_bytes: usize,
    cap_source_bytes: usize,
}

struct Entry {
    mtime: SystemTime,
    tree: Tree,
    source_len: usize,
}

impl AstCache {
    pub fn new(cap_mb: u64) -> Self {
        Self {
            entries: IndexMap::new(),
            total_source_bytes: 0,
            cap_source_bytes: (cap_mb as usize).saturating_mul(1024 * 1024),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.cap_source_bytes > 0
    }

    /// Returns a cloned Tree if `path` is cached and `current_mtime` matches.
    /// Removes stale entries (mtime changed) as a side effect.
    pub fn get(&mut self, path: &PathBuf, current_mtime: SystemTime) -> Option<Tree> {
        match self.entries.get(path) {
            Some(e) if e.mtime == current_mtime => Some(e.tree.clone()),
            Some(_) => {
                self.remove(path);
                None
            }
            None => None,
        }
    }

    pub fn insert(&mut self, path: PathBuf, mtime: SystemTime, tree: Tree, source_len: usize) {
        self.remove(&path);
        self.entries.insert(path, Entry { mtime, tree, source_len });
        self.total_source_bytes += source_len;
        while self.total_source_bytes > self.cap_source_bytes {
            if let Some((_, evicted)) = self.entries.shift_remove_index(0) {
                self.total_source_bytes -= evicted.source_len;
            } else {
                break;
            }
        }
    }

    fn remove(&mut self, path: &PathBuf) {
        if let Some(old) = self.entries.swap_remove(path) {
            self.total_source_bytes -= old.source_len;
        }
    }
}
