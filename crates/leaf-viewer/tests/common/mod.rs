#![allow(dead_code)]

use leaf_viewer::{WikiConfig, WikiIndex};
use std::fs;
use std::path::{Path, PathBuf};

/// A temporary wiki plus a database path outside it.
pub struct Fixture {
    _tmp: tempfile::TempDir,
    pub base: PathBuf,
    pub root: PathBuf,
    pub db: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        let root = base.join("wiki");
        for dir in [
            "entities",
            "concepts",
            "comparisons",
            "queries",
            "raw/chatgpt",
            "_meta",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let db = base.join("index.db");
        Self {
            _tmp: tmp,
            base,
            root,
            db,
        }
    }

    pub fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    pub fn config(&self) -> WikiConfig {
        WikiConfig::new(&self.root, &self.db)
    }

    pub fn open(&self) -> WikiIndex {
        WikiIndex::open(self.config()).unwrap()
    }

    pub fn synced(&self) -> WikiIndex {
        let mut index = self.open();
        let report = index.sync().unwrap();
        assert!(report.is_complete(), "unexpected sync errors: {report:?}");
        index
    }

    /// Count rows in an internal table, to prove FTS and structured rows agree.
    pub fn count(&self, table: &str) -> i64 {
        let db = rusqlite::Connection::open(&self.db).unwrap();
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

pub fn page(title: &str, body: &str) -> String {
    format!("---\ntitle: {title}\nconfidence: medium\n---\n# {title}\n\n{body}\n")
}

pub fn cite(file: &str, id: &str, label: &str) -> String {
    format!("[[raw/chatgpt/{file}#^message-{id}|{label}]]")
}

/// Snapshot every file under a directory, to prove it was not modified.
pub fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}
