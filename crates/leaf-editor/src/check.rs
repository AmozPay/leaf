//! Checking metadata keys and values against a vocabulary and the wiki's
//! index. Batches check against the vocabulary as they change it, so the
//! checks take the vocabulary as a parameter rather than the editor's own.

use crate::error::Result;
use crate::suggest::{self, Candidate, Lookup};
use crate::vocabulary::{ValueKind, Vocabulary, base_key};
use leaf_common::publication::entity_slug;
use leaf_viewer::WikiIndex;
use std::path::Path;

/// A page a batch creates or retitles, as the checks see it before it is
/// written, so later operations can refer to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedPage {
    pub path: String,
    pub title: String,
    /// Other names of the page (`aliases`).
    pub aliases: Vec<String>,
    /// An entity page that is a person (has a `relationship` map, or is
    /// named in `related-people`).
    pub person: bool,
}

impl StagedPage {
    /// The names the page answers to: its title, then its aliases.
    fn names(&self) -> Vec<String> {
        std::iter::once(self.title.clone())
            .chain(self.aliases.iter().cloned())
            .collect()
    }
}

/// Checks against one vocabulary and the wiki's index, as a batch in
/// progress would leave it.
pub(crate) struct Checker<'a> {
    pub vocabulary: &'a Vocabulary,
    pub index: &'a WikiIndex,
    pub root: &'a Path,
    /// Pages the batch has created or changed (they replace the index's
    /// entries for the same path).
    pub staged: &'a [StagedPage],
    /// Paths the batch removes from the published pages.
    pub gone: &'a [String],
    /// Raw files the batch adds.
    pub staged_raw: &'a [String],
}

