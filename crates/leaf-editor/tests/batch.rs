//! Batches end to end on a temporary wiki: what is written, what is
//! rejected (leaving every byte unchanged), and the bookkeeping.

mod common;

use common::{EditorExt, TempWiki, batch, errors, rejected, snapshot};
use leaf_editor::Batch;
use leaf_viewer::{FacetMatch, SearchRequest};

const CITE: &str = "[[raw/notes/a.md#^message-a]]";

fn wiki() -> TempWiki {
    let wiki = TempWiki::new();
    wiki.write(
        "raw/notes/a.md",
        "---\nsource: direct statement\ningested: 2026-10-01\nsha256: ab\n---\n### user\n\nIt is so.\n\n^message-a\n",
    );
    wiki.write(
        "index.md",
        "[[concepts/garden]] [[entities/alice]] [[entities/bob-martin]]\n",
    );
    wiki.write(
        "log.md",
        "# Wiki Log\n\n## [2026-10-01] create | Wiki initialized\n",
    );
    wiki.write(
        "_meta/vocabulary.yaml",
        "labels:\n  tags: {person: {}, plants: {}}\nplaces:\n  paris: {title: Paris}\n",
    );
    let page = |title: &str, kind: &str, extra: &str, body: &str| {
        format!(
            "---\ntitle: {title}\ncreated: 2026-10-01\nupdated: 2026-10-02\ntype: {kind}\n\
             tags: [person]\nsources: [raw/notes/a.md]\nconfidence: medium\n{extra}---\n{body}"
        )
    };
    wiki.write(
        "concepts/garden.md",
        &page(
            "Garden",
            "concept",
            "",
            &format!("# Garden\n\nAn introduction.\n\n## Beds\n\nBeds are raised. {CITE}\n\n## Old notes\n\nUncited claim.\n"),
        ),
    );
    wiki.write(
        "entities/alice.md",
        &page(
            "Alice",
            "entity",
            "related-people: [alice]\nrelationship: {bob-martin: [colleague]}\n",
            &format!("# Alice\n\n## Work\n\nAlice gardens. {CITE}\n"),
        ),
    );
    wiki.write(
        "entities/bob-martin.md",
        &page(
            "Bob Martin",
            "entity",
            "aliases: [Bobby]\nrelationship: {alice: [colleague]}\n",
            &format!("# Bob Martin\n\n## Work\n\nBob digs. {CITE}\n"),
        ),
    );
    wiki
}

#[test]
fn a_batch_declares_and_uses_a_tag_and_keeps_the_books() {
    let wiki = wiki();
    let mut editor = wiki.editor();
    let outcome = editor
        .apply(&batch(
            "- {op: declare_value, key: tags, value: Gardening, description: Growing plants}\n\
             - {op: add_metadata, page: concepts/garden, key: tags, values: [gardening, Plants]}\n\
             - {op: set_metadata, page: concepts/garden.md, key: confidence, values: [strong]}\n\
             - {op: add_metadata, page: concepts/garden, key: locations, values: [Paris]}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{outcome:#?}");
    assert_eq!(
        outcome.changed,
        ["concepts/garden.md", "_meta/vocabulary.yaml", "log.md"]
    );
    // Synonyms and spellings are rewritten, and reported.
    let rewrites: Vec<_> = outcome
        .rewrites
        .iter()
        .map(|r| (r.from.as_str(), r.to.as_str()))
        .collect();
    assert!(rewrites.contains(&("Plants", "plants")), "{rewrites:?}");
    assert!(rewrites.contains(&("strong", "high")), "{rewrites:?}");
    assert!(
        rewrites.contains(&("locations", "related-places")),
        "{rewrites:?}"
    );
    // The page: frontmatter changed in place, `updated` set, body untouched.
    let page = wiki.read("concepts/garden.md");
    assert!(
        page.starts_with(
            "---\ntitle: Garden\ncreated: 2026-10-01\nupdated: 2026-10-17\ntype: concept\n\
         tags: [person, gardening, plants]\nsources: [raw/notes/a.md]\nconfidence: high\n\
         related-places: [paris]\n---\n# Garden\n\nAn introduction.\n"
        ),
        "{page}"
    );
    // The pre-existing uncited section is a warning, not a rejection.
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("Old notes") && w.contains("already before"))
    );
    // The vocabulary extension and the log.
    let vocabulary = wiki.read("_meta/vocabulary.yaml");
    assert!(
        vocabulary.contains("    gardening:\n      description: Growing plants\n"),
        "{vocabulary}"
    );
    let log = wiki.read("log.md");
    assert!(log.starts_with("# Wiki Log\n\n## [2026-10-01] create"));
    assert!(
        log.ends_with(
            "\n## [2026-10-17] update | Test batch\n- By curator (small-8b).\n- Because.\n\
         - Changed: `concepts/garden.md`.\n- Declared: tags `gardening`.\n"
        ),
        "{log}"
    );
    // The index is in sync with the change.
    let hits = editor
        .index()
        .search(&SearchRequest::new("", 5, 10_000).with_tags(["gardening"], FacetMatch::All))
        .unwrap();
    assert_eq!(hits.hits[0].page_path, "concepts/garden.md");
    assert!(!wiki.root().join("_meta/.leaf-editor/journal.json").exists());
}

