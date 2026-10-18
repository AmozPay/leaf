//! Batches: what an agent asks the editor to do, and what happened.
//!
//! A batch is plain data (YAML or JSON) so any client can send one. Unknown
//! fields and operations are rejected, so a typo is an error rather than a
//! silently ignored instruction.

use crate::vocabulary::ValueKind;
use indexmap::IndexMap;
use leaf_viewer::SyncReport;
use serde::{Deserialize, Serialize};

/// A set of operations applied together, or not at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    /// The agent making the batch (recorded in `log.md`).
    pub agent: String,
    /// The model the agent runs on (recorded in `log.md`).
    pub model: String,
    pub kind: BatchKind,
    /// The `log.md` heading.
    pub summary: String,
    /// `log.md` bullets explaining the batch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub ops: Vec<Op>,
}

impl Batch {
    /// Read a batch from YAML or JSON.
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_saphyr::from_str(text).map_err(|err| {
            let message = err.to_string();
            format!(
                "invalid batch: {}",
                message.lines().next().unwrap_or_default()
            )
        })
    }
}

/// The kind of change, as the `log.md` entry type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchKind {
    Update,
    Correction,
    Review,
    Merge,
}

impl BatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Correction => "correction",
            Self::Review => "review",
            Self::Merge => "merge",
        }
    }
}

/// One operation. Values are checked against the vocabulary: synonyms are
/// rewritten (and reported), unknown values rejected with suggestions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Replace a key's values. Map entries are `relationship.<person>`.
    SetMetadata {
        page: String,
        key: String,
        #[serde(deserialize_with = "texts")]
        values: Vec<String>,
    },
    /// Add values to a key, keeping the ones it has.
    AddMetadata {
        page: String,
        key: String,
        #[serde(deserialize_with = "texts")]
        values: Vec<String>,
    },
    /// Remove some values of a key, or the whole key when `values` is absent.
    RemoveMetadata {
        page: String,
        key: String,
        #[serde(
            default,
            deserialize_with = "optional_texts",
            skip_serializing_if = "Option::is_none"
        )]
        values: Option<Vec<String>>,
    },
    /// Add a passage: one line of plain text and its citations, after the
    /// last passage under `under` (a heading, or the page title for the
    /// introduction) or right after the passage `after`.
    AddPassage {
        page: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        under: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        citations: Vec<String>,
    },
    /// Rewrite a passage. Every citation it had must stay, unless listed in
    /// `drop_citations`.
    ReplacePassage {
        passage: String,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        citations: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        drop_citations: Vec<String>,
    },
    /// Remove a passage; the reason goes to `log.md`.
    RemovePassage { passage: String, reason: String },
    /// Add a citation to a passage.
    AddCitation { passage: String, citation: String },
    /// Create a page of a page type: frontmatter in the vocabulary's key order
    /// (`created` and `updated` are today), an introduction and sections of
    /// cited passages, and a line in `index.md`.
    CreatePage(NewPage),
    /// Change a page's title (and its top heading, if that is the old title).
    SetTitle { page: String, title: String },
    /// Change the description of a page in `index.md`.
    SetSummary { page: String, summary: String },
    /// Add an empty heading: at `level` (2 to 6, default 2), at the end of
    /// the section of the heading `after`, or at the end of the page.
    AddHeading {
        page: String,
        heading: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
    },
    /// Rename a heading. Its passages get new IDs (reported), because the
    /// heading is part of their ID.
    RenameHeading {
        page: String,
        heading: String,
        to: String,
    },
    /// Move a page to `_archive/` with `archived` and `archived-reason`, and
    /// take it off `index.md`. Refused while other pages still refer to it
    /// (links, people, organizations, pages in their metadata), unless the
    /// same batch removes those references.
    ArchivePage { page: String, reason: String },
    /// Move an archived page back (`page` is its `_archive/…` path or its
    /// original path) and put it on `index.md` again; it must pass the checks
    /// as a new page would. `summary` is its `index.md` description when the
    /// archive did not keep one.
    RestorePage {
        page: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    /// Add a raw source: `text` or `base64` (for a binary file), under a
    /// declared, addable `raw/<collection>/`. A Markdown source gets
    /// `source`/`source_url` as given, `ingested` (today) and the `sha256` of
    /// its body in its frontmatter. Raw files are never changed or removed.
    AddRaw {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base64: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_url: Option<String>,
    },
    /// Declare a new label for a `label` key (a tag, a role, a relationship type).
    DeclareValue {
        key: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        synonyms: Vec<String>,
    },
    /// Declare a new place slug.
    DeclarePlace {
        place: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        synonyms: Vec<String>,
    },
    /// Declare a new metadata key.
    DeclareKey {
        key: String,
        values: ValueKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map: Option<ValueKind>,
        #[serde(default)]
        single: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        only_on: Option<String>,
        description: String,
    },
    /// Add a spelling that is rewritten to a declared label or place.
    AddSynonym {
        key: String,
        value: String,
        synonym: String,
    },
}

/// A page to create (the fields of `create_page`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPage {
    /// A page type of the vocabulary (`concept`, `entity`, …).
    #[serde(rename = "type")]
    pub page_type: String,
    /// Lowercase, hyphenated; the file is `<folder>/<slug>.md`.
    pub slug: String,
    pub title: String,
    /// The one-line description after the title in `index.md`.
    pub summary: String,
    /// Frontmatter keys other than the ones the editor sets itself. A value
    /// is a text or a list of texts.
    #[serde(
        default,
        deserialize_with = "metadata",
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub metadata: IndexMap<String, Vec<String>>,
    /// Passages under the page title, before the first section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intro: Vec<NewPassage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
}

