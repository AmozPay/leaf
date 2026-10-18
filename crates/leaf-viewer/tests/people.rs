//! People and places from frontmatter facets, and generic facet filters.

mod common;

use common::Fixture;
use leaf_viewer::{FacetMatch, SearchRequest, SearchResponse, WikiIndex};

fn person(title: &str, extra: &str) -> String {
    format!(
        "---\ntitle: {title}\ntype: entity\ntags: [person]\n{extra}---\n# {title}\n\n{title} works on orchards.\n"
    )
}

fn concept(title: &str, people: &str, places: &str) -> String {
    format!(
        "---\ntitle: {title}\ntype: concept\ntags: [projects]\nrelated-people: {people}\nrelated-places: {places}\n---\n\
         # {title}\n\n{title} orchard claim.\n"
    )
}

fn fixture() -> (Fixture, WikiIndex) {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[entities/alice]] [[entities/bob-martin]] [[entities/chloe-dubois]] \
         [[concepts/acme]] [[concepts/lyon-years]] [[concepts/solo]]",
    );
    wiki.write(
        "entities/alice.md",
        &person(
            "Alice",
            "related-people: [alice]\nrelated-places: [lyon]\nrelationship:\n  bob-martin: [friend, colleague]\n  chloe-dubois: [friend]\n",
        ),
    );
    wiki.write(
        "entities/bob-martin.md",
        &person(
            "Bob Martin",
            "related-people: [alice, chloe-dubois]\nrelated-places: []\nrelationship: {alice: [friend, colleague]}\n\
             aliases: [Bobby]\nroles: [greenhouse-maintainer]\nassociated-organizations: [acme]\n",
        ),
    );
    wiki.write(
        "entities/chloe-dubois.md",
        &person(
            "Chloé Dubois",
            "relationship:\n  alice: [friend]\naliases: [Clo]\nroles: [CTO]\nrelated-organizations: [Acme]\n",
        ),
    );
    wiki.write(
        "concepts/acme.md",
        &concept(
            "Acme",
            "[alice, bob-martin, chloe-dubois]",
            "[lyon, france]",
        ),
    );
    wiki.write(
        "concepts/lyon-years.md",
        &concept("Lyon years", "[alice, jane-doe]", "[Lyon]"),
    );
    wiki.write("concepts/solo.md", &concept("Solo", "[alice]", "[]"));
    let index = wiki.synced();
    (wiki, index)
}

fn pages(response: &SearchResponse) -> Vec<String> {
    let mut pages: Vec<_> = response.hits.iter().map(|h| h.page_path.clone()).collect();
    pages.sort();
    pages.dedup();
    pages
}

fn find(index: &WikiIndex, request: SearchRequest) -> Vec<String> {
    pages(&index.search(&request).unwrap())
}

fn all(query: &str) -> SearchRequest {
    SearchRequest::new(query, 50, 100_000)
}

#[test]
fn people_directory_joins_mentions_with_entity_pages() {
    let (_wiki, index) = fixture();
    let people = index.people().unwrap();
    let slugs: Vec<_> = people.iter().map(|p| (p.slug.as_str(), p.pages)).collect();
    assert_eq!(
        slugs,
        [
            ("alice", 5),
            ("chloe-dubois", 2),
            ("bob-martin", 1),
            ("jane-doe", 1)
        ]
    );
    let bobby = &people[2];
    assert_eq!(bobby.title.as_deref(), Some("Bob Martin"));
    assert_eq!(bobby.page_path.as_deref(), Some("entities/bob-martin.md"));
    assert_eq!(bobby.aliases, ["Bobby"]);
    assert_eq!(bobby.metadata["relationship"], ["alice"]);
    assert_eq!(
        bobby.metadata["relationship.alice"],
        ["friend", "colleague"]
    );
    assert_eq!(bobby.metadata["roles"], ["greenhouse-maintainer"]);
    assert_eq!(bobby.metadata["associated-organizations"], ["acme"]);
    // Named but without a published entity page.
    assert_eq!(people[3].title, None);
    assert_eq!(people[3].page_path, None);
}

