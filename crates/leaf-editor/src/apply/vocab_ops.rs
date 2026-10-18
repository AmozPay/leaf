//! Vocabulary operations of the planner: declaring keys, labels and places,
//! and adding synonyms.

use super::{Planner, Step};
use crate::suggest::{Lookup, Reason};
use crate::vocabulary::{Extension, KeySpec, Term, ValueKind, Vocabulary, base_key};

impl Planner<'_> {
    /// Apply a changed extension, unless it makes the vocabulary invalid.
    pub(super) fn extend(&mut self, number: usize, extension: Extension, declared: String) -> Step {
        let vocabulary = Vocabulary::baseline().extended(extension.clone());
        let problems = vocabulary.problems();
        if !problems.is_empty() {
            return Err(self.reject(number, problems.join("; ")));
        }
        self.vocabulary = vocabulary;
        self.declarations.extension = extension;
        self.declarations.changed = true;
        self.declarations.log.push(declared);
        Ok(())
    }

    /// The base key whose labels `key` uses; the operation is rejected if it
    /// takes none.
    pub(super) fn label_key(&mut self, number: usize, key: &str) -> Step<String> {
        let key = self.key(number, key)?;
        let base = base_key(&key).to_string();
        if self.vocabulary.keys.get(&base).map(|spec| spec.values) != Some(ValueKind::Label) {
            return Err(self.reject(
                number,
                format!("`{base}` does not take labels; declare places with declare_place"),
            ));
        }
        Ok(base)
    }

    /// Check that `value` is new for `key`: rejected if it already exists or
    /// means something declared; warns about look-alikes.
    pub(super) fn is_new(&mut self, number: usize, key: &str, value: &str) -> Step {
        match self.checker().check_value(key, value)? {
            Lookup::Known { value } => {
                Err(self.reject(number, format!("`{value}` is already declared for `{key}`")))
            }
            Lookup::Rewritten {
                value: canonical, ..
            } => Err(self.reject_with(
                number,
                format!("`{value}` already means `{canonical}` for `{key}`; use `{canonical}`"),
                vec![canonical],
            )),
            Lookup::Unknown { suggestions, .. } => {
                let similar: Vec<String> = suggestions
                    .into_iter()
                    .filter(|s| s.reason == Reason::Similar)
                    .map(|s| s.value)
                    .collect();
                if !similar.is_empty() {
                    self.outcome.warnings.push(format!(
                        "declared `{value}` for `{key}`, which looks like {}",
                        similar.join(", ")
                    ));
                }
                Ok(())
            }
            Lookup::Invalid { reason, .. } => {
                Err(self.reject(number, format!("`{value}` is {reason}")))
            }
        }
    }

    pub(super) fn declare_value(
        &mut self,
        number: usize,
        key: &str,
        value: &str,
        description: &Option<String>,
        synonyms: &[String],
    ) -> Step {
        let base = self.label_key(number, key)?;
        let value = value.trim().to_lowercase();
        self.is_new(number, &base, &value)?;
        let mut extension = self.declarations.extension.clone();
        extension.labels.entry(base.clone()).or_default().insert(
            value.clone(),
            Term {
                title: None,
                description: description.clone(),
                synonyms: synonyms.to_vec(),
            },
        );
        self.extend(number, extension, format!("{base} `{value}`"))
    }

    pub(super) fn declare_place(
        &mut self,
        number: usize,
        place: &str,
        title: &Option<String>,
        description: &Option<String>,
        synonyms: &[String],
    ) -> Step {
        let slug = slug::slugify(place);
        if slug.is_empty() {
            return Err(self.reject(number, "empty place"));
        }
        self.is_new(number, "related-places", &slug)?;
        let title = title
            .clone()
            .or_else(|| (place.trim() != slug).then(|| place.trim().to_string()));
        let mut extension = self.declarations.extension.clone();
        extension.places.insert(
            slug.clone(),
            Term {
                title,
                description: description.clone(),
                synonyms: synonyms.to_vec(),
            },
        );
        self.extend(number, extension, format!("place `{slug}`"))
    }

    pub(super) fn declare_key(&mut self, number: usize, key: &str, spec: KeySpec) -> Step {
        let key = key.trim().to_lowercase();
        if let Some(existing) = leaf_common::dictionary::dictionary().canonical_key(&key) {
            return Err(self.reject_with(
                number,
                format!("`{key}` already means `{existing}`; use `{existing}`"),
                vec![existing.to_string()],
            ));
        }
        if self.vocabulary.keys.contains_key(&key) {
            return Err(self.reject(number, format!("`{key}` is already declared")));
        }
        let mut extension = self.declarations.extension.clone();
        extension.keys.insert(key.clone(), spec);
        self.extend(number, extension, format!("key `{key}`"))
    }

    pub(super) fn add_synonym(
        &mut self,
        number: usize,
        key: &str,
        value: &str,
        synonym: &str,
    ) -> Step {
        let key = self.key(number, key)?;
        let base = base_key(&key).to_string();
        let value = value.trim().to_lowercase();
        let synonym = synonym.trim().to_lowercase();
        let places = self.vocabulary.keys.get(&base).map(|s| s.values) == Some(ValueKind::Place);
        let term = if places {
            self.vocabulary.places.get(&value).cloned()
        } else {
            self.vocabulary
                .labels_of(&base)
                .and_then(|l| l.get(&value))
                .cloned()
        };
        let Some(mut term) = term else {
            return Err(self.reject(number, format!("`{value}` is not declared for `{base}`")));
        };
        self.is_new(number, &key, &synonym)?;
        term.synonyms.push(synonym.clone());
        let mut extension = self.declarations.extension.clone();
        if places {
            extension.places.insert(value.clone(), term);
        } else {
            extension
                .labels
                .entry(base.clone())
                .or_default()
                .insert(value.clone(), term);
        }
        self.extend(
            number,
            extension,
            format!("synonym `{synonym}` of {base} `{value}`"),
        )
    }
}
