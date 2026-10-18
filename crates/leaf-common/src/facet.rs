//! Frontmatter facets: page metadata such as `tags`, `related-people`,
//! `related-places` or `roles`, stored per page and usable as search filters.
//!
//! Every frontmatter key except `title` whose value is a scalar, a list of
//! scalars, or a one-level map of those is a facet. Keys are lowercased. Values keep their spelling for
//! display and match case-insensitively; tags are also lowercased for display
//! and lose a leading `#`.

/// Topic tags.
pub const TAGS: &str = "tags";
/// Slugs of person entity pages (`entities/<slug>.md`) a page is about.
pub const PEOPLE: &str = "related-people";
/// Geographic slugs a page relates to.
pub const PLACES: &str = "related-places";
/// Other names of a person, on their entity page.
pub const ALIASES: &str = "aliases";
/// A person's relationships, keyed by the other person's slug:
/// `relationship: {alice: [friend, colleague]}`.
pub const RELATIONSHIP: &str = "relationship";

/// The facet key a filter kind names, given the indexed `keys`, in order:
/// the key itself; the built-in dictionary's key for it (`person`,
/// `persons`, `individuals` → `related-people`) if indexed; by convention
/// `related-<kind>s`, `related-<kind>` or `<kind>s`; the only key ending in
/// `-<kind>s` or `-<kind>` (`organization` finds `associated-organizations`);
/// else the dictionary's key, or the kind as written. Map entry keys
/// (`relationship.x`) only match exactly.
pub fn resolve_kind(kind: &str, keys: &[String]) -> String {
    let kind = normalize_key(kind);
    if keys.contains(&kind) {
        return kind;
    }
    let known = crate::dictionary::dictionary()
        .canonical_key(&kind)
        .map(String::from);
    if let Some(key) = known.as_ref().filter(|key| keys.contains(key)) {
        return key.clone();
    }
    let conventional = [
        format!("related-{kind}s"),
        format!("related-{kind}"),
        format!("{kind}s"),
    ]
    .into_iter()
    .find(|candidate| keys.contains(candidate));
    if let Some(key) = conventional {
        return key;
    }
    let suffixes = [format!("-{kind}s"), format!("-{kind}")];
    let mut suffixed = keys
        .iter()
        .filter(|key| !key.contains('.') && suffixes.iter().any(|suffix| key.ends_with(suffix)));
    match (suffixed.next(), suffixed.next()) {
        (Some(key), None) => key.clone(),
        _ => known.unwrap_or(kind),
    }
}

/// Whether a key's values are organization slugs (e.g.
/// `associated-organizations`), resolved like people's names.
pub fn names_organizations(key: &str) -> bool {
    key.ends_with("organizations") || key.ends_with("organization")
}
/// List-shaped facets: a map or deeper value there is a sync warning. Other
/// keys may hold one-level maps (`relationship: {alice: [friend]}`), indexed
/// as `key` = the map keys and `key.<name>` = each entry's values; anything
/// deeper is skipped silently.
pub const WARN_IF_MALFORMED: [&str; 4] = [TAGS, PEOPLE, PLACES, ALIASES];

pub fn normalize_key(key: &str) -> String {
    key.trim().to_lowercase()
}

/// The form a value is matched by.
pub fn normalize_value(key: &str, value: &str) -> String {
    let value = value.trim();
    let value = if key == TAGS {
        value.strip_prefix('#').unwrap_or(value).trim()
    } else {
        value
    };
    value.to_lowercase()
}

/// Values as stored and shown: trimmed, without empties or case-insensitive
/// duplicates, in their written order (tags: normalized and sorted).
pub fn clean_values<S: AsRef<str>>(key: &str, values: impl IntoIterator<Item = S>) -> Vec<String> {
    let mut seen = Vec::new();
    let mut cleaned = Vec::new();
    for value in values {
        let norm = normalize_value(key, value.as_ref());
        if norm.is_empty() || seen.contains(&norm) {
            continue;
        }
        let shown = if key == TAGS {
            norm.clone()
        } else {
            value.as_ref().trim().to_string()
        };
        seen.push(norm);
        cleaned.push(shown);
    }
    if key == TAGS {
        cleaned.sort();
    }
    cleaned
}

/// Normalized, deduplicated filter values.
pub fn filter_values<S: AsRef<str>>(key: &str, values: &[S]) -> Vec<String> {
    let mut normalized: Vec<String> = values
        .iter()
        .map(|value| normalize_value(key, value.as_ref()))
        .filter(|value| !value.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_cleaned_per_key() {
        assert_eq!(
            clean_values(TAGS, ["Software", "#dns", "software", " "]),
            ["dns", "software"]
        );
        assert_eq!(
            clean_values(ALIASES, [" Bobby ", "bobby", "Bob"]),
            ["Bobby", "Bob"]
        );
        assert_eq!(normalize_value(PEOPLE, " Chloe-Dubois "), "chloe-dubois");
        assert_eq!(filter_values(TAGS, &["#B", "a", "b", ""]), ["a", "b"]);
    }

    #[test]
    fn kinds_resolve_to_keys() {
        let keys: Vec<String> = [
            "roles",
            "related-organizations",
            "relationship",
            "relationship.chloe-dubois",
            "events",
            "associated-teams",
            "home-team",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(resolve_kind("Roles", &keys), "roles");
        assert_eq!(resolve_kind("person", &keys), PEOPLE);
        assert_eq!(resolve_kind("place", &keys), PLACES);
        assert_eq!(resolve_kind("tag", &keys), TAGS);
        assert_eq!(resolve_kind("organization", &keys), "related-organizations");
        assert_eq!(resolve_kind("event", &keys), "events");
        assert_eq!(
            resolve_kind("relationship.alice", &keys),
            "relationship.alice"
        );
        assert_eq!(resolve_kind("unknown", &keys), "unknown");
        // A unique `-<kind>s` key; ambiguous suffixes and map entries do not match.
        let associated: Vec<String> = ["associated-organizations".to_string()].to_vec();
        assert_eq!(
            resolve_kind("organization", &associated),
            "associated-organizations"
        );
        assert_eq!(resolve_kind("team", &keys), "team");
        assert_eq!(resolve_kind("carol", &keys), "carol");
        // Dictionary synonyms.
        assert_eq!(resolve_kind("persons", &keys), PEOPLE);
        assert_eq!(resolve_kind("individuals", &keys), PEOPLE);
        assert_eq!(resolve_kind("locations", &keys), PLACES);
        assert_eq!(resolve_kind("nicknames", &keys), ALIASES);
        assert_eq!(resolve_kind("companies", &keys), "related-organizations");
    }
}
