//! Command-line arguments and the paths and citations they resolve to.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum, ValueHint};
use leaf_viewer::{FacetMatch, SearchRequest};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Citation-preserving search over a local Markdown wiki.
///
/// `viewer` commands read the pages published in the wiki's index.md and keep
/// a disposable SQLite cache; they never write to the wiki.
#[derive(Debug, Parser)]
#[command(version)]
pub struct Cli {
    /// Wiki root, containing index.md and raw/ [default: ~/wiki]
    #[arg(long, env = "LEAF_WIKI", global = true, value_hint = ValueHint::DirPath)]
    pub wiki: Option<PathBuf>,

    /// Cache database; must be outside raw/ [default: ~/.cache/leaf/leaf.db]
    #[arg(long, env = "LEAF_DB", global = true, value_hint = ValueHint::FilePath)]
    pub db: Option<PathBuf>,

    /// Print JSON on stdout instead of text
    #[arg(long, global = true)]
    pub json: bool,

    /// Use the cache as it is instead of syncing before reading it
    #[arg(long, global = true)]
    pub no_sync: bool,

    /// Refuse a cache made by an incompatible Leaf version instead of
    /// rebuilding it (the cache is disposable, so rebuilding is the default)
    #[arg(long, global = true)]
    pub no_rebuild: bool,

    /// Accepted for compatibility: outdated caches are rebuilt by default
    #[arg(long, global = true, hide = true)]
    pub rebuild_cache: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// The tool a command belongs to: `leaf viewer …` reads the wiki,
/// `leaf editor …` (built with `--features editor`) changes it.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Read-only commands: sync the index, search, cite, list metadata
    Viewer {
        #[command(subcommand)]
        command: ViewerCommand,
    },

    /// Validated edits to the wiki: check and apply batches, read metadata
    /// and pages, check conformance
    #[cfg(feature = "editor")]
    Editor {
        #[command(subcommand)]
        command: EditorCommand,
    },