#[test]
fn people_filter_by_slug_title_alias_or_name() {
    let (_wiki, index) = fixture();
    let bob = ["concepts/acme.md"];
    for name in ["bob-martin", "Bob Martin", "bobby", "BOBBY"] {
        assert_eq!(
            find(&index, all("orchard").with_people([name], FacetMatch::All)),
            bob,
            "{name}"
        );
    }
    let chloe = ["concepts/acme.md", "entities/bob-martin.md"];
    for name in ["Chloé Dubois", "chloe dubois", "Clo"] {
        assert_eq!(
            find(&index, all("orchard").with_people([name], FacetMatch::All)),
            chloe,
            "{name}"
        );
    }
    assert_eq!(
        find(
            &index,
            all("orchard").with_people(["bobby", "jane-doe"], FacetMatch::Any)
        ),
        ["concepts/acme.md", "concepts/lyon-years.md"]
    );
    assert!(
        find(
            &index,
            all("orchard").with_people(["bobby", "jane-doe"], FacetMatch::All)
        )
        .is_empty()
    );
    assert!(
        find(
            &index,
            all("orchard").with_people(["nobody"], FacetMatch::Any)
        )
        .is_empty()
    );
}

#[test]
fn places_filter_and_count_case_insensitively() {
    let (_wiki, index) = fixture();
    let places: Vec<_> = index
        .places()
        .unwrap()
        .into_iter()
        .map(|p| (p.value.to_lowercase(), p.pages))
        .collect();
    assert_eq!(places, [("lyon".to_string(), 3), ("france".to_string(), 1)]);
    assert_eq!(
        find(&index, all("").with_places(["LYON"], FacetMatch::All)),
        [
            "concepts/acme.md",
            "concepts/lyon-years.md",
            "entities/alice.md"
        ]
    );
    assert_eq!(
        find(
            &index,
            all("").with_places(["lyon", "france"], FacetMatch::All)
        ),
        ["concepts/acme.md"]
    );
}

#[test]
fn different_facets_combine_with_and() {
    let (_wiki, index) = fixture();
    let request = all("orchard")
        .with_people(["alice"], FacetMatch::All)
        .with_places(["lyon"], FacetMatch::All)
        .with_tags(["projects"], FacetMatch::All);
    assert_eq!(
        find(&index, request),
        ["concepts/acme.md", "concepts/lyon-years.md"]
    );
    let none = all("orchard")
        .with_people(["bobby"], FacetMatch::All)
        .with_places(["lyon"], FacetMatch::Any)
        .with_tags(["person"], FacetMatch::All);
    assert!(find(&index, none).is_empty());
}

#[test]
fn any_frontmatter_key_is_a_facet() {
    let (_wiki, index) = fixture();
    assert_eq!(
        find(
            &index,
            all("").with_facet("Roles", ["cto"], FacetMatch::All)
        ),
        ["entities/chloe-dubois.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("relationship.alice", ["friend"], FacetMatch::All)
        ),
        ["entities/bob-martin.md", "entities/chloe-dubois.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("relationship.alice", ["colleague"], FacetMatch::All)
        ),
        ["entities/bob-martin.md"]
    );
    let keys: Vec<_> = index
        .facets()
        .unwrap()
        .into_iter()
        .map(|k| k.value)
        .collect();
    for key in [
        "type",
        "tags",
        "related-people",
        "related-places",
        "aliases",
        "roles",
        "relationship",
        "relationship.alice",
    ] {
        assert!(keys.iter().any(|k| k == key), "{key} in {keys:?}");
    }
    assert!(!keys.iter().any(|k| k == "title"));
    let types: Vec<_> = index
        .facet_values("type")
        .unwrap()
        .into_iter()
        .map(|v| (v.value, v.pages))
        .collect();
    assert_eq!(
        types,
        [("concept".to_string(), 3), ("entity".to_string(), 3)]
    );
}

#[test]
fn hits_carry_people_and_places() {
    let (_wiki, index) = fixture();
    let response = index
        .search(&all("acme").with_tags(["projects"], FacetMatch::All))
        .unwrap();
    let hit = &response.hits[0];
    assert_eq!(hit.people, ["alice", "bob-martin", "chloe-dubois"]);
    assert_eq!(hit.places, ["lyon", "france"]);
    assert_eq!(hit.tags, ["projects"]);
}

#[test]
fn filter_kinds_resolve_by_shorthand_and_convention() {
    let (_wiki, index) = fixture();
    // `person` and `place` are shorthands; `organization` finds
    // `related-organizations` by convention; exact keys work as written.
    assert_eq!(
        find(
            &index,
            all("").with_facet("person", ["bobby"], FacetMatch::All)
        ),
        ["concepts/acme.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("place", ["france"], FacetMatch::All)
        ),
        ["concepts/acme.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("organization", ["acme"], FacetMatch::All)
        ),
        ["entities/chloe-dubois.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("tag", ["person"], FacetMatch::All)
        )
        .len(),
        3
    );
}