impl Checker<'_> {
    pub fn vocabulary(&self) -> &Vocabulary {
        self.vocabulary
    }

    pub fn index(&self) -> &WikiIndex {
        self.index
    }

    pub fn wiki_root(&self) -> &Path {
        self.root
    }

    /// Check a metadata key. Map entries (`relationship.<person>`) are
    /// checked as the map key plus an entry name of the map's kind.
    pub fn check_key(&self, key: &str) -> Result<Lookup> {
        let written = key.trim().to_lowercase();
        let keys = self.key_candidates();
        let Some((base, entry)) = written.split_once('.') else {
            return Ok(suggest::lookup(&keys, key));
        };
        let base_lookup = suggest::lookup(&keys, base);
        let Some(base_value) = base_lookup.accepted() else {
            return Ok(base_lookup);
        };
        let Some(kind) = self.vocabulary.keys[base_value].map else {
            return Ok(Lookup::Invalid {
                value: key.into(),
                reason: format!("`{base_value}` is not a map key, so it has no `.` entries"),
            });
        };
        let entry_lookup = suggest::lookup(&self.candidates(kind, base_value)?, entry);
        let joined = |entry: &str| format!("{base_value}.{entry}");
        Ok(match (&base_lookup, entry_lookup) {
            (Lookup::Known { .. }, Lookup::Known { value }) => Lookup::Known {
                value: joined(&value),
            },
            (_, Lookup::Known { value }) | (_, Lookup::Rewritten { value, .. }) => {
                Lookup::Rewritten {
                    value: joined(&value),
                    from: key.into(),
                    reason: match &base_lookup {
                        Lookup::Rewritten { reason, .. } => *reason,
                        _ => suggest::Reason::Normalized,
                    },
                }
            }
            (_, Lookup::Unknown { suggestions, .. }) => Lookup::Unknown {
                value: key.into(),
                suggestions: suggestions
                    .into_iter()
                    .map(|mut s| {
                        s.value = joined(&s.value);
                        s
                    })
                    .collect(),
            },
            (_, Lookup::Invalid { reason, .. }) => Lookup::Invalid {
                value: key.into(),
                reason,
            },
        })
    }

    /// Declared keys, with the built-in dictionary's synonyms for them
    /// (`persons` for `related-people`).
    fn key_candidates(&self) -> Vec<Candidate> {
        let dictionary = leaf_common::dictionary::dictionary();
        self.vocabulary
            .keys
            .keys()
            .map(|key| Candidate {
                value: key.clone(),
                other_names: dictionary
                    .keys
                    .get(key)
                    .map(|entry| entry.synonyms.clone())
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// Check a value for a key; the key must be acceptable first.
    pub fn check_value(&self, key: &str, value: &str) -> Result<Lookup> {
        let key_lookup = self.check_key(key)?;
        let Some(key) = key_lookup.accepted() else {
            return Ok(Lookup::Invalid {
                value: value.into(),
                reason: format!("`{key}` is not a known key"),
            });
        };
        let Some(spec) = self.vocabulary.key(key) else {
            return Ok(Lookup::Invalid {
                value: value.into(),
                reason: format!("`{key}` is not a known key"),
            });
        };
        let written = value.trim();
        let invalid = |reason: &str| Lookup::Invalid {
            value: value.into(),
            reason: reason.into(),
        };
        Ok(match spec.values {
            ValueKind::Text if written.is_empty() => invalid("empty value"),
            ValueKind::Text => Lookup::Known {
                value: written.into(),
            },
            ValueKind::Date => match written.parse::<jiff::civil::Date>() {
                Ok(date) => known_or_rewritten(date.to_string(), value),
                Err(_) => invalid("not an ISO date such as 2026-10-17"),
            },
            ValueKind::Boolean => match written.to_lowercase().as_str() {
                "true" | "yes" | "on" => known_or_rewritten("true".into(), value),
                "false" | "no" | "off" => known_or_rewritten("false".into(), value),
                _ => invalid("not true or false"),
            },
            ValueKind::Raw => match written.parse::<leaf_common::Citation>() {
                Ok(citation) if citation.anchor.is_none() => {
                    if self.root.join(&citation.raw_path).is_file()
                        || self.staged_raw.contains(&citation.raw_path)
                    {
                        known_or_rewritten(citation.raw_path, value)
                    } else {
                        invalid("no such raw file")
                    }
                }
                _ => invalid("not a raw/<path> file reference"),
            },
            kind => suggest::lookup(&self.candidates(kind, key)?, value),
        })
    }

    fn is_staged(&self, path: &str) -> bool {
        self.staged.iter().any(|page| page.path == path)
    }

    fn is_published(&self, path: &str) -> bool {
        !self.gone.iter().any(|gone| gone == path)
    }

    /// Every published page as the batch leaves it: its path and the names
    /// it answers to (its title first, then aliases).
    pub fn pages(&self) -> Result<Vec<(String, Vec<String>)>> {
        let mut pages: Vec<(String, Vec<String>)> = self
            .index
            .pages()?
            .into_iter()
            .filter(|page| self.is_published(&page.path) && !self.is_staged(&page.path))
            .map(|page| (page.path, vec![page.title]))
            .collect();
        pages.extend(
            self.staged
                .iter()
                .filter(|page| self.is_published(&page.path))
                .map(|page| (page.path.clone(), page.names())),
        );
        Ok(pages)
    }

    /// The known values of `kind` for `key`.
    pub fn candidates(&self, kind: ValueKind, key: &str) -> Result<Vec<Candidate>> {
        let terms =
            |terms: Option<&std::collections::BTreeMap<String, crate::vocabulary::Term>>| {
                terms
                    .into_iter()
                    .flatten()
                    .map(|(value, term)| Candidate {
                        value: value.clone(),
                        other_names: term.synonyms.clone(),
                    })
                    .collect::<Vec<_>>()
            };
        Ok(match kind {
            ValueKind::Label => with_dictionary_labels(terms(self.vocabulary.labels_of(key)), key),
            ValueKind::Place => terms(Some(&self.vocabulary.places)),
            // Only people with an entity page: a name merely mentioned in
            // `related-people` is not a person the wiki knows.
            ValueKind::Person => {
                let mut people: Vec<Candidate> =
                    self.index
                        .people()?
                        .into_iter()
                        .filter(|person| {
                            person.page_path.as_ref().is_some_and(|path| {
                                self.is_published(path) && !self.is_staged(path)
                            })
                        })
                        .map(|person| Candidate {
                            value: person.slug,
                            other_names: person.title.into_iter().chain(person.aliases).collect(),
                        })
                        .collect();
                people.extend(
                    self.staged
                        .iter()
                        .filter(|page| page.person)
                        .filter_map(|page| {
                            Some(Candidate {
                                value: entity_slug(&page.path)?.to_string(),
                                other_names: page.names(),
                            })
                        }),
                );
                people
            }
            ValueKind::Organization | ValueKind::Entity => {
                let people: Vec<String> =
                    self.index.people()?.into_iter().map(|p| p.slug).collect();
                let staged_people = |slug: &str| {
                    self.staged
                        .iter()
                        .any(|page| page.person && entity_slug(&page.path) == Some(slug))
                };
                self.pages()?
                    .into_iter()
                    .filter_map(|(path, names)| {
                        let slug = entity_slug(&path)?;
                        let is_person = if self.is_staged(&path) {
                            staged_people(slug)
                        } else {
                            people.iter().any(|p| p == slug) || staged_people(slug)
                        };
                        (kind == ValueKind::Entity || !is_person).then(|| Candidate {
                            value: slug.to_string(),
                            other_names: names,
                        })
                    })
                    .collect()
            }
            ValueKind::Page => self
                .pages()?
                .into_iter()
                .map(|(path, mut names)| Candidate {
                    other_names: {
                        names.truncate(1);
                        names.insert(0, path.trim_end_matches(".md").to_string());
                        names
                    },
                    value: path,
                })
                .collect(),
            ValueKind::Raw | ValueKind::Date | ValueKind::Boolean | ValueKind::Text => Vec::new(),
        })
    }
}

/// Add the built-in dictionary's common labels for `key` (relationship
/// types, confidence levels) and their synonyms to the declared ones.
fn with_dictionary_labels(mut candidates: Vec<Candidate>, key: &str) -> Vec<Candidate> {
    let base = base_key(key);
    let Some(labels) = leaf_common::dictionary::dictionary().labels.get(base) else {
        return candidates;
    };
    for (label, synonyms) in labels {
        match candidates.iter_mut().find(|c| c.value == *label) {
            Some(candidate) => candidate.other_names.extend(synonyms.iter().cloned()),
            None => candidates.push(Candidate {
                value: label.clone(),
                other_names: synonyms.clone(),
            }),
        }
    }
    candidates
}

fn known_or_rewritten(value: String, written: &str) -> Lookup {
    if value == written {
        Lookup::Known { value }
    } else {
        Lookup::Rewritten {
            value,
            from: written.into(),
            reason: suggest::Reason::Normalized,
        }
    }
}