    /// Not built into this binary (rebuild with `--features editor`)
    #[cfg(not(feature = "editor"))]
    #[command(hide = true)]
    Editor {
        #[arg(num_args = 0.., allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

/// The editor's commands (`leaf editor …`).
#[cfg(feature = "editor")]
#[derive(Debug, Subcommand)]
pub enum EditorCommand {
    /// Create a new wiki in an empty folder: folders, SCHEMA.md, index.md,
    /// log.md and _meta/vocabulary.yaml from the baseline
    ///
    /// The wiki is --wiki (default ~/wiki). Refuses a folder that already has
    /// files in it.
    Init {
        /// What the wiki covers, in one line
        #[arg(long)]
        domain: Option<String>,

        /// A starting tag; repeat or comma-separate for several
        #[arg(long = "tag", value_name = "TAG", value_delimiter = ',')]
        tags: Vec<String>,
    },

    /// Check a batch (YAML or JSON) without writing anything
    ///
    /// Prints whether it would be applied, and why not: every error with the
    /// operation it belongs to and suggestions, spellings that would be
    /// rewritten, and warnings. Exits with status 4 when it would be rejected.
    Check {
        /// The batch file, or - for standard input
        batch: PathBuf,
    },

    /// Apply a batch: all of it or none of it, then sync the index
    ///
    /// Writes only when the whole batch is acceptable (see `check`); a
    /// rejected batch changes nothing and exits with status 4. Rehearse on a
    /// copy of a wiki first.
    Apply {
        /// The batch file, or - for standard input
        batch: PathBuf,
    },

    /// List metadata keys with their values, page counts and descriptions
    Metadata,

    /// Check a metadata key, or a value of a key, against the vocabulary
    ///
    /// Says whether it is known, would be rewritten (a synonym), or is
    /// unknown (with suggestions).
    Suggest {
        /// A key, e.g. related-people or relationship.alice
        key: String,
        /// A value of that key to check
        value: Option<String>,
    },

    /// Check the wiki against the vocabulary and propose declarations
    Conformance,

    /// Print a page with its metadata, headings and passage IDs
    Page {
        /// A published page, e.g. concepts/garden
        page: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ViewerCommand {
    /// Bring the cache up to date with the pages published in index.md
    ///
    /// Exits with status 3 when a published page could not be indexed.
    Sync,

    /// Search published passages; the words are matched literally
    ///
    /// --meta limits the search to pages by frontmatter metadata, e.g.
    /// --meta=person:bobby,chloe --meta=place:lyon --meta=roles:cto. KIND
    /// is a frontmatter key, a shorthand (person, place, tag), or by
    /// convention related-<KIND>s. Without words, every passage of the
    /// matching pages is listed in order. Values match case-insensitively.
    Search(SearchArgs),

    /// Print the raw source behind a citation found by `search`
    ///
    /// A whole document, one transcript message, an Obsidian block or a heading
    /// section. A source that is not text (PDF, image, …) is described by its
    /// media type and size.
    Cite {
        /// raw/<path>, raw/<path>#^message-<id>, raw/<path>#^<block-id> or
        /// raw/<path>#<heading>[#<subheading>]; the [[…|label]] wikilink also works
        reference: String,

        /// Most characters of text to print
        #[arg(short = 'c', long, default_value_t = 2_000)]
        max_chars: usize,
    },

    /// List the frontmatter tags of indexed pages, most used first
    Tags,

    /// List people named in related-people, with their entity page metadata
    People,

    /// List places named in related-places, most used first
    Places,

    /// List metadata kinds usable with --meta, or the values of one kind
    Meta {
        /// A kind, e.g. person, place, roles or relationship.<person>
        kind: Option<String>,

        /// List the built-in suggested metadata keys, with their synonyms and
        /// common labels, instead of this wiki's metadata (needs no wiki)
        #[arg(long, conflicts_with = "kind")]
        suggested: bool,
    },

    /// Count indexed pages, passages and citations
    Stats,
}

/// The arguments of `search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Words to search for (optional with a filter)
    #[arg(
        required_unless_present_any = ["tags", "meta", "meta_all", "meta_any", "meta_not"],
        num_args = 0..
    )]
    pub query: Vec<String>,

    /// Only pages with this metadata, values combined as --match says;
    /// repeat for several kinds
    #[arg(long, value_name = "KIND:VALUE[,VALUE]", value_parser = parse_meta)]
    pub meta: Vec<(String, Vec<String>)>,

    /// Only pages with every one of these values (AND), whatever --match says
    #[arg(long, value_name = "KIND:VALUE[,VALUE]", value_parser = parse_meta)]
    pub meta_all: Vec<(String, Vec<String>)>,

    /// Only pages with at least one of these values (OR), whatever --match says
    #[arg(long, value_name = "KIND:VALUE[,VALUE]", value_parser = parse_meta)]
    pub meta_any: Vec<(String, Vec<String>)>,

    /// Leave out pages with any of these values (NOT); pages without the
    /// kind are kept
    #[arg(long, value_name = "KIND:VALUE[,VALUE]", value_parser = parse_meta)]
    pub meta_not: Vec<(String, Vec<String>)>,

    /// Shortcut for --meta=tag:TAG; repeat or comma-separate for several
    #[arg(short = 't', long = "tag", value_name = "TAG", value_delimiter = ',')]
    pub tags: Vec<String>,

    /// How the values of --meta and --tag combine: a page needs all of
    /// them (AND) or any one (OR). Different filters always combine with AND
    #[arg(short = 'm', long = "match", value_enum, default_value_t = Match::All)]
    pub mode: Match,

    /// Most passages to return
    #[arg(short = 'n', long, default_value_t = 5)]
    pub max_passages: usize,

    /// Most characters to return, summed over every returned text field
    #[arg(short = 'c', long, default_value_t = 4_000)]
    pub max_chars: usize,
}

impl SearchArgs {
    /// The request these arguments describe: one filter per kind and mode,
    /// so `--meta=person:a --meta=person:b` is `person:a,b`.
    pub fn request(&self) -> SearchRequest {
        let mut request =
            SearchRequest::new(self.query.join(" "), self.max_passages, self.max_chars);
        let tag_filter = (!self.tags.is_empty()).then(|| ("tag".to_string(), self.tags.clone()));
        let filters = [
            (
                FacetMatch::from(self.mode),
                self.meta.iter().cloned().chain(tag_filter).collect(),
            ),
            (FacetMatch::All, self.meta_all.clone()),
            (FacetMatch::Any, self.meta_any.clone()),
            (FacetMatch::Not, self.meta_not.clone()),
        ];
        let mut grouped: BTreeMap<(String, usize), (FacetMatch, Vec<String>)> = BTreeMap::new();
        for (order, (mode, kinds)) in filters.into_iter().enumerate() {
            for (kind, values) in kinds {
                grouped
                    .entry((kind.trim().to_lowercase(), order))
                    .or_insert_with(|| (mode, Vec::new()))
                    .1
                    .extend(values);
            }
        }
        for ((kind, _), (mode, values)) in grouped {
            request = request.with_facet(kind, values, mode);
        }
        request
    }
}

/// How several values of one filter combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Match {
    /// Pages with every value (AND)
    All,
    /// Pages with at least one value (OR)
    Any,
}

impl From<Match> for FacetMatch {
    fn from(value: Match) -> Self {
        match value {
            Match::All => FacetMatch::All,
            Match::Any => FacetMatch::Any,
        }
    }
}

/// `KIND:VALUE[,VALUE…]` for `--meta`.
fn parse_meta(arg: &str) -> Result<(String, Vec<String>), String> {
    let (key, values) = arg
        .split_once(':')
        .ok_or_else(|| format!("`{arg}` is not KIND:VALUE"))?;
    let key = key.trim();
    let values: Vec<String> = values
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .collect();
    if key.is_empty() || values.is_empty() {
        return Err(format!("`{arg}` needs a kind and at least one value"));
    }
    Ok((key.to_string(), values))
}

impl Cli {
    pub fn wiki_root(&self) -> Result<PathBuf> {
        match &self.wiki {
            Some(path) => Ok(path.clone()),
            None => leaf_viewer::default_wiki_root().context("pass --wiki"),
        }
    }

    /// The configured database, or the default one in the user cache
    /// directory (created if needed).
    pub fn database(&self) -> Result<PathBuf> {
        match &self.db {
            Some(path) => Ok(path.clone()),
            None => leaf_viewer::default_database_path().context("pass --db"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn arguments_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn meta_arguments() {
        assert_eq!(
            parse_meta(" person : bobby, Chloé Dubois ,").unwrap(),
            (
                "person".to_string(),
                vec!["bobby".to_string(), "Chloé Dubois".to_string()]
            )
        );
        assert_eq!(
            parse_meta("relationship.alice:friend").unwrap().0,
            "relationship.alice"
        );
        for bad in ["person", ":bobby", "person:", "person: , "] {
            assert!(parse_meta(bad).is_err(), "{bad}");
        }
    }
}
