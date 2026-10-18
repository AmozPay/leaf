//! The conformance report: issues to fix and declarations to propose, for a
//! wiki checked against the baseline vocabulary plus its own extension.

mod common;

use common::{TempWiki, snapshot};
use leaf_editor::vocabulary::ValueKind;
use leaf_editor::{ConformanceReport, Editor, EditorConfig, Vocabulary};

const CITE: &str = "[[raw/notes/a.md#^message-a]]";
const DATES: &str = "created: 2026-10-01\nupdated: 2026-10-02\n";

fn report(wiki: &TempWiki) -> ConformanceReport {
    let editor = Editor::open(EditorConfig::new(wiki.root(), wiki.db())).unwrap();
    editor.conformance().unwrap()
}

fn sources(wiki: &TempWiki) {
    wiki.write(
        "raw/notes/a.md",
        "---\nsource: direct statement\ningested: 2026-10-01\nsha256: ab\n---\nIt is so.\n\n^message-a\n",
    );
    wiki.write("raw/exports/x.md", "Exported.\n");
    wiki.write(
        "_meta/manifest.json",
        "{\"files\": [{\"path\": \"raw/notes/a.md\", \"bytes\": 1, \"sha256\": \"ab\"}]}",
    );
}

/// A wiki with one problem of each kind.
fn uneven() -> TempWiki {
    let wiki = TempWiki::new();
    sources(&wiki);
    wiki.write(
        "index.md",
        "[[concepts/garden]] [[concepts/misfiled]] [[entities/alice]] [[entities/bob-martin]] [[entities/acme]]\n",
    );
    wiki.write(
        "concepts/garden.md",
        &format!(
            "---\ntitle: Garden\ntype: concept\n{DATES}tags: [Gardening]\nsources: [raw/notes/a.md]\n\
             confidence: medium\nrelated-places: [paris]\n---\n# Garden\n\nAn introduction.\n\n\
             ## Beds\n\nBeds are raised. {CITE}\n"
        ),
    );
    wiki.write(
        "concepts/misfiled.md",
        &format!(
            "---\ntitle: Misfiled\ntype: entity\n{DATES}tags: [gardening]\nconfidence: strong\n\
             related-people: [nobody]\n---\n# Misfiled\n\n## Details\n\nNo source here.\n\n\
             Cited claim. {CITE}\n"
        ),
    );
    wiki.write(
        "entities/alice.md",
        &format!(
            "---\ntitle: Alice\ntype: entity\n{DATES}tags: [person]\nsources: [raw/notes/a.md]\n\
             related-persons: [bob-martin]\nrelationship: {{bob-martin: [friend]}}\n---\n\
             # Alice\n\n## Work\n\nAlice gardens. {CITE}\n"
        ),
    );
    wiki.write(
        "entities/bob-martin.md",
        &format!(
            "---\ntitle: Bob Martin\ntype: entity\n{DATES}tags: [person]\nsources: [raw/notes/a.md]\n\
             confidence: high\naliases: [Bobby]\nassociated-organizations: [acme]\n\
             relationship: {{alice: [colleague]}}\n---\n# Bob Martin\n\n## Work\n\nBob digs. {CITE}\n"
        ),
    );
    wiki.write(
        "entities/acme.md",
        &format!(
            "---\ntitle: Acme\ntype: entity\n{DATES}tags: [organization]\nsources: [raw/notes/a.md]\n\
             confidence: high\n---\nAcme sells seeds. {CITE}\n"
        ),
    );
    wiki
}

fn problems(report: &ConformanceReport, path: &str) -> Vec<String> {
    report
        .issues
        .iter()
        .filter(|i| i.path == path)
        .map(|i| i.problem.clone())
        .collect()
}

fn has(report: &ConformanceReport, path: &str, needle: &str) -> bool {
    problems(report, path).iter().any(|p| p.contains(needle))
}

