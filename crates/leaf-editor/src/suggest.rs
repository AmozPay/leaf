//! Matching a key or value against the known vocabulary, with suggestions.
//!
//! A value that differs from a known one only in case or a leading `#`, or
//! that is a curated synonym or an entity's title or alias, is rewritten to
//! the canonical value. Anything else unknown is rejected with suggestions:
//! the same slug or singular/plural form when there is one (alone, so the
//! answer is unambiguous), else look-alikes by string similarity (`strsim`)
//! for typos.

use serde::Serialize;

/// Similarity below this is not suggested.
const MIN_SIMILARITY: f64 = 0.8;
/// Suggestions returned at most.
const MAX_SUGGESTIONS: usize = 3;

/// A known value and the other spellings that mean it.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub value: String,
    /// Curated synonyms, titles and aliases: rewritten to `value`.
    pub other_names: Vec<String>,
}

impl Candidate {
    #[cfg(test)]
    pub fn plain(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            other_names: Vec::new(),
        }
    }
}

/// The outcome of checking a key or value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Lookup {
    /// Known exactly as written.
    Known { value: String },
    /// Known under another spelling; use `value`.
    Rewritten {
        value: String,
        from: String,
        reason: Reason,
    },
    /// Not known. Use a suggestion, or declare it explicitly.
    Unknown {
        value: String,
        suggestions: Vec<Suggestion>,
    },
    /// Not acceptable for this key whatever the vocabulary (e.g. not a date).
    Invalid { value: String, reason: String },
}

