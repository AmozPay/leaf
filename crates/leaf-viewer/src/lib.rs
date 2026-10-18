//! Local, citation-preserving search over published Markdown wiki pages.
//!
//! Leaf indexes the pages linked from a wiki's `index.md` (under
//! `entities/`, `concepts/`, `comparisons/`, `queries/`) into a disposable
//! SQLite FTS5 cache and returns small, whole-paragraph passages with their
//! `[[raw/…]]` citations: a whole source of any type, or one transcript
//! message, Obsidian block or heading section in it. It never writes to the
//! wiki. Raw sources are read only through [`WikiIndex::read_citation`], for
//! a citation found on an indexed passage.
//!
//! This is the read-only part of Leaf: depend on it alone to search.
//!
//! ```
//! use leaf_viewer::{SearchRequest, WikiConfig, WikiIndex};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let tmp = tempfile::tempdir()?;
//! # let wiki = tmp.path().join("wiki");
//! # std::fs::create_dir_all(wiki.join("concepts"))?;
//! # std::fs::create_dir_all(wiki.join("raw/chatgpt"))?;
//! # std::fs::write(wiki.join("index.md"), "- [[concepts/rivers|Rivers]]\n")?;
//! # std::fs::write(wiki.join("concepts/rivers.md"),
//! #     "---\ntitle: Rivers\n---\n# Rivers\n\nThe delta floods each spring. [[raw/chatgpt/a.md#^message-m1|Report]]\n")?;
//! # std::fs::write(wiki.join("raw/chatgpt/a.md"), "### user · 2025\n\nIt floods every spring.\n\n^message-m1\n")?;
//! # let db = tmp.path().join("leaf.db");
//! let mut index = WikiIndex::open(WikiConfig::new(&wiki, &db))?;
//! let report = index.sync()?;
//! assert!(report.is_complete());
//!
//! let response = index.search(&SearchRequest::new("when does the delta flood?", 5, 2_000))?;
//! let hit = &response.hits[0];
//! assert_eq!(hit.text, "The delta floods each spring.");
//! assert!(response.chars_used <= 2_000);
//!
//! let source = index.read_citation(&hit.citations[0], 500)?;
//! assert!(source.text.unwrap().contains("It floods every spring."));
//! # Ok(())
//! # }
//! ```

mod config;
mod model;
mod people;
mod search;
mod store;
mod wiki;

// Shared with the editor through `leaf-common`; re-exported so that
// readers depend on this crate alone.
pub use config::{WikiConfig, default_database_path, default_wiki_root};
pub use leaf_common::{Anchor, Citation, CitationExcerpt, SyncIssue, dictionary, parse};
pub use model::{
    DatabaseError, Error, FacetCount, FacetFilter, FacetMatch, IndexStats, PageRef, Person, Result,
    SearchHit, SearchRequest, SearchResponse, SyncReport,
};
pub use wiki::WikiIndex;
