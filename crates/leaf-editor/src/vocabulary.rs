//! The vocabulary: page types, metadata keys and what their values mean,
//! allowed labels and places, raw collections, and batch limits.
//!
//! Leaf ships a baseline (`data/baseline.yaml`, the LLM-wiki conventions).
//! A wiki's optional `_meta/vocabulary.yaml` only extends it: its own tags,
//! places, keys and collections, or stricter requirements. Entries with the
//! same name replace the baseline's (page types, keys, raw collections,
//! labels, places); lists are combined (required keys, summary headings);
//! `limits`, `uncited_intro` and `raw_manifest` override when given.

use crate::error::{Error, Result};
use leaf_common::parse::PUBLISHED_DIRS;
use leaf_common::publication::is_plain_relative;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Where the vocabulary lives, relative to the wiki root.
pub const VOCABULARY_PATH: &str = "_meta/vocabulary.yaml";
/// The vocabulary format this build reads.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vocabulary {
    pub version: u32,
    /// Page types by `type` value.
    pub page_types: BTreeMap<String, PageType>,
    /// Frontmatter keys every page must have (`title` is always required).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_keys: Vec<String>,
    /// Frontmatter keys a new page must have, beyond `required_keys` (pages
    /// written before the rule are not rejected for lacking them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_on_create: Vec<String>,
    /// Whether a page's introduction (passages before its first section
    /// heading, i.e. under its title) may be uncited.
    #[serde(default, skip_serializing_if = "is_false")]
    pub uncited_intro: bool,
    /// Section headings under which passages may be uncited (summaries).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary_headings: Vec<String>,
    #[serde(default)]
    pub limits: Limits,
    /// The checksum manifest new raw files are recorded in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_manifest: Option<String>,
    /// `raw/<collection>/` folders and their rules.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub raw_collections: BTreeMap<String, RawCollection>,
    /// Metadata keys and what their values mean.
    pub keys: BTreeMap<String, KeySpec>,
    /// Allowed values of `label` keys, by key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, BTreeMap<String, Term>>,
    /// Allowed `place` values (slugs).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub places: BTreeMap<String, Term>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageType {
    /// One of the published directories, e.g. `entities`.
    pub folder: String,
    /// The `index.md` section listing pages of this type, e.g. `Entities`.
    pub index_section: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Limits {
    pub ops_per_batch: usize,
    pub passage_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            ops_per_batch: 50,
            passage_chars: 2_000,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCollection {
    /// Whether new files may be added (false for original exports).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub addable: bool,
    /// Allowed media types, e.g. `text/markdown`; empty allows any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<String>,
    /// Frontmatter keys new Markdown files must have.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require_frontmatter: Vec<String>,
    /// An anchor kind new files must contain at least once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_anchor: Option<AnchorKind>,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorKind {
    /// `^message-<id>`
    Message,
    /// `^<block-id>`
    Block,
    /// Either.
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySpec {
    /// What each value is (for a map key: what each entry's values are).
    pub values: ValueKind,
    /// A map key (`relationship: {alice: [friend]}`): what the entry names are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<ValueKind>,
    /// At most one value.
    #[serde(default, skip_serializing_if = "is_false")]
    pub single: bool,
    /// Only on pages of this type, or on `person` entities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_on: Option<String>,
    /// Labels of a person map mirrored on the other person's page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symmetric: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What a metadata value refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueKind {
    /// One of the key's declared `labels`.
    Label,
    /// The slug of a person entity page.
    Person,
    /// The slug of a non-person entity page.
    Organization,
    /// The slug of any entity page.
    Entity,
    /// A declared place slug.
    Place,
    /// A published page path.
    Page,
    /// A raw source path.
    Raw,
    /// An ISO date (`2026-10-17`).
    Date,
    /// `true` or `false`.
    Boolean,
    /// Any non-empty text.
    Text,
}

/// A declared label or place.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Term {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Other spellings, rewritten to this term.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synonyms: Vec<String>,
}

/// A wiki's additions to the baseline (`_meta/vocabulary.yaml`). Every field
/// is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extension {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub page_types: BTreeMap<String, PageType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_on_create: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncited_intro: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary_headings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub raw_collections: BTreeMap<String, RawCollection>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, KeySpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, BTreeMap<String, Term>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub places: BTreeMap<String, Term>,
}

fn version() -> u32 {
    VERSION
}

impl Extension {
    /// The extension as YAML.
    pub fn to_yaml(&self) -> String {
        to_yaml(self)
    }