#[test]
fn relationships_resolve_people_in_keys_and_values() {
    let (_wiki, index) = fixture();
    // People related to Bobby (by alias), and Alice's friends named by title.
    assert_eq!(
        find(
            &index,
            all("").with_facet("relationship", ["Bobby"], FacetMatch::All)
        ),
        ["entities/alice.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("relationship", ["alice"], FacetMatch::All)
        ),
        ["entities/bob-martin.md", "entities/chloe-dubois.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("relationship.Alice", ["friend"], FacetMatch::All)
        ),
        ["entities/bob-martin.md", "entities/chloe-dubois.md"]
    );
    assert_eq!(
        find(
            &index,
            all("").with_facet("relationship.Chloé Dubois", ["friend"], FacetMatch::All)
        ),
        ["entities/alice.md"]
    );
}

#[test]
fn modes_mix_per_kind_and_exclude() {
    let (_wiki, index) = fixture();
    // All of these people, any of these places.
    let mixed = all("orchard")
        .with_people(["alice", "bobby"], FacetMatch::All)
        .with_places(["paris", "france"], FacetMatch::Any);
    assert_eq!(find(&index, mixed), ["concepts/acme.md"]);
    // Pages about Alice that are not about Bobby.
    let without = all("")
        .with_people(["alice"], FacetMatch::All)
        .with_people(["bobby"], FacetMatch::Not);
    assert_eq!(
        find(&index, without),
        [
            "concepts/lyon-years.md",
            "concepts/solo.md",
            "entities/alice.md",
            "entities/bob-martin.md"
        ]
    );
    // A page without the facet at all passes a NOT filter.
    let no_lyon = all("orchard").with_places(["lyon"], FacetMatch::Not);
    assert_eq!(
        find(&index, no_lyon),
        [
            "concepts/solo.md",
            "entities/bob-martin.md",
            "entities/chloe-dubois.md"
        ]
    );
    // Exclusion alone lists the remaining pages.
    let not_entities = all("").with_facet("type", ["entity"], FacetMatch::Not);
    assert_eq!(find(&index, not_entities).len(), 3);
}

#[test]
fn organizations_resolve_like_people() {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[entities/acme]] [[entities/lumen-school]] [[entities/bobby]] [[concepts/school]]",
    );
    wiki.write(
        "entities/acme.md",
        "---\ntitle: Acme\ntype: entity\naliases: [Acme SAS]\n---\nAcme builds orchards.\n",
    );
    wiki.write(
        "entities/lumen-school.md",
        "---\ntitle: Lumen School\ntype: entity\nrelated-places: [lyon]\n---\nA school with orchards.\n",
    );
    wiki.write(
        "entities/bobby.md",
        "---\ntitle: Bobby\ntype: entity\nrelationship: {alice: [friend]}\n\
         associated-organizations: [acme, lumen-school]\n---\nBobby tends orchards.\n",
    );
    wiki.write(
        "concepts/school.md",
        "---\ntitle: School\nassociated-organizations: [Lumen-School]\n---\nSchool orchards.\n",
    );
    let index = wiki.synced();
    let by = |values: &[&str]| {
        find(
            &index,
            all("").with_facet("organization", values.iter().copied(), FacetMatch::All),
        )
    };
    // `organization` finds `associated-organizations`; names resolve by
    // slug, title, alias or a unique word.
    assert_eq!(by(&["acme"]), ["entities/bobby.md"]);
    assert_eq!(by(&["Acme SAS"]), ["entities/bobby.md"]);
    assert_eq!(
        by(&["Lumen School"]),
        ["concepts/school.md", "entities/bobby.md"]
    );
    assert_eq!(by(&["lumen"]), ["concepts/school.md", "entities/bobby.md"]);
    assert_eq!(by(&["lumen", "Acme"]), ["entities/bobby.md"]);
    // Entities that are people are not organizations.
    assert!(by(&["bobby"]).is_empty());
    // A place is not resolved against entities: `lyon` stays `lyon`.
    assert_eq!(
        find(&index, all("").with_places(["lyon"], FacetMatch::All)),
        ["entities/lumen-school.md"]
    );
}
