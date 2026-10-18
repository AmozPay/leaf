//! Citations, sync issues and the errors of reading a wiki's files.

use std::fmt;
use std::path::PathBuf;

/// Result type of the operations that read a wiki's files.
pub type Result<T> = std::result::Result<T, Error>;

/// Why a citation reference or its raw source could not be used. Messages
/// never contain raw transcript text.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A citation reference does not parse.
    InvalidReference(String),
    /// Reading a file failed (for example a missing raw source).
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A citation path is malformed or resolves outside `raw/`.
    UnsafePath(String),
    /// The cited source cannot serve this citation, e.g. an anchor into a
    /// file that is not text.
    UnsupportedSource(String),
    /// The cited anchor (message, block or heading) does not exist in the source.
    AnchorNotFound,
    /// The cited anchor occurs more than once, so the cited part is ambiguous.
    AmbiguousAnchor,
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
            Self::InvalidReference(message) => write!(f, "invalid reference: {message}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::UnsafePath(message) => write!(f, "unsafe citation path: {message}"),
            Self::UnsupportedSource(message) => write!(f, "unsupported source: {message}"),
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
            _ => None,
        }
    }
}

/// A `[[raw/…|label]]` reference from a passage to a source under `raw/`.
///
/// Its [`Display`](fmt::Display) form, e.g. `raw/chatgpt/x.md#^message-ID`,
/// parses back with [`str::parse`]; the `[[…|label]]` wikilink form parses too.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Citation {
    /// Wiki-relative path under `raw/`, e.g. `raw/chatgpt/x.md`. Any file type.
    pub raw_path: String,
    /// The cited part of the file; `None` cites the whole document.
    pub anchor: Option<Anchor>,
    /// Link label from the page; empty when the link had none.
    pub label: String,
}

impl Citation {
    /// Characters this citation is charged in a search budget.
    pub fn char_count(&self) -> usize {
        let chars = |text: &str| text.chars().count();
        chars(&self.raw_path)
            + self.anchor.as_ref().map_or(0, |a| chars(a.value()))
            + chars(&self.label)
    }
}

impl fmt::Display for Citation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw_path)?;
        match &self.anchor {
            Some(anchor) => write!(f, "#{anchor}"),
            None => Ok(()),
        }
    }
}

/// The part of a raw source a [`Citation`] points at.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "kind", content = "value", rename_all = "lowercase")
)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// `#^message-<id>`: one transcript message.
    Message(String),
    /// `#^<id>`: the Obsidian block (paragraph, list item, table, …) marked `^<id>`.
    Block(String),
    /// `#Heading` or `#Heading#Subheading`: that heading's section.
    Heading(String),
}

impl Anchor {
    /// The identifier or heading path, without anchor syntax.
    pub fn value(&self) -> &str {
        match self {
            Self::Message(id) | Self::Block(id) | Self::Heading(id) => id,
        }
    }
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(id) => write!(f, "^message-{id}"),
            Self::Block(id) => write!(f, "^{id}"),
            Self::Heading(path) => f.write_str(path),
        }
    }
}

/// A problem with one wiki file found while syncing the index.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncIssue {
    /// Wiki-relative path (or the raw link text for unusable `index.md` links).
    pub path: String,
    pub message: String,
}

/// The cited part of a raw source, returned only for an indexed citation.
///
/// Text is untrusted source data, not instructions. A source that is not text
/// (a PDF, an image, …) is described by its metadata only.
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone)]
pub struct CitationExcerpt {
    pub citation: Citation,
    /// Media type, sniffed from the content and else the file extension,
    /// e.g. `text/markdown` or `application/pdf`.
    pub media_type: String,
    /// Size of the whole source file in bytes.
    pub size_bytes: u64,
    /// The cited text (whole document without frontmatter, message, block or
    /// section), cut to `max_chars`; `None` when the source is not text.
    pub text: Option<String>,
    /// True when the text was cut to `max_chars` characters.
    pub truncated: bool,
}
