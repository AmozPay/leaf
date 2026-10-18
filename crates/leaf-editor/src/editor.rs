//! `Editor`: a wiki with its vocabulary and index. This slice reads only:
//! checking keys and values, and listing the metadata in use.

use crate::check::Checker;
use crate::error::{Error, Result};
use crate::suggest::{Candidate, Lookup};
use crate::vocabulary::{KeySpec, ValueKind, Vocabulary, base_key};
use leaf_common::parse;
use leaf_common::publication::page_path;
use leaf_viewer::{WikiConfig, WikiIndex};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Where the wiki is and where the viewer's cache lives.
#[derive(Debug, Clone)]
pub struct EditorConfig {
    /// Root of the wiki (contains `index.md`, `raw/` and `_meta/vocabulary.yaml`).
    pub wiki_root: PathBuf,
    /// The viewer's SQLite cache, as for [`WikiConfig`].
    pub database_path: PathBuf,
    /// The date written to `created`/`updated` and `log.md`; today by default.
    pub today: Option<jiff::civil::Date>,
}

impl EditorConfig {
    pub fn new(wiki_root: impl Into<PathBuf>, database_path: impl Into<PathBuf>) -> Self {
        Self {
            wiki_root: wiki_root.into(),
            database_path: database_path.into(),
            today: None,
        }
    }

    /// Use a fixed date instead of today (for tests and replays).
    pub fn with_today(mut self, today: jiff::civil::Date) -> Self {
        self.today = Some(today);
        self
    }
}

/// A wiki opened for validated editing.
pub struct Editor {
    pub(crate) root: PathBuf,
    pub(crate) vocabulary: Vocabulary,
    pub(crate) index: WikiIndex,
    pub(crate) today: Option<jiff::civil::Date>,
}

/// A metadata key in the vocabulary or in use, with its values.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetadataKey {
    pub key: String,
    /// The vocabulary's definition (for `relationship.x`: of `relationship`);
    /// `None` for a key in use but not declared.
    pub spec: Option<KeySpec>,
    /// Pages that have the key.
    pub pages: usize,
    pub values: Vec<MetadataValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetadataValue {
    pub value: String,
    /// Pages that have the value.
    pub pages: usize,
    /// Whether the vocabulary (or the wiki, for page references) knows it.
    pub declared: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub synonyms: Vec<String>,
}

/// A page as an agent reads it before editing: its metadata, headings and
/// passages with the IDs that passage operations take.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageView {
    pub path: String,
    pub title: String,
    /// Frontmatter keys and values (map entries as `key.<name>`).
    pub metadata: std::collections::BTreeMap<String, Vec<String>>,
    pub headings: Vec<String>,
    pub passages: Vec<PassageView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PassageView {
    /// `<page>#p-<hash>`: what `replace_passage`, `remove_passage`,
    /// `add_citation` and `add_passage {after}` take.
    pub id: String,
    pub heading: String,
    pub text: String,
    /// Citation references, as `add_citation` and `drop_citations` take them.
    pub citations: Vec<String>,
}

impl Editor {
    /// Open a wiki: roll back a batch a crash interrupted (if any), load
    /// and check its vocabulary, then bring the index up to date
    /// (rebuilding an outdated cache).
    pub fn open(config: EditorConfig) -> Result<Self> {
        crate::commit::recover(&config.wiki_root)?;
        let vocabulary = Vocabulary::load(&config.wiki_root)?;
        let mut index = WikiIndex::open(
            WikiConfig::new(&config.wiki_root, &config.database_path)
                .rebuild_incompatible_cache(true),
        )?;
        index.sync()?;
        Ok(Self {
            root: config.wiki_root,
            vocabulary,
            index,
            today: config.today,
        })
    }

    pub fn vocabulary(&self) -> &Vocabulary {
        &self.vocabulary
    }

    /// Read access to the wiki's index.
    pub fn index(&self) -> &WikiIndex {
        &self.index
    }

    pub fn wiki_root(&self) -> &Path {
        &self.root
    }

    /// The date the editor writes.
    pub fn today(&self) -> jiff::civil::Date {
        self.today.unwrap_or_else(|| jiff::Zoned::now().date())
    }