    /// The wiki's extension (`_meta/vocabulary.yaml`), empty if it has none.
    pub fn load(wiki_root: &Path) -> Result<Self> {
        let path = wiki_root.join(VOCABULARY_PATH);
        match fs::read_to_string(&path) {
            Ok(text) => from_yaml(&text, &path),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Self {
                version: VERSION,
                ..Self::default()
            }),
            Err(err) => Err(Error::io(path, err)),
        }
    }
}

/// YAML in the wiki's style (YAML 1.2 scalars, no directive).
fn to_yaml(value: &impl Serialize) -> String {
    let mut options = serde_saphyr::SerializerOptions::default();
    options.yaml_12 = true;
    options.no_lang_directive = true;
    options.prefer_block_scalars = false;
    serde_saphyr::to_string_with_options(value, options).unwrap_or_default()
}

static BASELINE: &str = include_str!("../data/baseline.yaml");

impl Vocabulary {
    /// The built-in baseline: Leaf's LLM-wiki conventions. Keys without
    /// a description take the built-in dictionary's.
    pub fn baseline() -> Self {
        let mut baseline = Self::parse(BASELINE, PathBuf::from("data/baseline.yaml"))
            .expect("the bundled baseline vocabulary is valid");
        let dictionary = leaf_common::dictionary::dictionary();
        for (key, spec) in &mut baseline.keys {
            if spec.description.is_none() {
                spec.description = dictionary.keys.get(key).map(|k| k.description.clone());
            }
        }
        baseline
    }