impl Lookup {
    /// The value to store, if acceptable as is or after rewriting.
    pub fn accepted(&self) -> Option<&str> {
        match self {
            Self::Known { value } | Self::Rewritten { value, .. } => Some(value),
            Self::Unknown { .. } | Self::Invalid { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub value: String,
    pub reason: Reason,
    /// 1.0 for normalized matches; string similarity otherwise.
    pub score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    /// Same apart from case, a leading `#`, slug form, or singular/plural.
    Normalized,
    /// A curated synonym, or an entity's title or alias.
    Synonym,
    /// Looks alike (probably a typo).
    Similar,
}

/// Check `value` against `candidates`.
pub(crate) fn lookup(candidates: &[Candidate], value: &str) -> Lookup {
    let written = value.trim();
    let plain = written.strip_prefix('#').unwrap_or(written).trim();
    if plain.is_empty() {
        return Lookup::Invalid {
            value: value.into(),
            reason: "empty value".into(),
        };
    }
    // Compared in slug form: case, spacing, punctuation and accents do not
    // change what a word means.
    let wanted = comparable(plain);
    if let Some(candidate) = candidates.iter().find(|c| c.value == written) {
        return Lookup::Known {
            value: candidate.value.clone(),
        };
    }
    if let Some(candidate) = candidates.iter().find(|c| comparable(&c.value) == wanted) {
        return Lookup::Rewritten {
            value: candidate.value.clone(),
            from: value.into(),
            reason: Reason::Normalized,
        };
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|c| c.other_names.iter().any(|name| comparable(name) == wanted))
    {
        return Lookup::Rewritten {
            value: candidate.value.clone(),
            from: value.into(),
            reason: Reason::Synonym,
        };
    }
    Lookup::Unknown {
        value: value.into(),
        suggestions: suggest(candidates, plain),
    }
}

fn suggest(candidates: &[Candidate], value: &str) -> Vec<Suggestion> {
    let wanted = comparable(value);
    let mut suggestions: Vec<Suggestion> = candidates
        .iter()
        .filter_map(|candidate| {
            std::iter::once(&candidate.value)
                .chain(&candidate.other_names)
                .map(|name| score(&wanted, &comparable(name)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .filter(|(score, _)| *score >= MIN_SIMILARITY)
                .map(|(score, reason)| Suggestion {
                    value: candidate.value.clone(),
                    reason,
                    score,
                })
        })
        .collect();
    suggestions.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.value.cmp(&b.value))
    });
    suggestions.dedup_by(|a, b| a.value == b.value);
    // A normalized match is the answer; look-alikes would only distract.
    if suggestions.iter().any(|s| s.reason == Reason::Normalized) {
        suggestions.retain(|s| s.reason == Reason::Normalized);
    }
    suggestions.truncate(MAX_SUGGESTIONS);
    suggestions
}

/// Lowercase slug form, so `Related Person` and `related-person` compare equal.
fn comparable(text: &str) -> String {
    slug::slugify(text)
}

fn score(a: &str, b: &str) -> (f64, Reason) {
    if a == b || singular(a) == singular(b) {
        return (1.0, Reason::Normalized);
    }
    let similarity = strsim::jaro_winkler(a, b).max(strsim::normalized_damerau_levenshtein(a, b));
    (similarity, Reason::Similar)
}

/// The slug with its last word singular (`related-people` → `related-person`).
fn singular(slug: &str) -> String {
    let (head, last) = slug.rsplit_once('-').map_or(("", slug), |(h, l)| (h, l));
    let last = pluralizer::pluralize(last, 1, false);
    if head.is_empty() {
        last
    } else {
        format!("{head}-{last}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<Candidate> {
        vec![
            Candidate::plain("friend"),
            Candidate {
                value: "colleague".into(),
                other_names: vec!["coworker".into(), "co-worker".into()],
            },
            Candidate {
                value: "bob-martin".into(),
                other_names: vec!["Bob Martin".into(), "Bobby".into()],
            },
            Candidate::plain("related-people"),
            Candidate::plain("associated-organizations"),
        ]
    }

    fn suggested(value: &str) -> Vec<(String, Reason)> {
        match lookup(&candidates(), value) {
            Lookup::Unknown { suggestions, .. } => suggestions
                .into_iter()
                .map(|s| (s.value, s.reason))
                .collect(),
            other => panic!("expected unknown, got {other:?}"),
        }
    }

    fn rewrite(value: &str) -> (String, Reason) {
        match lookup(&candidates(), value) {
            Lookup::Rewritten { value, reason, .. } => (value, reason),
            other => panic!("expected a rewrite, got {other:?}"),
        }
    }

    #[test]
    fn known_values_and_rewrites() {
        assert_eq!(
            lookup(&candidates(), "friend"),
            Lookup::Known {
                value: "friend".into()
            }
        );
        // Same slug: case, spacing and `#` do not matter.
        for written in ["Friend", "#friend"] {
            assert_eq!(
                rewrite(written),
                ("friend".into(), Reason::Normalized),
                "{written}"
            );
        }
        assert_eq!(
            rewrite("Related People "),
            ("related-people".into(), Reason::Normalized)
        );
        assert_eq!(
            rewrite("Bob Martin"),
            ("bob-martin".into(), Reason::Normalized)
        );
        // Other names: synonyms, titles and aliases.
        for written in ["Coworker", "CO-WORKER"] {
            assert_eq!(
                rewrite(written),
                ("colleague".into(), Reason::Synonym),
                "{written}"
            );
        }
        assert_eq!(rewrite("bobby"), ("bob-martin".into(), Reason::Synonym));
        assert!(matches!(
            lookup(&candidates(), " # "),
            Lookup::Invalid { .. }
        ));
    }

    #[test]
    fn unknown_values_get_ranked_suggestions() {
        assert_eq!(
            suggested("related-person"),
            [("related-people".to_string(), Reason::Normalized)]
        );
        assert_eq!(
            suggested("associated-organization")[0],
            ("associated-organizations".to_string(), Reason::Normalized)
        );
        assert_eq!(
            suggested("freind")[0],
            ("friend".to_string(), Reason::Similar)
        );
        assert_eq!(
            suggested("colleage")[0],
            ("colleague".to_string(), Reason::Similar)
        );
        assert_eq!(suggested("co-workers")[0].0, "colleague");
        assert!(suggested("greenhouse").is_empty());
    }
}
