//! What the Leaf viewer and editor share, so that the editor reads pages
//! exactly as the index does and neither crate carries a copy of the other's
//! rules.
//!
//! * [`parse`]: the read-only page parser (passages, headings, byte ranges,
//!   frontmatter, published pages, citation checks).
//! * [`markdown`]: the extraction behind [`parse`]. Changing what it extracts
//!   requires bumping `PARSER_VERSION` in `leaf-viewer`, or unchanged
//!   pages keep stale index rows.
//! * [`facet`]: frontmatter facet keys and values.
//! * [`mod@reference`], [`citation`]: the `raw/…` citation syntax and reading the
//!   cited part of a source.
//! * [`publication`]: the `index.md` allowlist and safe page paths.
//! * [`dictionary`]: the built-in synonym dictionary.
//!
//! Nothing here writes to a wiki, and no part of it needs a database.

pub mod citation;
pub mod dictionary;
pub mod facet;
pub mod markdown;
mod model;
pub mod parse;
pub mod publication;
pub mod reference;

pub use model::{Anchor, Citation, CitationExcerpt, Error, Result, SyncIssue};