#[test]
fn issues_name_what_to_fix() {
    let wiki = uneven();
    let report = report(&wiki);
    assert_eq!(report.pages, 5);
    let misfiled = "concepts/misfiled.md";
    assert!(
        has(&report, misfiled, "missing required `sources`"),
        "{report:#?}"
    );
    assert!(has(
        &report,
        misfiled,
        "`type: entity` belongs in entities/"
    ));
    assert!(has(&report, misfiled, "write `high` instead of `strong`"));
    assert!(has(&report, misfiled, "`nobody` refers to no page"));
    assert!(has(
        &report,
        misfiled,
        "uncited passage(s) under \"Details\""
    ));
    // Synonym keys and one-sided symmetric relationships.
    assert!(has(
        &report,
        "entities/alice.md",
        "write `related-people` instead of `related-persons`"
    ));
    assert!(has(
        &report,
        "entities/bob-martin.md",
        "`friend` with alice"
    ));
    assert!(has(
        &report,
        "entities/alice.md",
        "`colleague` with bob-martin"
    ));
    // Introductions are allowed uncited; conformant pages raise nothing.
    assert!(
        problems(&report, "concepts/garden.md").is_empty(),
        "{report:#?}"
    );
    assert!(
        problems(&report, "entities/acme.md").is_empty(),
        "{report:#?}"
    );
    assert!(
        report.notes.iter().any(|n| n.starts_with("1 pages lack")),
        "{:?}",
        report.notes
    );
}

#[test]
fn declarations_are_proposed_not_imposed() {
    let wiki = uneven();
    let before = snapshot(&wiki.root());
    let report = report(&wiki);
    assert_eq!(
        snapshot(&wiki.root()),
        before,
        "the report must not write to the wiki"
    );
    let extension = &report.extension;
    // The wiki's own taxonomy and places.
    let tags: Vec<_> = extension.labels["tags"].keys().cloned().collect();
    assert_eq!(tags, ["gardening", "organization", "person"]);
    assert!(extension.places.contains_key("paris"));
    // A new key, typed from the dictionary's word for its values.
    assert_eq!(
        extension.keys["associated-organizations"].values,
        ValueKind::Organization
    );
    // Undeclared raw collections and the checksum manifest.
    assert!(extension.raw_collections.contains_key("exports"));
    assert!(extension.raw_collections.contains_key("notes"));
    assert_eq!(
        extension.raw_manifest.as_deref(),
        Some("_meta/manifest.json")
    );
    // Nothing from the baseline is repeated.
    assert!(!extension.keys.contains_key("tags"));
    assert!(extension.page_types.is_empty() && extension.required_keys.is_empty());
    assert!(
        report
            .to_text()
            .contains("Proposed additions to _meta/vocabulary.yaml")
    );
}

#[test]
fn saving_the_proposal_and_fixing_pages_makes_the_wiki_conformant() {
    let wiki = TempWiki::new();
    sources(&wiki);
    wiki.write("index.md", "[[concepts/garden]]\n");
    wiki.write(
        "concepts/garden.md",
        &format!(
            "---\ntitle: Garden\ntype: concept\n{DATES}tags: [gardening]\nsources: [raw/notes/a.md]\n\
             confidence: medium\nrelated-places: [paris]\n---\n# Garden\n\nAn introduction.\n\n\
             ## Beds\n\nBeds are raised. {CITE}\n"
        ),
    );
    let first = report(&wiki);
    assert!(first.issues.is_empty(), "{first:#?}");
    assert!(!first.is_conformant(), "declarations are still needed");
    wiki.write("_meta/vocabulary.yaml", &first.extension.to_yaml());
    let second = report(&wiki);
    assert!(second.is_conformant(), "{second:#?}");
    assert!(second.to_text().contains("No issues."));
    // The saved extension is a valid extension of the baseline.
    let vocabulary = Vocabulary::load(&wiki.root()).unwrap();
    assert!(vocabulary.labels["tags"].contains_key("gardening"));
}