#[test]
fn symmetric_relationships_are_mirrored_both_ways() {
    let wiki = wiki();
    let mut editor = wiki.editor();
    let outcome = editor
        .apply(&batch(
            "- {op: add_metadata, page: entities/alice, key: relationship.bobby, values: [Buddy]}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{outcome:#?}");
    assert_eq!(
        outcome.changed,
        ["entities/alice.md", "entities/bob-martin.md", "log.md"]
    );
    assert!(
        wiki.read("entities/alice.md")
            .contains("relationship:\n  bob-martin: [colleague, friend]\n")
    );
    assert!(
        wiki.read("entities/bob-martin.md")
            .contains("relationship:\n  alice: [colleague, friend]\n")
    );
    let outcome = editor
        .apply(&batch(
            "- {op: remove_metadata, page: entities/bob-martin, key: relationship.alice, values: [colleague, friend]}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{outcome:#?}");
    assert!(!wiki.read("entities/alice.md").contains("relationship"));
    assert!(!wiki.read("entities/bob-martin.md").contains("relationship"));
}

#[test]
fn contested_is_written_only_when_true() {
    let wiki = wiki();
    let mut editor = wiki.editor();
    let set = editor
        .apply(&batch(
            "- {op: set_metadata, page: concepts/garden, key: contradictions, values: [entities/alice]}\n\
             - {op: set_metadata, page: concepts/garden, key: disputed, values: [yes]}\n",
        ))
        .unwrap();
    assert!(set.applied, "{set:#?}");
    assert!(
        wiki.read("concepts/garden.md")
            .contains("contested: true\n")
    );
    let unset = editor
        .apply(&batch(
            "- {op: set_metadata, page: concepts/garden, key: contested, values: [false]}\n",
        ))
        .unwrap();
    assert!(unset.applied, "{unset:#?}");
    assert!(!wiki.read("concepts/garden.md").contains("contested"));
}

#[test]
fn a_contested_page_must_name_the_pages_it_contradicts() {
    let wiki = wiki();
    let problems = rejected(
        &wiki,
        "- {op: set_metadata, page: concepts/garden, key: contested, values: [true]}\n",
    );
    assert!(problems.contains("needs `contradictions`"), "{problems}");
}

#[test]
fn a_long_page_is_a_warning_not_a_rejection() {
    let wiki = wiki();
    let mut page = wiki.read("concepts/garden.md");
    for n in 0..210 {
        page.push_str(&format!("\nExtra intro line {n}. {CITE}\n"));
    }
    wiki.write("concepts/garden.md", &page);
    let outcome = wiki
        .editor()
        .apply(&batch(
            "- {op: add_metadata, page: concepts/garden, key: tags, values: [plants]}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{outcome:#?}");
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("concepts/garden.md") && w.contains("consider splitting")),
        "{:?}",
        outcome.warnings
    );
}

#[test]
fn checking_writes_nothing() {
    let wiki = wiki();
    let before = snapshot(&wiki.root());
    let editor = wiki.editor();
    let outcome = editor
        .check(&batch(
            "- {op: add_metadata, page: concepts/garden, key: tags, values: [plants]}\n",
        ))
        .unwrap();
    assert!(outcome.is_acceptable() && !outcome.applied, "{outcome:#?}");
    assert_eq!(outcome.changed, ["concepts/garden.md", "log.md"]);
    assert_eq!(snapshot(&wiki.root()), before);
}

#[test]
fn unknown_or_forbidden_changes_are_rejected_untouched() {
    let wiki = wiki();
    let cases = [
        (
            "- {op: add_metadata, page: concepts/garden, key: tags, values: [orchards]}\n",
            "declare it with declare_value",
        ),
        (
            "- {op: add_metadata, page: concepts/garden, key: tgas, values: [plants]}\n",
            "unknown key `tgas`",
        ),
        (
            "- {op: add_metadata, page: concepts/garden, key: places, values: [rome]}\n",
            "declare_place",
        ),
        (
            "- {op: add_metadata, page: concepts/garden, key: related-people, values: [nobody]}\n",
            "existing page",
        ),
        (
            "- {op: set_metadata, page: concepts/garden, key: updated, values: [2020-01-01]}\n",
            "set by the editor",
        ),
        (
            "- {op: set_metadata, page: concepts/garden, key: type, values: [entity]}\n",
            "set by the editor",
        ),
        (
            "- {op: set_metadata, page: entities/alice, key: relationship, values: [bob-martin]}\n",
            "is a map",
        ),
        (
            "- {op: set_metadata, page: concepts/garden, key: confidence, values: [high, low]}\n",
            "takes one value",
        ),
        (
            "- {op: add_metadata, page: concepts/missing, key: tags, values: [plants]}\n",
            "not a published page",
        ),
        (
            "- {op: remove_metadata, page: concepts/garden, key: sources}\n",
            "missing required `sources`",
        ),
        (
            "- {op: add_metadata, page: concepts/garden, key: tags, values: [person]}\n",
            "changes nothing",
        ),
        (
            "- {op: declare_value, key: tags, value: plants}\n",
            "already declared",
        ),
        (
            "- {op: declare_value, key: relationship, value: coworker}\n",
            "already means `colleague`",
        ),
        (
            "- {op: declare_key, key: persons, values: person, description: People}\n",
            "already means `related-people`",
        ),
        (
            "- {op: declare_value, key: related-places, value: rome}\n",
            "declare places with declare_place",
        ),
    ];
    for (ops, expected) in cases {
        let message = rejected(&wiki, ops);
        assert!(
            message.contains(expected),
            "{ops}\nexpected `{expected}` in:\n{message}"
        );
    }
}

#[test]
fn batch_level_rules_are_enforced() {
    let wiki = wiki();
    let before = snapshot(&wiki.root());
    let mut editor = wiki.editor();
    let no_agent = Batch::parse(
        "agent: ' '\nmodel: small-8b\nkind: update\nsummary: S\nops:\n- {op: add_metadata, page: concepts/garden, key: tags, values: [plants]}\n",
    )
    .unwrap();
    assert!(errors(&editor.apply(&no_agent).unwrap()).contains("`agent` is required"));
    let empty = Batch::parse("agent: a\nmodel: m\nkind: update\nsummary: S\nops: []\n").unwrap();
    assert!(errors(&editor.apply(&empty).unwrap()).contains("no operations"));
    let op = "- {op: add_metadata, page: concepts/garden, key: tags, values: [plants]}\n";
    let many = batch(&op.repeat(51));
    assert!(errors(&editor.apply(&many).unwrap()).contains("at most 50"));
    assert_eq!(snapshot(&wiki.root()), before);
}

#[test]
fn the_readme_example_batch_is_valid() {
    let readme = include_str!("../../../README.md");
    let editor_section = &readme[readme
        .find("## Editor")
        .expect("README has an Editor section")..];
    let start = editor_section.find("```yaml\n").expect("a YAML example") + "```yaml\n".len();
    let example = &editor_section[start..start + editor_section[start..].find("```").unwrap()];
    let batch = Batch::parse(example).unwrap();
    let wiki = wiki();
    let mut editor = wiki.editor();
    let outcome = editor.apply(&batch).unwrap();
    assert!(outcome.applied, "{outcome:#?}");
    assert!(
        wiki.read("entities/bob-martin.md")
            .contains("roles: [maintainer]")
    );
    assert!(wiki.read("entities/bob-martin.md").ends_with(
        "Bob digs. [[raw/notes/a.md#^message-a]]\n\n\
         In October 2026, Alice said Bobby still maintains the Greenhouse setup. [[raw/notes/a.md#^message-a]]\n"
    ));
    let rewrites: Vec<_> = outcome.rewrites.iter().map(|r| r.to.as_str()).collect();
    assert!(rewrites.contains(&"colleague") && rewrites.contains(&"relationship.bob-martin"));
}
