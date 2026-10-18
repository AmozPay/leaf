//! Test support shared by Leaf's tests (a development dependency only).
//!
//! Tests use temporary wikis, never a real one: [`TempWiki`] is a temporary
//! directory with a `wiki/` folder to fill and room beside it for the cache
//! database, and [`snapshot`] records every file's bytes so a test can prove
//! that a rejected operation changed nothing.

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A temporary directory holding a wiki (`wiki/`) and, beside it, the files
/// that must stay outside the wiki (the cache database).
pub struct TempWiki {
    dir: TempDir,
}

impl TempWiki {
    /// An empty one: the wiki root does not exist until a file is written.
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        }
    }

    /// The wiki root.
    pub fn root(&self) -> PathBuf {
        self.dir.path().join("wiki")
    }

    /// A cache database path, outside the wiki.
    pub fn db(&self) -> PathBuf {
        self.path("cache.db")
    }

    /// A path beside the wiki (not inside it).
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Write a file of the wiki, creating its folders.
    pub fn write(&self, relative: &str, content: &str) {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().expect("a file path")).expect("create folders");
        fs::write(path, content).expect("write a file");
    }

    /// Read a file of the wiki as text.
    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root().join(relative)).expect("read a file")
    }
}

impl Default for TempWiki {
    fn default() -> Self {
        Self::new()
    }
}

/// Every file under `root` with its bytes, sorted by path.
pub fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).expect("read a folder") {
            let path = entry.expect("a folder entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = fs::read(&path).expect("read a file");
                files.push((path, bytes));
            }
        }
    }
    files.sort();
    files
}
