//! Entity pages (`entities/<slug>.md`): the people directory (slugs named in
//! `related-people`, joined with their pages), the other entities such as
//! organizations, and resolution of a name to an entity slug.

use crate::model::{Person, Result};
use crate::store::Store;
use leaf_common::facet::{ALIASES, PEOPLE, RELATIONSHIP};
use leaf_common::publication::{ENTITIES_DIR, entity_slug};
use std::collections::BTreeMap;

/// Every person named in `related-people`, most mentioned first, plus person
/// entity pages nobody names yet.
pub(crate) fn directory(store: &Store) -> Result<Vec<Person>> {
    let mut people: BTreeMap<String, Person> = BTreeMap::new();
    for named in store.facet_values(PEOPLE)? {
        let slug = named.value.to_lowercase();
        people
            .entry(slug.clone())
            .or_insert_with(|| empty(slug))
            .pages += named.pages;
    }
    for (path, title) in store.pages_under(ENTITIES_DIR)? {
        let Some(slug) = entity_slug(&path) else {
            continue;
        };
        let mut metadata = store.page_facets(&path)?;
        // An entity page is a person when someone names it in
        // `related-people`, or when it has a `relationship` map.
        if !people.contains_key(slug) && !metadata.contains_key(RELATIONSHIP) {
            continue;
        }
        let person = people
            .entry(slug.to_string())
            .or_insert_with(|| empty(slug.to_string()));
        person.page_path = Some(path.clone());
        person.title = Some(title);
        person.aliases = metadata.remove(ALIASES).unwrap_or_default();
        person.metadata = metadata;
    }
    let mut people: Vec<Person> = people.into_values().collect();
    people.sort_by(|a, b| b.pages.cmp(&a.pages).then_with(|| a.slug.cmp(&b.slug)));
    Ok(people)
}

fn empty(slug: String) -> Person {
    Person {
        slug,
        page_path: None,
        title: None,
        aliases: Vec::new(),
        metadata: BTreeMap::new(),
        pages: 0,
    }
}

/// What a name can match: an entity's slug, title and aliases.
#[derive(Debug, Clone, Default)]
pub(crate) struct Entity {
    pub slug: String,
    pub title: Option<String>,
    pub aliases: Vec<String>,
}

impl From<&Person> for Entity {
    fn from(person: &Person) -> Self {
        Self {
            slug: person.slug.clone(),
            title: person.title.clone(),
            aliases: person.aliases.clone(),
        }
    }
}

/// Published entity pages that are not people (organizations, projects, …).
pub(crate) fn other_entities(store: &Store, people: &[Person]) -> Result<Vec<Entity>> {
    let mut entities = Vec::new();
    for (path, title) in store.pages_under(ENTITIES_DIR)? {
        let Some(slug) = entity_slug(&path) else {
            continue;
        };
        if people.iter().any(|person| person.slug == slug) {
            continue;
        }
        let aliases = store
            .page_facets(&path)?
            .remove(ALIASES)
            .unwrap_or_default();
        entities.push(Entity {
            slug: slug.to_string(),
            title: Some(title),
            aliases,
        });
    }
    Ok(entities)
}

/// The slug a name refers to among `entities`: a slug, title or alias
/// (ignoring case); else an entity one of whose slug or title words is the
/// name, if only one is (`chloe` is `chloe-dubois`); else the name's own
/// slug (`Chloé Dubois` is `chloe-dubois`).
pub(crate) fn resolve(entities: &[Entity], name: &str) -> String {
    let wanted = name.trim().to_lowercase();
    let slugged = slug::slugify(name);
    let exact = |entity: &&Entity| {
        entity.slug == wanted
            || entity.slug == slugged
            || entity
                .title
                .as_deref()
                .is_some_and(|title| title.to_lowercase() == wanted)
            || entity
                .aliases
                .iter()
                .any(|alias| alias.to_lowercase() == wanted)
    };
    if let Some(entity) = entities.iter().find(exact) {
        return entity.slug.clone();
    }
    let has_word = |entity: &&Entity| {
        let title = entity
            .title
            .as_deref()
            .map(slug::slugify)
            .unwrap_or_default();
        !slugged.is_empty()
            && [entity.slug.as_str(), title.as_str()]
                .iter()
                .any(|words| words.split('-').any(|word| word == slugged))
    };
    let mut partial = entities.iter().filter(has_word);
    match (partial.next(), partial.next()) {
        (Some(entity), None) => entity.slug.clone(),
        _ => slugged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_by_slug_title_or_alias() {
        let bobby = Entity {
            slug: "bob-martin".into(),
            title: Some("Bob Martin".into()),
            aliases: vec!["Bobby".into()],
        };
        let alice = Entity {
            slug: "alice".into(),
            ..Entity::default()
        };
        let people = [bobby, alice];
        for name in ["bob-martin", "Bob Martin", "bobby", " BOBBY "] {
            assert_eq!(resolve(&people, name), "bob-martin", "{name}");
        }
        assert_eq!(resolve(&people, "Alice"), "alice");
        assert_eq!(resolve(&people, "Chloé Dubois"), "chloe-dubois");
        // One unique word of the slug or title.
        assert_eq!(resolve(&people, "bob"), "bob-martin");
        assert_eq!(resolve(&people, "Martin"), "bob-martin");
        let other = Entity {
            slug: "bob-durand".into(),
            title: Some("Bob Durand".into()),
            aliases: Vec::new(),
        };
        let crowded = [people[0].clone(), other];
        assert_eq!(resolve(&crowded, "bob"), "bob");
        assert_eq!(resolve(&crowded, "durand"), "bob-durand");
    }
}
