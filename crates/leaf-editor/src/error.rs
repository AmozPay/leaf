//! Editor errors.

use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The vocabulary does not parse or is inconsistent.
    InvalidVocabulary {
        path: PathBuf,
        problems: Vec<String>,
    },
    /// A page that is not published, or that cannot be read as one.
    Page { path: String, message: String },
    /// Reading a wiki file failed.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A new wiki cannot be created there.
    Init { path: PathBuf, message: String },
    /// The read side (index, search) failed.
    Viewer(leaf_viewer::Error),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl From<leaf_viewer::Error> for Error {
    fn from(err: leaf_viewer::Error) -> Self {
        Self::Viewer(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVocabulary { path, problems } => {
                write!(f, "{} is invalid: {}", path.display(), problems.join("; "))
            }
            Self::Page { path, message } => write!(f, "{path}: {message}"),
            // The folder is not named: whoever runs `init` already chose it.
            Self::Init { message, .. } => write!(f, "cannot create the wiki: {message}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Viewer(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Viewer(err) => Some(err),
            _ => None,
        }
    }
}
