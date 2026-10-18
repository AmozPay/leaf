//! Writing a validated batch: all files or none.
//!
//! One batch at a time holds an exclusive lock
//! (`_meta/.leaf-editor/lock`). Before writing, the previous content of
//! every target is backed up and a journal listing the targets is saved.
//! Each file is then written to a temporary file in its own directory,
//! synced, and renamed into place. If any step fails, the files already
//! replaced are restored from the backups. A journal left behind by a crash
//! is rolled back the next time the wiki is opened.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The editor's own state, inside the wiki but outside published content.
const STATE_DIR: &str = "_meta/.leaf-editor";

/// One file to write, or to remove.
pub(crate) struct Write {
    /// Absolute path of the file (pages: their resolved file).
    pub path: PathBuf,
    /// The new content; `None` removes the file (its backup keeps it
    /// recoverable until the batch completes).
    pub content: Option<Vec<u8>>,
}

impl Write {
    pub fn file(path: PathBuf, content: Vec<u8>) -> Self {
        Self {
            path,
            content: Some(content),
        }
    }

    pub fn remove(path: PathBuf) -> Self {
        Self {
            path,
            content: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Journal {
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    /// The backup of the previous content; `None` when the file is new.
    backup: Option<PathBuf>,
}

/// Write every file, or none of them.
pub(crate) fn commit(root: &Path, writes: &[Write]) -> Result<()> {
    commit_failing_after(root, writes, None)
}

/// [`commit`], failing on purpose before write number `fail_after` (tests).
pub(crate) fn commit_failing_after(
    root: &Path,
    writes: &[Write],
    fail_after: Option<usize>,
) -> Result<()> {
    let state = root.join(STATE_DIR);
    fs::create_dir_all(&state).map_err(|err| Error::io(&state, err))?;
    let _lock = lock(&state)?;
    roll_back(&state)?;

    let backups = state.join("backup");
    fs::create_dir_all(&backups).map_err(|err| Error::io(&backups, err))?;
    let mut entries = Vec::new();
    for (number, write) in writes.iter().enumerate() {
        let backup = if write.path.exists() {
            let backup = backups.join(number.to_string());
            fs::copy(&write.path, &backup).map_err(|err| Error::io(&write.path, err))?;
            Some(backup)
        } else {
            None
        };
        entries.push(Entry {
            path: write.path.clone(),
            backup,
        });
    }
    save_journal(&state, &Journal { entries })?;

    for (number, write) in writes.iter().enumerate() {
        let result = if fail_after == Some(number) {
            Err(Error::io(
                &write.path,
                std::io::Error::other("simulated failure"),
            ))
        } else {
            match &write.content {
                Some(content) => replace(&write.path, content),
                None => remove(&write.path),
            }
        };
        if let Err(err) = result {
            roll_back(&state)?;
            return Err(err);
        }
    }
    finish(&state)
}

/// Roll back a batch interrupted by a crash, if one was.
pub(crate) fn recover(root: &Path) -> Result<bool> {
    let state = root.join(STATE_DIR);
    if !state.join("journal.json").exists() {
        return Ok(false);
    }
    let _lock = lock(&state)?;
    roll_back(&state)
}

fn lock(state: &Path) -> Result<File> {
    let path = state.join("lock");
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|err| Error::io(&path, err))?;
    file.lock().map_err(|err| Error::io(&path, err))?;
    Ok(file)
}

fn save_journal(state: &Path, journal: &Journal) -> Result<()> {
    let path = state.join("journal.json");
    let text = serde_json::to_vec(journal).map_err(|err| Error::io(&path, err.into()))?;
    replace(&path, &text)
}

/// Restore every journaled file to its backup (or remove it if it was new),
/// then clear the journal. Returns whether there was anything to undo.
fn roll_back(state: &Path) -> Result<bool> {
    let path = state.join("journal.json");
    let text = match fs::read(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(Error::io(&path, err)),
    };
    let journal: Journal =
        serde_json::from_slice(&text).map_err(|err| Error::io(&path, err.into()))?;
    for entry in journal.entries {
        match entry.backup {
            Some(backup) => {
                let content = fs::read(&backup).map_err(|err| Error::io(&backup, err))?;
                replace(&entry.path, &content)?;
            }
            None => match fs::remove_file(&entry.path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(Error::io(&entry.path, err)),
            },
        }
    }
    finish(state)?;
    Ok(true)
}

fn finish(state: &Path) -> Result<()> {
    let journal = state.join("journal.json");
    match fs::remove_file(&journal) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::io(&journal, err)),
    }
    let backups = state.join("backup");
    if backups.exists() {
        fs::remove_dir_all(&backups).map_err(|err| Error::io(&backups, err))?;
    }
    Ok(())
}

fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(Error::io(path, err)),
    }
}

/// Write `content` to `path` through a synced temporary file and a rename.
fn replace(path: &Path, content: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|err| Error::io(dir, err))?;
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|err| Error::io(dir, err))?;
    temp.write_all(content)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|err| Error::io(path, err))?;
    temp.persist(path)
        .map_err(|err| Error::io(path, err.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: PathBuf, content: &str) -> Write {
        Write::file(path, content.as_bytes().to_vec())
    }

    #[test]
    fn all_files_are_written_and_state_is_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.md"), "old a").unwrap();
        commit(
            root,
            &[
                write(root.join("a.md"), "new a"),
                write(root.join("b/c.md"), "new c"),
            ],
        )
        .unwrap();
        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "new a");
        assert_eq!(fs::read_to_string(root.join("b/c.md")).unwrap(), "new c");
        assert!(!root.join(STATE_DIR).join("journal.json").exists());
        assert!(!root.join(STATE_DIR).join("backup").exists());
    }

    #[test]
    fn a_failure_midway_restores_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.md"), "old a").unwrap();
        fs::write(root.join("b.md"), "old b").unwrap();
        let writes = [
            write(root.join("a.md"), "new a"),
            write(root.join("new.md"), "new file"),
            write(root.join("b.md"), "new b"),
        ];
        assert!(commit_failing_after(root, &writes, Some(2)).is_err());
        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "old a");
        assert_eq!(fs::read_to_string(root.join("b.md")).unwrap(), "old b");
        assert!(!root.join("new.md").exists());
        assert!(!root.join(STATE_DIR).join("journal.json").exists());
    }

    #[test]
    fn a_removed_file_is_restored_when_the_batch_fails() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.md"), "old a").unwrap();
        fs::write(root.join("b.md"), "old b").unwrap();
        let writes = [
            write(root.join("moved/a.md"), "old a"),
            Write::remove(root.join("a.md")),
            write(root.join("b.md"), "new b"),
        ];
        assert!(commit_failing_after(root, &writes, Some(2)).is_err());
        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "old a");
        assert_eq!(fs::read_to_string(root.join("b.md")).unwrap(), "old b");
        assert!(!root.join("moved/a.md").exists());
        // And when it succeeds, the file is gone and the copy is in place.
        commit(root, &writes[..2]).unwrap();
        assert!(!root.join("a.md").exists());
        assert_eq!(
            fs::read_to_string(root.join("moved/a.md")).unwrap(),
            "old a"
        );
    }

    #[test]
    fn a_crashed_batch_is_rolled_back_on_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(
            !recover(root).unwrap(),
            "nothing to recover, nothing created"
        );
        assert!(!root.join(STATE_DIR).exists());
        fs::write(root.join("a.md"), "old a").unwrap();
        // Simulate a crash after the journal was saved and one file replaced.
        let state = root.join(STATE_DIR);
        fs::create_dir_all(state.join("backup")).unwrap();
        fs::copy(root.join("a.md"), state.join("backup/0")).unwrap();
        save_journal(
            &state,
            &Journal {
                entries: vec![
                    Entry {
                        path: root.join("a.md"),
                        backup: Some(state.join("backup/0")),
                    },
                    Entry {
                        path: root.join("created.md"),
                        backup: None,
                    },
                ],
            },
        )
        .unwrap();
        fs::write(root.join("a.md"), "half-written").unwrap();
        fs::write(root.join("created.md"), "new").unwrap();
        assert!(recover(root).unwrap());
        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "old a");
        assert!(!root.join("created.md").exists());
        assert!(!recover(root).unwrap());
    }
}
