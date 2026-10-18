//! The editor's read side: vocabulary loading, key and value checks with
//! suggestions and the built-in dictionary, and metadata listing. Fixtures
//! are a small neutral wiki.

mod common;

use common::TempWiki;
use leaf_editor::{Editor, EditorConfig, Error, Lookup, Reason, Vocabulary};

const VOCABULARY: &str = "\
version: 1
page_types:
  entity: {folder: entities, index_section: Entities}
  concept: {folder: concepts, index_section: Concepts}
required_keys: [title, type, tags]
uncited_intro: true
raw_manifest: _meta/manifest.json
raw_collections:
  notes: {media: [text/markdown], require_anchor: message, require_frontmatter: [source]}
keys:
  type: {values: label, single: true}
  tags: {values: label}
  created: {values: date, single: true}
  contested: {values: boolean, single: true}
  related-people: {values: person}
  related-places: {values: place}
  relationship: {map: person, values: label, symmetric: [friend], only_on: person}
  aliases: {values: text}
  associated-organizations: {values: organization}
  sources: {values: raw}
labels:
  type: {entity: {}, concept: {}}
  tags: {software: {description: Software work}, unused-tag: {}}
  relationship:
    friend: {}
    colleague: {synonyms: [teammate-at-work]}
places:
  paris: {title: Paris}
";

const CITE: &str = "[[raw/notes/a.md#^message-a]]";

fn fixture() -> TempWiki {
    let wiki = TempWiki::new();
    wiki.write(
        "index.md",
        "[[entities/alice]] [[entities/bob-martin]] [[entities/acme]] [[concepts/greenhouse]]\n",
    );
    wiki.write(
        "entities/alice.md",
        &format!(
            "---\ntitle: Alice\ntype: entity\ntags: [person]\nrelated-people: [alice, bob-martin]\n\
             relationship: {{bob-martin: [friend, colleague]}}\n---\n# Alice\n\nAlice is the subject.\n\n\
             ## Work\n\nWorks on greenhouses. {CITE}\n"
        ),
    );
    wiki.write(
        "entities/bob-martin.md",
        &format!(
            "---\ntitle: Bob Martin\ntype: entity\ntags: [person]\nrelated-people: [alice]\naliases: [Bobby]\n\
             relationship: {{alice: [friend, colleague]}}\nassociated-organizations: [acme]\n---\n\
             # Bob Martin\n\n## Work\n\nMaintains the greenhouse. {CITE}\n\n\
             ## Historical evidence\n\nNot independently verified.\n"
        ),
    );
    wiki.write(
        "entities/acme.md",
        &format!("---\ntitle: Acme\ntype: entity\ntags: [organization]\n---\nAcme grows tomatoes. {CITE}\n"),
    );
    wiki.write(
        "concepts/greenhouse.md",
        &format!(
            "---\ntitle: Greenhouse\ntype: concept\ntags: [software, Gardening]\nrelated-people: [alice]\n\
             related-places: [paris]\ncreated: 2026-10-14\nsources: [raw/notes/a.md]\n---\n\
             # Greenhouse\n\nThe greenhouse is at Acme. {CITE}\n"
        ),
    );
    wiki.write(
        "raw/notes/a.md",
        "---\nsource: direct statement\ningested: 2026-10-17\nsha256: ab\n---\nIt is so.\n\n^message-a\n",
    );
    wiki.write(
        "_meta/manifest.json",
        "{\"created\": \"2026-10-14\", \"files\": [{\"path\": \"raw/notes/a.md\", \"bytes\": 1, \"sha256\": \"ab\"}]}",
    );
    wiki
}

fn config(wiki: &TempWiki) -> EditorConfig {
    EditorConfig::new(wiki.root(), wiki.db())
}

fn editor(wiki: &TempWiki) -> Editor {
    wiki.write("_meta/vocabulary.yaml", VOCABULARY);
    Editor::open(config(wiki)).unwrap()
}

fn known(value: &str) -> Lookup {
    Lookup::Known {
        value: value.into(),
    }
}

fn rewritten(lookup: Lookup) -> (String, Reason) {
    match lookup {
        Lookup::Rewritten { value, reason, .. } => (value, reason),
        other => panic!("expected a rewrite, got {other:?}"),
    }
}

fn suggestions(lookup: Lookup) -> Vec<String> {
    match lookup {
        Lookup::Unknown { suggestions, .. } => suggestions.into_iter().map(|s| s.value).collect(),
        other => panic!("expected unknown, got {other:?}"),
    }
}

#[test]
fn opening_uses_the_baseline_and_rejects_invalid_extensions() {
    let wiki = fixture();
    // No vocabulary file: the baseline applies.
    let editor = Editor::open(config(&wiki)).unwrap();
    assert_eq!(editor.vocabulary(), &Vocabulary::baseline());
    // An empty extension is the baseline too.
    wiki.write("_meta/vocabulary.yaml", "version: 1\n");
    assert_eq!(
        Editor::open(config(&wiki)).unwrap().vocabulary(),
        &Vocabulary::baseline()
    );
    // Typos and inconsistencies are rejected.
    for invalid in [
        "labls: {}\n",
        "keys:\n  tags: {values: nonsense}\n",
        "required_keys: [undeclared]\n",
    ] {
        wiki.write("_meta/vocabulary.yaml", invalid);
        assert!(
            matches!(
                Editor::open(config(&wiki)),
                Err(Error::InvalidVocabulary { .. })
            ),
            "{invalid}"
        );
    }
}

