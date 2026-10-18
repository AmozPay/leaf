//! Validated edits to a curated Markdown wiki (in progress; see
//! `docs/EDITOR-SPEC.md`): the only write path for agents. Batches of
//! operations are checked against the wiki's vocabulary (Leaf's baseline
//! plus the wiki's extension) and applied all at once or not at all.

mod apply;
pub mod batch;
mod check;
mod commit;
mod conformance;
mod editor;
mod error;
mod frontmatter;
mod index_md;
mod init;
mod pages;
mod passages;
mod suggest;
pub mod vocabulary;

pub use batch::{
    Batch, BatchKind, BatchOutcome, NewPage, NewPassage, Op, OpError, PassageId, Rewrite, Section,
};
pub use conformance::{ConformanceReport, Issue};
pub use editor::{Editor, EditorConfig, MetadataKey, MetadataValue, PageView, PassageView};
pub use error::{Error, Result};
pub use init::{InitOptions, init_wiki};
pub use suggest::{Lookup, Reason, Suggestion};
pub use vocabulary::{Extension, Vocabulary};
