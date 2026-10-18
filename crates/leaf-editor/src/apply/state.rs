//! What a batch in progress knows, grouped by concern: the pages it has
//! loaded or created, the pages it archives and restores, the raw files it
//! adds, `index.md`, and what it declares.

use super::Staged;
use super::archive_ops::Moved;
use crate::check::StagedPage;
use crate::index_md::IndexDoc;
use crate::vocabulary::Extension;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The pages of the batch.
pub(super) struct PageSet {
    /// Pages loaded into the plan (and the ones it creates), by path.
    pub loaded: BTreeMap<String, Staged>,
    /// The loaded pages as the checks see them (see `refresh_staged`).
    pub checks: Vec<StagedPage>,
    /// The published pages as the batch leaves it.
    pub published: BTreeSet<String>,
    /// The published pages before the batch.
    pub initial_published: BTreeSet<String>,
    /// Pages the batch creates.
    pub created: Vec<String>,
    /// Pages the batch creates or restores: checked as new pages, so every
    /// problem they have is the batch's.
    pub fresh: BTreeSet<String>,
    /// Paths that stop being published (archived pages).
    pub gone: Vec<String>,
    /// New entity pages that declare themselves people (a `relationship`
    /// map, or their own slug in `related-people`) before they have it.
    pub person_hints: BTreeSet<String>,
}

impl PageSet {
    pub fn new(published: BTreeSet<String>) -> Self {
        Self {
            loaded: BTreeMap::new(),
            checks: Vec::new(),
            initial_published: published.clone(),
            published,
            created: Vec::new(),
            fresh: BTreeSet::new(),
            gone: Vec::new(),
            person_hints: BTreeSet::new(),
        }
    }
}

/// Pages archived and restored.
#[derive(Default)]
pub(super) struct Archive {
    /// Pages moved to `_archive/`.
    pub moved: Vec<Moved>,
    /// Archived pages moved back: (archive path, original path, file).
    pub restored: Vec<(String, String, PathBuf)>,
}

/// Raw files the batch adds.
#[derive(Default)]
pub(super) struct RawFiles {
    pub content: BTreeMap<String, Vec<u8>>,
    /// The paths, in the order they were added.
    pub paths: Vec<String>,
}

/// `index.md`, once an operation needs it.
#[derive(Default)]
pub(super) struct IndexState {
    pub doc: Option<IndexDoc>,
    pub changed: bool,
}

/// What the batch adds to the wiki's vocabulary.
pub(super) struct Declarations {
    pub extension: Extension,
    pub changed: bool,
    /// What was declared, for `log.md`.
    pub log: Vec<String>,
}

impl Declarations {
    pub fn new(extension: Extension) -> Self {
        Self {
            extension,
            changed: false,
            log: Vec::new(),
        }
    }
}