#[test]
fn keys_are_checked_with_the_dictionary_and_suggestions() {
    let wiki = fixture();
    let editor = editor(&wiki);
    let key = |key: &str| editor.check_key(key).unwrap();
    assert_eq!(key("tags"), known("tags"));
    assert_eq!(rewritten(key("Tags")).0, "tags");
    // Dictionary synonyms of declared keys are rewritten.
    for synonym in [
        "related-person",
        "persons",
        "individuals",
        "Related Individuals",
    ] {
        assert_eq!(
            rewritten(key(synonym)),
            ("related-people".into(), Reason::Synonym),
            "{synonym}"
        );
    }
    assert_eq!(rewritten(key("nicknames")).0, "aliases");
    assert_eq!(rewritten(key("locations")).0, "related-places");
    // Every dictionary key is in the baseline, so its synonyms work in any wiki.
    assert_eq!(key("related-events"), known("related-events"));
    assert_eq!(rewritten(key("happenings")).0, "related-events");
    assert_eq!(rewritten(key("companies")).0, "related-organizations");
    // Typos still get look-alikes.
    assert_eq!(suggestions(key("tgas")), ["tags"]);
    // Map entries: the entry name resolves like a person.
    assert_eq!(
        rewritten(key("relationship.bobby")).0,
        "relationship.bob-martin"
    );
    assert_eq!(key("relationship.alice"), known("relationship.alice"));
    assert!(matches!(key("relationship.nobody"), Lookup::Unknown { .. }));
    assert!(matches!(key("tags.x"), Lookup::Invalid { .. }));
}

#[test]
fn values_are_checked_by_kind() {
    let wiki = fixture();
    let editor = editor(&wiki);
    let value = |key: &str, value: &str| editor.check_value(key, value).unwrap();
    // Declared synonyms, dictionary synonyms, and dictionary labels.
    assert_eq!(
        rewritten(value("relationship.alice", "teammate-at-work")),
        ("colleague".into(), Reason::Synonym)
    );
    assert_eq!(
        rewritten(value("relationship.alice", "Coworker")),
        ("colleague".into(), Reason::Synonym)
    );
    assert_eq!(
        rewritten(value("relationship.alice", "spouse")),
        ("partner".into(), Reason::Synonym)
    );
    assert_eq!(
        rewritten(value("relationship.alice", "Buddies")).0,
        "friend"
    );
    assert_eq!(value("relationship.alice", "mentor"), known("mentor"));
    assert_eq!(
        suggestions(value("relationship.alice", "freind")),
        ["friend"]
    );
    // People and organizations by slug, title or alias.
    assert_eq!(
        rewritten(value("related-people", "Bob Martin")),
        ("bob-martin".into(), Reason::Normalized)
    );
    assert_eq!(rewritten(value("related-people", "bobby")).0, "bob-martin");
    assert_eq!(value("associated-organizations", "acme"), known("acme"));
    assert!(matches!(
        value("associated-organizations", "alice"),
        Lookup::Unknown { .. }
    ));
    // Places, dates, booleans, raw paths, text.
    assert_eq!(rewritten(value("related-places", "Paris")).0, "paris");
    assert!(matches!(
        value("related-places", "rome"),
        Lookup::Unknown { .. }
    ));
    assert!(matches!(
        value("created", "2026-10-17"),
        Lookup::Known { .. }
    ));
    assert!(matches!(
        value("created", "17/10/2026"),
        Lookup::Invalid { .. }
    ));
    assert_eq!(rewritten(value("contested", "yes")).0, "true");
    assert!(matches!(
        value("sources", "raw/notes/a.md"),
        Lookup::Known { .. }
    ));
    assert!(matches!(
        value("sources", "raw/missing.md"),
        Lookup::Invalid { .. }
    ));
    assert!(matches!(value("aliases", "Bob"), Lookup::Known { .. }));
    assert!(matches!(value("nonsense", "x"), Lookup::Invalid { .. }));
}

#[test]
fn metadata_listing_marks_what_is_declared() {
    let wiki = fixture();
    let editor = editor(&wiki);
    let listing = editor.list_metadata().unwrap();
    let key = |name: &str| listing.iter().find(|k| k.key == name).unwrap();

    let tags = key("tags");
    assert_eq!(tags.pages, 4);
    let tag = |name: &str| tags.values.iter().find(|v| v.value == name).unwrap();
    assert!(tag("software").declared);
    assert_eq!(
        tag("software").description.as_deref(),
        Some("Software work")
    );
    assert!(!tag("gardening").declared, "used but not declared");
    assert_eq!(tag("unused-tag").pages, 0);
    assert!(tag("unused-tag").declared);

    let labels = &key("relationship.alice").values;
    let colleague = labels.iter().find(|v| v.value == "colleague").unwrap();
    assert!(colleague.synonyms.contains(&"teammate-at-work".to_string()));
    assert!(colleague.synonyms.contains(&"co-worker".to_string()));
    let people: Vec<_> = key("relationship")
        .values
        .iter()
        .map(|v| v.value.as_str())
        .collect();
    assert_eq!(people, ["alice", "bob-martin"]);
    assert!(key("relationship").values.iter().all(|v| v.declared));
}
