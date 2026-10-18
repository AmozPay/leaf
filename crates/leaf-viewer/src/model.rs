//! Public requests, results, and errors.

pub(crate) use leaf_common::{Anchor, Citation, CitationExcerpt, SyncIssue};
use std::fmt;
use std::path::PathBuf;

/// Result type returned by every fallible Leaf operation.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by Leaf. Messages never contain raw transcript text.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The wiki root or database location is unusable or unsafe.
    InvalidConfig(String),
    /// A caller-supplied argument is out of range.
    InvalidRequest(String),
    /// Reading a file failed (for example a missing `index.md` or raw source).
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The database exists but was not created by Leaf; it is left untouched.
    ForeignDatabase(PathBuf),
    /// A Leaf cache with another schema version; reopen with
    /// [`WikiConfig::rebuild_incompatible_cache`](crate::WikiConfig::rebuild_incompatible_cache)
    /// to rebuild it.
    IncompatibleSchema { found: i64, expected: i64 },
    /// SQLite reported an error.
    Database(DatabaseError),
    /// A citation path is malformed or resolves outside `raw/`.
    UnsafePath(String),
    /// The cited source cannot serve this citation, e.g. an anchor into a
    /// file that is not text.
    UnsupportedSource(String),
    /// The citation is not attached to any indexed passage.
    CitationNotIndexed,
    /// The cited anchor (message, block or heading) does not exist in the source.
    AnchorNotFound,
    /// The cited anchor occurs more than once, so the cited part is ambiguous.
    AmbiguousAnchor,
}

/// Opaque SQLite failure; keeps the storage engine out of the public API.
#[derive(Debug)]
pub struct DatabaseError(rusqlite::Error);

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for DatabaseError {}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(DatabaseError(err))
    }
}

impl From<leaf_common::Error> for Error {
    fn from(err: leaf_common::Error) -> Self {
        use leaf_common::Error as Common;
        match err {
            Common::InvalidReference(message) => Self::InvalidRequest(message),
            Common::Io { path, source } => Self::Io { path, source },
            Common::UnsafePath(message) => Self::UnsafePath(message),
            Common::UnsupportedSource(message) => Self::UnsupportedSource(message),
            Common::AnchorNotFound => Self::AnchorNotFound,
            Common::AmbiguousAnchor => Self::AmbiguousAnchor,
            // `Error` is non-exhaustive: a future variant needs a decision here.
            other => Self::InvalidRequest(other.to_string()),
        }
    }
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(f, "invalid configuration: {message}"),
            Self::InvalidRequest(message) => write!(f, "invalid request: {message}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::ForeignDatabase(path) => {
                write!(
                    f,
                    "{} is not a Leaf cache; refusing to modify it",
                    path.display()
                )
            }
            Self::IncompatibleSchema { found, expected } => write!(
                f,
                "cache schema version {found} is incompatible with {expected}; rebuild it explicitly"
            ),
            Self::Database(err) => write!(f, "database error: {err}"),
            Self::UnsafePath(message) => write!(f, "unsafe citation path: {message}"),
            Self::UnsupportedSource(message) => write!(f, "unsupported source: {message}"),
            Self::CitationNotIndexed => {
                f.write_str("citation is not referenced by any indexed passage")
            }
            Self::AnchorNotFound => f.write_str("cited anchor not found in the source"),
            Self::AmbiguousAnchor => {
                f.write_str("cited anchor occurs more than once in the source")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Database(err) => Some(err),
            _ => None,
        }
    }
}

/// Outcome of one synchronization.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// Published pages indexed for the first time.
    pub added: usize,
    /// Published pages re-indexed because their content changed.
    pub updated: usize,
    /// Pages removed because they are no longer published.
    pub removed: usize,
    /// Published pages whose content hash was unchanged.
    pub skipped: usize,
    /// Published pages that could not be indexed. They are absent from the
    /// index (never served from stale rows) until fixed.
    pub errors: Vec<SyncIssue>,
    /// Content that was indexed but with unsupported blocks left out.
    pub warnings: Vec<SyncIssue>,
}

impl SyncReport {
    /// True only when every published page was indexed.
    pub fn is_complete(&self) -> bool {
        self.errors.is_empty()
    }
}