    /// The vocabulary of the wiki at `wiki_root`: the baseline, extended by
    /// its `_meta/vocabulary.yaml` if it has one.
    pub fn load(wiki_root: &Path) -> Result<Self> {
        let path = wiki_root.join(VOCABULARY_PATH);
        match fs::read_to_string(&path) {
            Ok(text) => Self::baseline_with(&text, path),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Self::baseline()),
            Err(err) => Err(Error::io(path, err)),
        }
    }

    /// The baseline extended by extension text; `path` is only used in errors.
    pub fn baseline_with(text: &str, path: PathBuf) -> Result<Self> {
        let extension: Extension = from_yaml(text, &path)?;
        Self::baseline().extended(extension).checked(path)
    }

    /// Parse and check a complete vocabulary; `path` is only used in errors.
    pub fn parse(text: &str, path: PathBuf) -> Result<Self> {
        let vocabulary: Self = from_yaml(text, &path)?;
        vocabulary.checked(path)
    }

    fn checked(self, path: PathBuf) -> Result<Self> {
        let problems = self.problems();
        if problems.is_empty() {
            Ok(self)
        } else {
            Err(Error::InvalidVocabulary { path, problems })
        }
    }

    /// This vocabulary with `extension` applied (see the module docs).
    pub fn extended(mut self, extension: Extension) -> Self {
        self.version = extension.version;
        self.page_types.extend(extension.page_types);
        for (list, more) in [
            (&mut self.required_keys, extension.required_keys),
            (&mut self.required_on_create, extension.required_on_create),
            (&mut self.summary_headings, extension.summary_headings),
        ] {
            for item in more {
                if !list.contains(&item) {
                    list.push(item);
                }
            }
        }
        if let Some(uncited_intro) = extension.uncited_intro {
            self.uncited_intro = uncited_intro;
        }
        if let Some(limits) = extension.limits {
            self.limits = limits;
        }
        if extension.raw_manifest.is_some() {
            self.raw_manifest = extension.raw_manifest;
        }
        self.raw_collections.extend(extension.raw_collections);
        self.keys.extend(extension.keys);
        for (key, labels) in extension.labels {
            self.labels.entry(key).or_default().extend(labels);
        }
        self.places.extend(extension.places);
        self
    }

    /// What a complete vocabulary adds to the baseline, as an extension.
    pub fn extension_over_baseline(&self) -> Extension {
        let baseline = Self::baseline();
        let new_items = |list: &[String], base: &[String]| -> Vec<String> {
            list.iter()
                .filter(|item| !base.contains(item))
                .cloned()
                .collect()
        };
        fn changed<V: PartialEq + Clone>(
            mine: &BTreeMap<String, V>,
            base: &BTreeMap<String, V>,
        ) -> BTreeMap<String, V> {
            mine.iter()
                .filter(|(name, value)| base.get(*name) != Some(*value))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        }
        Extension {
            version: self.version,
            page_types: changed(&self.page_types, &baseline.page_types),
            required_keys: new_items(&self.required_keys, &baseline.required_keys),
            required_on_create: new_items(&self.required_on_create, &baseline.required_on_create),
            uncited_intro: (self.uncited_intro != baseline.uncited_intro)
                .then_some(self.uncited_intro),
            summary_headings: new_items(&self.summary_headings, &baseline.summary_headings),
            limits: (self.limits != baseline.limits).then_some(self.limits),
            raw_manifest: self
                .raw_manifest
                .clone()
                .filter(|_| self.raw_manifest != baseline.raw_manifest),
            raw_collections: changed(&self.raw_collections, &baseline.raw_collections),
            keys: changed(&self.keys, &baseline.keys),
            labels: self
                .labels
                .iter()
                .map(|(key, labels)| {
                    let empty = BTreeMap::new();
                    (
                        key.clone(),
                        changed(labels, baseline.labels.get(key).unwrap_or(&empty)),
                    )
                })
                .filter(|(_, labels)| !labels.is_empty())
                .collect(),
            places: changed(&self.places, &baseline.places),
        }
    }

    /// The vocabulary as YAML.
    pub fn to_yaml(&self) -> String {
        to_yaml(self)
    }

    /// The labels allowed for `key`, also for map entries (`relationship.x`).
    pub fn labels_of(&self, key: &str) -> Option<&BTreeMap<String, Term>> {
        self.labels.get(base_key(key))
    }

    /// The spec of a key, also for map entries (`relationship.x`).
    pub fn key(&self, key: &str) -> Option<&KeySpec> {
        self.keys.get(base_key(key))
    }

    /// Inconsistencies that make the vocabulary unusable.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut problem = |text: String| problems.push(text);
        if self.version != VERSION {
            problem(format!("version must be {VERSION}, not {}", self.version));
        }
        if self.page_types.is_empty() {
            problem("page_types must declare at least one type".into());
        }
        let mut folders = Vec::new();
        for (name, page_type) in &self.page_types {
            if !is_slug(name) {
                problem(format!("page type `{name}` must be a lowercase slug"));
            }
            if !PUBLISHED_DIRS.contains(&page_type.folder.as_str()) {
                problem(format!(
                    "page type `{name}`: folder `{}` is not one of {}",
                    page_type.folder,
                    PUBLISHED_DIRS.join(", ")
                ));
            }
            if folders.contains(&&page_type.folder) {
                problem(format!(
                    "folder `{}` is used by two page types",
                    page_type.folder
                ));
            }
            folders.push(&page_type.folder);
            if page_type.index_section.trim().is_empty() {
                problem(format!("page type `{name}` needs an index_section"));
            }
        }
        for key in self.required_keys.iter().chain(&self.required_on_create) {
            if key != "title" && !self.keys.contains_key(key) {
                problem(format!("required key `{key}` is not declared under keys"));
            }
        }
        if self.summary_headings.iter().any(|h| h.trim().is_empty()) {
            problem("summary_headings must not contain empty headings".into());
        }
        if self.limits.ops_per_batch == 0 || self.limits.passage_chars == 0 {
            problem("limits must be positive".into());
        }
        if let Some(manifest) = &self.raw_manifest
            && !(manifest.ends_with(".json")
                && is_plain_relative(manifest)
                && !manifest.starts_with("raw/"))
        {
            problem(format!(
                "raw_manifest `{manifest}` must be a relative .json path outside raw/"
            ));
        }
        for (name, collection) in &self.raw_collections {
            if !is_slug(name) {
                problem(format!("raw collection `{name}` must be a lowercase slug"));
            }
            for media in &collection.media {
                if !media.contains('/') {
                    problem(format!(
                        "raw collection `{name}`: `{media}` is not a media type"
                    ));
                }
            }
        }
        for (key, spec) in &self.keys {
            if key == "title" || !is_key(key) {
                problem(format!(
                    "key `{key}` must be a lowercase key without dots, and not `title`"
                ));
            }
            if let Some(only_on) = &spec.only_on
                && only_on != "person"
                && !self.page_types.contains_key(only_on)
            {
                problem(format!(
                    "key `{key}`: only_on `{only_on}` is neither `person` nor a page type"
                ));
            }
            if spec.map.is_some_and(|kind| {
                !matches!(
                    kind,
                    ValueKind::Person
                        | ValueKind::Organization
                        | ValueKind::Entity
                        | ValueKind::Label
                        | ValueKind::Place
                )
            }) {
                problem(format!(
                    "key `{key}`: map entries must be named by person, organization, entity, label or place"
                ));
            }
            if !spec.symmetric.is_empty() {
                if spec.map != Some(ValueKind::Person) || spec.values != ValueKind::Label {
                    problem(format!(
                        "key `{key}`: symmetric labels need `map: person` and `values: label`"
                    ));
                }
                for label in &spec.symmetric {
                    if !self
                        .labels
                        .get(key)
                        .is_some_and(|labels| labels.contains_key(label))
                    {
                        problem(format!(
                            "key `{key}`: symmetric label `{label}` is not declared under labels"
                        ));
                    }
                }
            }
        }
        for (key, labels) in &self.labels {
            if self.keys.get(key).map(|spec| spec.values) != Some(ValueKind::Label) {
                problem(format!(
                    "labels for `{key}`, which is not a `values: label` key"
                ));
            }
            check_terms(&format!("labels of `{key}`"), labels, &mut problem);
        }
        check_terms("places", &self.places, &mut problem);
        problems
    }
}

