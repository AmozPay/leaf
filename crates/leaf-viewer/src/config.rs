//! Wiki and cache locations, validated before anything touches the disk.

use crate::model::{Error, Result};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Where the wiki lives and where the disposable cache is kept.
#[derive(Debug, Clone)]
pub struct WikiConfig {
    /// Root of the Markdown wiki (contains `index.md` and `raw/`). Never written.
    pub wiki_root: PathBuf,
    /// SQLite cache file. Its directory must already exist and must not be
    /// inside `raw/`, directly or through symlinks.
    pub database_path: PathBuf,
    /// Drop and recreate a Leaf cache whose schema version differs.
    /// Databases not created by Leaf are never modified.
    pub rebuild_incompatible_cache: bool,
}

impl WikiConfig {
    pub fn new(wiki_root: impl Into<PathBuf>, database_path: impl Into<PathBuf>) -> Self {
        Self {
            wiki_root: wiki_root.into(),
            database_path: database_path.into(),
            rebuild_incompatible_cache: false,
        }
    }

    /// Opt in to rebuilding an incompatible Leaf cache.
    pub fn rebuild_incompatible_cache(mut self, rebuild: bool) -> Self {
        self.rebuild_incompatible_cache = rebuild;
        self
    }
}

/// The wiki root used when none is given: `~/wiki`.
pub fn default_wiki_root() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join("wiki"))
        .ok_or_else(|| Error::InvalidConfig("cannot determine the home directory".into()))
}

/// The cache file used when none is given: `leaf/leaf.db` in the
/// user's cache directory, which is created if needed.
pub fn default_database_path() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| Error::InvalidConfig("cannot determine the cache directory".into()))?
        .join("leaf");
    fs::create_dir_all(&dir).map_err(|err| Error::io(&dir, err))?;
    Ok(dir.join("leaf.db"))
}

/// Canonical paths derived from a [`WikiConfig`].
#[derive(Debug)]
pub(crate) struct ResolvedPaths {
    pub root: PathBuf,
    pub database: PathBuf,
}

pub(crate) fn resolve(config: &WikiConfig) -> Result<ResolvedPaths> {
    let root = fs::canonicalize(&config.wiki_root)
        .ok()
        .filter(|root| root.is_dir())
        .ok_or_else(|| {
            invalid(format!(
                "wiki root {} is not an existing directory",
                config.wiki_root.display()
            ))
        })?;
    // Compare against both the lexical and the resolved raw/, in case raw/ is a symlink.
    let mut raw_dirs = vec![root.join("raw")];
    if let Ok(canonical) = fs::canonicalize(root.join("raw")) {
        raw_dirs.push(canonical);
    }
    let inside_raw = |path: &Path| raw_dirs.iter().any(|raw| path.starts_with(raw));

    let requested = &config.database_path;
    let file_name = requested
        .file_name()
        .ok_or_else(|| invalid(format!("{} is not a file path", requested.display())))?;
    let parent = match requested.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    // Never create directories: a missing parent could be inside raw/.
    let parent = fs::canonicalize(parent)
        .ok()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| {
            invalid(format!(
                "database directory {} does not exist",
                parent.display()
            ))
        })?;
    let mut database = parent.join(file_name);
    if inside_raw(&database) {
        return Err(invalid("the database may not be inside raw/"));
    }
    match fs::symlink_metadata(&database) {
        Ok(meta) if meta.file_type().is_symlink() => {
            // A dangling link would make SQLite create its target, wherever it is.
            let target = fs::canonicalize(&database)
                .map_err(|_| invalid("the database path is a dangling symlink"))?;
            if inside_raw(&target) {
                return Err(invalid("the database symlink points inside raw/"));
            }
            database = target;
        }
        Ok(meta) if !meta.is_file() => {
            return Err(invalid("the database path is not a regular file"));
        }
        Ok(_) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(Error::io(database, err)),
    }
    if !database.is_file() && database.exists() {
        return Err(invalid("the database path is not a regular file"));
    }
    Ok(ResolvedPaths { root, database })
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidConfig(message.into())
}