/// A lexical search over published passages, optionally limited to pages
/// by frontmatter facets (`tags`, `related-people`, `related-places`, …).
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// Free text. Treated as literal words, never as query syntax. May be
    /// empty when [`filters`](Self::filters) are set: matching pages'
    /// passages are then listed in page and document order.
    pub query: String,
    /// Hard upper bound on returned passages.
    pub max_passages: usize,
    /// Hard upper bound on [`SearchResponse::chars_used`].
    pub max_chars: usize,
    /// A page must pass every filter.
    pub filters: Vec<FacetFilter>,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>, max_passages: usize, max_chars: usize) -> Self {
        Self {
            query: query.into(),
            max_passages,
            max_chars,
            filters: Vec::new(),
        }
    }

    /// Only pages whose facet has all, or any, of `values`. `key` is a
    /// frontmatter key (`roles`, `relationship.alice`) or a kind resolved to
    /// one: `person`, `place`, `tag`, or by convention `related-<kind>s`.
    /// Person names resolve to slugs in `related-people` and `relationship`.
    pub fn with_facet<S: Into<String>>(
        mut self,
        key: impl Into<String>,
        values: impl IntoIterator<Item = S>,
        mode: FacetMatch,
    ) -> Self {
        self.filters.push(FacetFilter {
            facet: key.into(),
            values: values.into_iter().map(Into::into).collect(),
            mode,
        });
        self
    }

    /// Only pages tagged with all, or any, of `tags` (case-insensitive,
    /// leading `#` ignored).
    pub fn with_tags<S: Into<String>>(
        self,
        tags: impl IntoIterator<Item = S>,
        mode: FacetMatch,
    ) -> Self {
        self.with_facet("tags", tags, mode)
    }

    /// Only pages about all, or any, of these people (`related-people`). A
    /// person may be named by slug, entity page title, or alias.
    pub fn with_people<S: Into<String>>(
        self,
        people: impl IntoIterator<Item = S>,
        mode: FacetMatch,
    ) -> Self {
        self.with_facet("related-people", people, mode)
    }

    /// Only pages related to all, or any, of these places (`related-places`).
    pub fn with_places<S: Into<String>>(
        self,
        places: impl IntoIterator<Item = S>,
        mode: FacetMatch,
    ) -> Self {
        self.with_facet("related-places", places, mode)
    }
}

/// Pages whose frontmatter `facet` has all, or any, of `values`. Keys and
/// values match case-insensitively.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetFilter {
    pub facet: String,
    pub values: Vec<String>,
    pub mode: FacetMatch,
}

/// How the values of one [`FacetFilter`] select pages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum FacetMatch {
    /// A page must have every value (AND).
    #[default]
    All,
    /// A page must have at least one value (OR).
    Any,
    /// A page must have none of the values (NOT); pages without the facet pass.
    Not,
}

/// A facet key or value and the number of indexed pages that carry it.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetCount {
    pub value: String,
    pub pages: usize,
}

/// An indexed page.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    /// Wiki-relative path, e.g. `entities/alice.md`.
    pub path: String,
    pub title: String,
}

/// A person named in `related-people`, with their entity page's metadata.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// Slug used in `related-people`; the entity page is `entities/<slug>.md`.
    pub slug: String,
    /// The published entity page, if any.
    pub page_path: Option<String>,
    /// The entity page title, if published.
    pub title: Option<String>,
    /// `aliases` from the entity page.
    pub aliases: Vec<String>,
    /// Every other facet of the entity page, e.g. `relationship.<person>`,
    /// `roles` or `associated-organizations`.
    pub metadata: std::collections::BTreeMap<String, Vec<String>>,
    /// Published pages listing this person in `related-people`.
    pub pages: usize,
}

/// One whole paragraph (or list item) with its page context and citations.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Locator that is stable across rebuilds while the paragraph and its
    /// heading are unchanged: `<page path>#p-<content hash>[-n]`.
    pub passage_id: String,
    pub page_path: String,
    pub title: String,
    pub heading: String,
    /// Claim text with raw citation links moved into [`citations`](Self::citations).
    pub text: String,
    /// Every raw citation from the paragraph; empty means the claim is unsourced.
    pub citations: Vec<Citation>,
    /// Page frontmatter `confidence`, if any.
    pub confidence: Option<String>,
    /// Page frontmatter `contested: true`.
    pub contested: bool,
    /// Page frontmatter `tags`, normalized to lowercase and sorted.
    pub tags: Vec<String>,
    /// Page frontmatter `related-people` (person slugs).
    pub people: Vec<String>,
    /// Page frontmatter `related-places` (place slugs).
    pub places: Vec<String>,
    /// 1-based position in the result list.
    pub rank: usize,
    /// Relevance score (negated FTS5 BM25); higher is better. 0 when the
    /// request had no query and passages were listed by filter only.
    pub score: f64,
}

impl SearchHit {
    /// Characters charged against `max_chars`: the sum of every returned
    /// string field, including citation fields. JSON framing is not counted.
    pub fn char_count(&self) -> usize {
        chars(&self.passage_id)
            + chars(&self.page_path)
            + chars(&self.title)
            + chars(&self.heading)
            + chars(&self.text)
            + self.confidence.as_deref().map_or(0, chars)
            + [&self.tags, &self.people, &self.places]
                .into_iter()
                .flatten()
                .map(|value| chars(value))
                .sum::<usize>()
            + self
                .citations
                .iter()
                .map(Citation::char_count)
                .sum::<usize>()
    }

    /// Whether the page attached at least one raw citation to this passage.
    /// An uncited passage is wiki prose, not source-verified evidence.
    pub fn is_cited(&self) -> bool {
        !self.citations.is_empty()
    }
}

/// Bounded search results.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Default, Clone)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    /// True when matching passages were left out because of a limit.
    pub truncated: bool,
    /// Sum of [`SearchHit::char_count`] over `hits`; never exceeds `max_chars`.
    pub chars_used: usize,
}

/// Size of the current index.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub pages: usize,
    pub passages: usize,
    pub citations: usize,
}

pub(crate) fn chars(text: &str) -> usize {
    text.chars().count()
}