    /// A published page with its headings and passage IDs.
    pub fn page(&self, page: &str) -> Result<PageView> {
        let path = page_path(page);
        let not_readable = |message: String| Error::Page {
            path: path.clone(),
            message,
        };
        if !self.index.pages()?.iter().any(|p| p.path == path) {
            return Err(not_readable("not a published page".into()));
        }
        let file = parse::published_file(&self.root, &path).map_err(not_readable)?;
        let content = std::fs::read_to_string(&file).map_err(|err| Error::io(&file, err))?;
        let parsed = parse::page(&content).map_err(not_readable)?;
        Ok(PageView {
            title: parsed.title,
            metadata: parsed.facets,
            headings: parsed.headings.into_iter().map(|h| h.text).collect(),
            passages: parsed
                .passages
                .into_iter()
                .map(|p| PassageView {
                    id: format!("{path}#{}", p.locator),
                    heading: p.heading,
                    text: p.text,
                    citations: p.citations.iter().map(|c| c.to_string()).collect(),
                })
                .collect(),
            path,
        })
    }

    /// Checks against this editor's vocabulary.
    pub(crate) fn checker(&self) -> Checker<'_> {
        Checker {
            vocabulary: &self.vocabulary,
            index: &self.index,
            root: &self.root,
            staged: &[],
            gone: &[],
            staged_raw: &[],
        }
    }

    /// Check a metadata key. Map entries (`relationship.<person>`) are
    /// checked as the map key plus an entry name of the map's kind.
    pub fn check_key(&self, key: &str) -> Result<Lookup> {
        self.checker().check_key(key)
    }

    /// Check a value for a key; the key must be acceptable first.
    pub fn check_value(&self, key: &str, value: &str) -> Result<Lookup> {
        self.checker().check_value(key, value)
    }

    /// Every declared or used metadata key with its values and page counts.
    pub fn list_metadata(&self) -> Result<Vec<MetadataKey>> {
        let used = self.index.facets()?;
        let mut names: Vec<String> = self.vocabulary.keys.keys().cloned().collect();
        names.extend(used.iter().map(|k| k.value.clone()));
        names.sort();
        names.dedup();
        let mut listing = Vec::new();
        for key in names {
            let pages = used.iter().find(|k| k.value == key).map_or(0, |k| k.pages);
            let spec = self.vocabulary.key(&key).cloned();
            let kind = self.listed_kind(&key);
            let known: Vec<Candidate> = match kind {
                Some(kind) => self.checker().candidates(kind, &key)?,
                None => Vec::new(),
            };
            let mut values: Vec<MetadataValue> = Vec::new();
            for used in self.index.facet_values(&key)? {
                let term = known
                    .iter()
                    .find(|c| c.value.to_lowercase() == used.value.to_lowercase());
                values.push(MetadataValue {
                    declared: term.is_some()
                        || matches!(
                            kind,
                            Some(
                                ValueKind::Text
                                    | ValueKind::Date
                                    | ValueKind::Boolean
                                    | ValueKind::Raw
                            )
                        ),
                    description: self.description(&key, &used.value),
                    synonyms: term.map(|t| t.other_names.clone()).unwrap_or_default(),
                    value: used.value,
                    pages: used.pages,
                });
            }
            if matches!(kind, Some(ValueKind::Label | ValueKind::Place)) {
                for candidate in &known {
                    if !values
                        .iter()
                        .any(|v| v.value.to_lowercase() == candidate.value.to_lowercase())
                    {
                        values.push(MetadataValue {
                            value: candidate.value.clone(),
                            pages: 0,
                            declared: true,
                            description: self.description(&key, &candidate.value),
                            synonyms: candidate.other_names.clone(),
                        });
                    }
                }
            }
            listing.push(MetadataKey {
                key,
                spec,
                pages,
                values,
            });
        }
        Ok(listing)
    }

    /// What the values listed under `key` are: a map key lists its entry
    /// names (`relationship` → people), a map entry its values (labels).
    fn listed_kind(&self, key: &str) -> Option<ValueKind> {
        let spec = self.vocabulary.key(key)?;
        match spec.map {
            Some(entries) if !key.contains('.') => Some(entries),
            _ => Some(spec.values),
        }
    }

    fn term(&self, key: &str, value: &str) -> Option<&crate::vocabulary::Term> {
        match self.listed_kind(key)? {
            ValueKind::Label => self.vocabulary.labels_of(base_key(key))?.get(value),
            ValueKind::Place => self.vocabulary.places.get(value),
            _ => None,
        }
    }

    fn description(&self, key: &str, value: &str) -> Option<String> {
        self.term(key, value)
            .and_then(|term| term.description.clone())
    }
}