fn from_yaml<T: serde::de::DeserializeOwned>(text: &str, path: &Path) -> Result<T> {
    serde_saphyr::from_str(text).map_err(|err| {
        let message = err.to_string();
        let first = message.lines().next().unwrap_or_default().to_string();
        Error::InvalidVocabulary {
            path: path.to_path_buf(),
            problems: vec![first],
        }
    })
}

/// Terms are slugs, and no spelling (term or synonym) is used twice.
fn check_terms(what: &str, terms: &BTreeMap<String, Term>, problem: &mut impl FnMut(String)) {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for (name, term) in terms {
        if name.trim().is_empty() || name.trim().to_lowercase() != *name {
            problem(format!("{what}: `{name}` must be lowercase and trimmed"));
        }
        for spelling in std::iter::once(name).chain(&term.synonyms) {
            let normalized = spelling.trim().to_lowercase();
            if let Some(other) = seen.insert(normalized.clone(), name) {
                problem(format!(
                    "{what}: `{normalized}` is used by both `{other}` and `{name}`"
                ));
            }
        }
    }
}

/// `relationship.alice` → `relationship`.
pub(crate) fn base_key(key: &str) -> &str {
    key.split_once('.').map_or(key, |(base, _)| base)
}

fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_key(text: &str) -> bool {
    is_slug(&text.replace('_', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const EXAMPLE: &str = "\
version: 1
page_types:
  entity: {folder: entities, index_section: Entities}
  concept: {folder: concepts, index_section: Concepts}
required_keys: [title, type, tags]
uncited_intro: true
summary_headings: [Historical evidence]
raw_manifest: _meta/manifest.json
raw_collections:
  chatgpt: {addable: false}
  user-confirmations: {media: [text/markdown], require_anchor: message, require_frontmatter: [source]}
keys:
  type: {values: label, single: true}
  tags: {values: label}
  related-people: {values: person}
  related-places: {values: place}
  relationship: {map: person, values: label, symmetric: [friend], only_on: person}
  aliases: {values: text, only_on: entity}
labels:
  type: {entity: {}, concept: {}}
  tags: {software: {description: Software work}}
  relationship:
    friend: {}
    colleague: {synonyms: [coworker, co-worker]}
places:
  lyon: {title: Lyon}
";

    fn parse(text: &str) -> Result<Vocabulary> {
        Vocabulary::parse(text, PathBuf::from("vocabulary.yaml"))
    }

    fn problems(text: &str) -> Vec<String> {
        match parse(text) {
            Err(Error::InvalidVocabulary { problems, .. }) => problems,
            other => panic!("expected problems, got {other:?}"),
        }
    }

    #[test]
    fn example_loads_and_round_trips() {
        let vocabulary = parse(EXAMPLE).unwrap();
        assert_eq!(vocabulary.limits, Limits::default());
        assert!(!vocabulary.raw_collections["chatgpt"].addable);
        assert!(vocabulary.raw_collections["user-confirmations"].addable);
        assert_eq!(
            vocabulary.raw_collections["user-confirmations"].require_anchor,
            Some(AnchorKind::Message)
        );
        assert_eq!(
            vocabulary.key("relationship.alice").unwrap().map,
            Some(ValueKind::Person)
        );
        assert!(
            vocabulary
                .labels_of("relationship.alice")
                .unwrap()
                .contains_key("colleague")
        );
        let again = parse(&vocabulary.to_yaml()).unwrap();
        assert_eq!(again, vocabulary);
    }

    #[test]
    fn typos_and_inconsistencies_are_reported() {
        assert!(!problems("version: 1\npage_types: {}\nkeys: {}\nlabls: {}\n").is_empty());
        let found = problems(
            "version: 2\npage_types:\n  entity: {folder: people, index_section: ''}\n\
             required_keys: [missing]\nlimits: {ops_per_batch: 0}\n\
             raw_manifest: ../manifest.json\nraw_collections:\n  Bad: {media: [markdown]}\n\
             keys:\n  rel: {map: person, values: label, symmetric: [friend], only_on: nobody}\n\
             \x20 title: {values: text}\n  tags: {values: text}\n\
             labels:\n  rel: {friend: {synonyms: [pal]}, buddy: {synonyms: [Pal]}}\n  tags: {a: {}}\n\
             places:\n  Lyon: {}\n",
        );
        let joined = found.join("\n");
        for expected in [
            "version must be 1",
            "folder `people`",
            "needs an index_section",
            "required key `missing`",
            "limits must be positive",
            "raw_manifest",
            "raw collection `Bad`",
            "`markdown` is not a media type",
            "only_on `nobody`",
            "not `title`",
            "`pal` is used by both",
            "labels for `tags`",
            "places: `Lyon` must be lowercase",
        ] {
            assert!(
                joined.contains(expected),
                "missing `{expected}` in:\n{joined}"
            );
        }
    }

    #[test]
    fn a_wiki_without_a_vocabulary_gets_the_baseline() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Vocabulary::load(dir.path()).unwrap(),
            Vocabulary::baseline()
        );
    }

    #[test]
    fn baseline_follows_the_conventions_and_the_dictionary() {
        let baseline = Vocabulary::baseline();
        assert_eq!(
            baseline.page_types.keys().collect::<Vec<_>>(),
            ["comparison", "concept", "entity", "query"]
        );
        for key in ["title", "type", "created", "updated", "tags", "sources"] {
            assert!(baseline.required_keys.iter().any(|k| k == key), "{key}");
        }
        assert_eq!(baseline.required_on_create, ["confidence"]);
        // A wiki brings its own tag taxonomy and places.
        assert!(!baseline.labels.contains_key("tags"));
        assert!(baseline.places.is_empty());
        let dictionary = leaf_common::dictionary::dictionary();
        for (key, entry) in &dictionary.keys {
            let spec = baseline
                .keys
                .get(key)
                .unwrap_or_else(|| panic!("{key} missing"));
            assert_eq!(
                format!("{:?}", spec.values).to_lowercase(),
                entry.values,
                "{key}"
            );
            assert_eq!(
                spec.map.map(|m| format!("{m:?}").to_lowercase()),
                entry.map,
                "{key}"
            );
            assert_eq!(spec.single, entry.single, "{key}");
            assert!(spec.description.is_some(), "{key} has a description");
        }
        for (key, labels) in &dictionary.labels {
            for label in labels.keys() {
                assert!(baseline.labels[key].contains_key(label), "{key}: {label}");
            }
        }
    }

    #[test]
    fn extensions_add_to_the_baseline() {
        let path = PathBuf::from("vocabulary.yaml");
        let merged = Vocabulary::baseline_with(
            "required_keys: [related-people]
summary_headings: [Historical evidence]
\
             raw_collections:
  chatgpt: {addable: false}
\
             keys:
  sources: {values: raw, description: Our own wording}
  rooms: {values: label}
\
             labels:
  tags: {gardening: {}}
  relationship: {rival: {}}
places:
  paris: {title: Paris}
",
            path.clone(),
        )
        .unwrap();
        let baseline = Vocabulary::baseline();
        assert!(
            merged
                .required_keys
                .ends_with(&["related-people".to_string()])
        );
        assert!(
            baseline
                .required_keys
                .iter()
                .all(|k| merged.required_keys.contains(k))
        );
        assert!(!merged.raw_collections["chatgpt"].addable);
        assert!(merged.raw_collections.contains_key("articles"));
        assert_eq!(
            merged.keys["sources"].description.as_deref(),
            Some("Our own wording")
        );
        assert!(merged.keys.contains_key("rooms") && merged.keys.contains_key("aliases"));
        assert!(merged.labels["relationship"].contains_key("rival"));
        assert!(merged.labels["relationship"].contains_key("friend"));
        assert!(merged.labels["tags"].contains_key("gardening"));
        assert_eq!(merged.limits, baseline.limits);

        let extension = merged.extension_over_baseline();
        assert_eq!(Vocabulary::baseline().extended(extension.clone()), merged);
        assert!(extension.keys.contains_key("rooms") && !extension.keys.contains_key("tags"));

        // Typos and inconsistencies in an extension are rejected.
        assert!(Vocabulary::baseline_with("labls: {}\n", path.clone()).is_err());
        assert!(Vocabulary::baseline_with("required_keys: [nope]\n", path).is_err());
    }
}
