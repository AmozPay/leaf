//! The built-in synonym dictionary for metadata, so a wiki does not have to
//! define one.
//!
//! It lists suggested metadata keys (`related-people`, `related-places`,
//! `aliases`, …) and common labels (relationship types, confidence levels),
//! each with the spellings that mean it: `persons`, `individuals` and
//! `related-persons` are `related-people`; `co-worker` is `colleague`.
//! Synonyms come from Open English WordNet for one chosen sense of each
//! keyword (see `data/keywords.yaml`), so they can be replaced safely; a
//! spelling that could mean two keywords is left out.
//!
//! Generated data: `data/synonyms.yaml`, from Open English WordNet 2025
//! (CC BY 4.0) and Princeton WordNet (WordNet License).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static DATA: &str = include_str!("../data/synonyms.yaml");
static DICTIONARY: LazyLock<Dictionary> = LazyLock::new(|| {
    serde_saphyr::from_str(DATA).expect("the bundled data/synonyms.yaml is valid")
});

/// The built-in dictionary.
pub fn dictionary() -> &'static Dictionary {
    &DICTIONARY
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dictionary {
    /// Where the synonyms come from, and their licenses.
    pub source: String,
    /// Suggested metadata keys.
    pub keys: BTreeMap<String, DictionaryKey>,
    /// Common labels by key, each with its synonyms.
    #[serde(default)]
    pub labels: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictionaryKey {
    pub description: String,
    /// What values are: `label`, `person`, `organization`, `place`, `page`,
    /// `raw`, `date`, `boolean` or `text`.
    pub values: String,
    /// For a map key (`relationship: {person: [labels]}`): what entries are named by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<String>,
    /// At most one value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub single: bool,
    /// Spellings meaning this key, as slugs.
    #[serde(default)]
    pub synonyms: Vec<String>,
}

impl Dictionary {
    /// The suggested key a word means (`Persons` → `related-people`), if any.
    pub fn canonical_key(&self, word: &str) -> Option<&str> {
        let slug = slug::slugify(word);
        self.keys
            .iter()
            .find(|(key, spec)| **key == slug || spec.synonyms.contains(&slug))
            .map(|(key, _)| key.as_str())
    }

    /// The common label a word means for `key` (`relationship`, `co-worker`
    /// → `colleague`), if any. Map entries (`relationship.x`) use the map
    /// key's labels.
    pub fn canonical_label(&self, key: &str, word: &str) -> Option<&str> {
        let base = key.split_once('.').map_or(key, |(base, _)| base);
        let slug = slug::slugify(word);
        self.labels
            .get(base)?
            .iter()
            .find(|(label, synonyms)| **label == slug || synonyms.contains(&slug))
            .map(|(label, _)| label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_dictionary_resolves_synonyms() {
        let dictionary = dictionary();
        assert!(dictionary.source.contains("WordNet"));
        for (word, key) in [
            ("related-people", "related-people"),
            ("Persons", "related-people"),
            ("related-individuals", "related-people"),
            ("locations", "related-places"),
            ("related-companies", "related-organizations"),
            ("nicknames", "aliases"),
            ("pseudonym", "aliases"),
            ("references", "sources"),
            ("last-modified", "updated"),
            ("keywords", "tags"),
        ] {
            assert_eq!(dictionary.canonical_key(word), Some(key), "{word}");
        }
        for (word, label) in [
            ("co-worker", "colleague"),
            ("coworker", "colleague"),
            ("Buddies", "friend"),
            ("spouse", "partner"),
            ("advisor", "mentor"),
            ("kin", "family"),
        ] {
            assert_eq!(
                dictionary.canonical_label("relationship.someone", word),
                Some(label),
                "{word}"
            );
        }
        // Senses that are not the keyword's meaning are not synonyms.
        assert_eq!(dictionary.canonical_label("relationship", "enemy"), None);
        assert_eq!(
            dictionary.canonical_label("relationship", "acquaintance"),
            Some("acquaintance")
        );
        assert_eq!(dictionary.canonical_key("job"), None);
        assert_eq!(dictionary.canonical_key("soul"), None);
        assert_eq!(
            dictionary.canonical_label("confidence", "moderate"),
            Some("medium")
        );
    }

    #[test]
    fn no_spelling_means_two_keywords() {
        let dictionary = dictionary();
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for (key, spec) in &dictionary.keys {
            for spelling in std::iter::once(key).chain(&spec.synonyms) {
                assert_eq!(
                    *seen.entry(spelling).or_insert(key),
                    key.as_str(),
                    "{spelling}"
                );
            }
        }
        for labels in dictionary.labels.values() {
            let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
            for (label, synonyms) in labels {
                for spelling in std::iter::once(label).chain(synonyms) {
                    assert_eq!(
                        *seen.entry(spelling).or_insert(label),
                        label.as_str(),
                        "{spelling}"
                    );
                }
            }
        }
    }
}