/// A passage of a new page: one line of text and its citations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPassage {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<String>,
}

/// A second-level heading of a new page and its passages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub heading: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passages: Vec<NewPassage>,
}

/// Frontmatter keys with their values, each a text or a list of texts of any
/// YAML scalar type.
fn metadata<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<IndexMap<String, Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Values {
        Many(Vec<serde_json::Value>),
        One(serde_json::Value),
    }
    let map: IndexMap<String, Values> = Deserialize::deserialize(deserializer)?;
    let text = |value: serde_json::Value| match value {
        serde_json::Value::String(text) => Ok(text),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Ok(value.to_string()),
        other => Err(serde::de::Error::custom(format!(
            "a value must be text, not {other}"
        ))),
    };
    map.into_iter()
        .map(|(key, values)| {
            let values = match values {
                Values::One(value) => vec![text(value)?],
                Values::Many(values) => values.into_iter().map(text).collect::<Result<_, _>>()?,
            };
            Ok((key, values))
        })
        .collect()
}

/// Values as text, whatever YAML scalar they were written as (`yes`,
/// `2024`, `true`), so a small model's unquoted value is not rejected.
fn texts<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let values: Vec<serde_json::Value> = Deserialize::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| match value {
            serde_json::Value::String(text) => Ok(text),
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Ok(value.to_string()),
            other => Err(serde::de::Error::custom(format!(
                "a value must be text, not {other}"
            ))),
        })
        .collect()
}

fn optional_texts<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    texts(deserializer).map(Some)
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SetMetadata { .. } => "set_metadata",
            Self::AddMetadata { .. } => "add_metadata",
            Self::RemoveMetadata { .. } => "remove_metadata",
            Self::AddPassage { .. } => "add_passage",
            Self::ReplacePassage { .. } => "replace_passage",
            Self::RemovePassage { .. } => "remove_passage",
            Self::AddCitation { .. } => "add_citation",
            Self::CreatePage { .. } => "create_page",
            Self::SetTitle { .. } => "set_title",
            Self::SetSummary { .. } => "set_summary",
            Self::AddHeading { .. } => "add_heading",
            Self::RenameHeading { .. } => "rename_heading",
            Self::AddRaw { .. } => "add_raw",
            Self::ArchivePage { .. } => "archive_page",
            Self::RestorePage { .. } => "restore_page",
            Self::DeclareValue { .. } => "declare_value",
            Self::DeclarePlace { .. } => "declare_place",
            Self::DeclareKey { .. } => "declare_key",
            Self::AddSynonym { .. } => "add_synonym",
        }
    }
}

/// What happened to a batch.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BatchOutcome {
    /// Whether the files were written. Always false for a check.
    pub applied: bool,
    /// Why the batch was rejected; empty when it is acceptable.
    pub errors: Vec<OpError>,
    /// Problems the changed pages already had before the batch, and
    /// other things worth knowing; they do not block the batch.
    pub warnings: Vec<String>,
    /// Values written differently from how they were given (synonyms,
    /// spelling), so the agent learns the canonical form.
    pub rewrites: Vec<Rewrite>,
    /// Files the batch writes, relative to the wiki root.
    pub changed: Vec<String>,
    /// Files the batch removes (a page moves to or from `_archive/`).
    pub removed: Vec<String>,
    /// IDs of the passages the batch adds or rewrites, to address them next.
    pub passages: Vec<PassageId>,
    /// The index sync after an applied batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncReport>,
}

impl BatchOutcome {
    pub fn is_acceptable(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Why an operation, or the batch, is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpError {
    /// The operation's position (from 0), if the error is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<usize>,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

/// The ID a passage has after an operation (`<page>#p-<hash>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PassageId {
    pub op: usize,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rewrite {
    pub op: usize,
    pub from: String,
    pub to: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_parse_strictly() {
        let batch = Batch::parse(
            "agent: curator\nmodel: small-8b\nkind: update\nsummary: Tags\nops:\n\
             - {op: add_metadata, page: concepts/garden.md, key: tags, values: [gardening]}\n\
             - {op: remove_metadata, page: concepts/garden.md, key: contested}\n\
             - {op: declare_key, key: rooms, values: label, description: Rooms of a house}\n",
        )
        .unwrap();
        assert_eq!(batch.ops.len(), 3);
        assert_eq!(batch.ops[1].name(), "remove_metadata");
        let typed = Batch::parse(
            "agent: a\nmodel: m\nkind: update\nsummary: s\n\
             ops: [{op: set_metadata, page: x, key: k, values: [yes, 2024, Paris]}]\n",
        )
        .unwrap();
        assert!(matches!(
            &typed.ops[0],
            Op::SetMetadata { values, .. } if values == &["true", "2024", "Paris"]
        ));
        // The same batch as JSON.
        let json = serde_json::to_string(&batch).unwrap();
        assert_eq!(Batch::parse(&json).unwrap(), batch);
        for bad in [
            "agent: a\nmodel: m\nkind: update\nsummary: s\nops: []\nextra: 1\n",
            "agent: a\nmodel: m\nkind: tidy\nsummary: s\nops: []\n",
            "agent: a\nmodel: m\nkind: update\nsummary: s\nops: [{op: delete_page, page: x}]\n",
            "agent: a\nmodel: m\nkind: update\nsummary: s\nops: [{op: add_metadata, page: x, key: k, value: [v]}]\n",
            "model: m\nkind: update\nsummary: s\nops: []\n",
            "agent: a\nmodel: m\nkind: update\nsummary: s\nops: [{op: add_metadata, page: x, key: k, values: [[nested]]}]\n",
        ] {
            assert!(Batch::parse(bad).is_err(), "{bad}");
        }
    }
}
