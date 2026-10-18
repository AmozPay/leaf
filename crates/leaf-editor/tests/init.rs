//! A new wiki from `init_wiki`: conformant, readable by the viewer, and ready
//! for a first batch.

mod common;

use common::{EditorExt, TempWiki, batch, errors};
use leaf_editor::{InitOptions, init_wiki};
use leaf_viewer::{WikiConfig, WikiIndex};

fn options() -> InitOptions {
    InitOptions {
        domain: Some("Greenhouse notes".into()),
        tags: vec!["Gardening".into(), "soil".into(), "gardening".into()],
        today: Some("2026-10-17".parse().unwrap()),
    }
}

fn fresh() -> TempWiki {
    let wiki = TempWiki::new();
    init_wiki(&wiki.root(), &options()).unwrap();
    wiki
}

#[test]
fn a_new_wiki_has_the_baseline_layout() {
    let wiki = fresh();
    for folder in [
        "entities",
        "concepts",
        "comparisons",
        "queries",
        "raw/articles",
        "raw/papers",
        "raw/transcripts",
        "raw/assets",
        "_meta",
    ] {
        assert!(wiki.root().join(folder).is_dir(), "{folder}");
    }
    let index = wiki.read("index.md");
    assert!(index.contains("> Last updated: 2026-10-17 | Total pages: 0\n"));
    let sections: Vec<&str> = index.lines().filter(|l| l.starts_with("## ")).collect();
    assert_eq!(
        sections,
        ["## Entities", "## Concepts", "## Comparisons", "## Queries"]
    );
    assert_eq!(
        wiki.read("log.md"),
        "# Wiki Log\n\n## [2026-10-17] create | Wiki initialized\n- Domain: Greenhouse notes\n"
    );
    let schema = wiki.read("SCHEMA.md");
    assert!(schema.contains("Greenhouse notes"));
    assert!(schema.contains("`title`, `type`, `created`, `updated`, `tags`, `sources`"));
    assert!(schema.contains("- `gardening`\n- `soil`\n"), "{schema}");
    assert!(wiki.read("_meta/vocabulary.yaml").contains("gardening"));
}

#[test]
fn a_new_wiki_conforms_and_the_viewer_reads_it() {
    let wiki = fresh();
    let editor = wiki.editor();
    let report = editor.conformance().unwrap();
    assert!(report.is_conformant(), "{}", report.to_text());
    let mut index = WikiIndex::open(WikiConfig::new(wiki.root(), wiki.path("viewer.db"))).unwrap();
    let sync = index.sync().unwrap();
    assert!(sync.is_complete() && sync.warnings.is_empty(), "{sync:?}");
    assert_eq!(index.stats().unwrap().pages, 0);
}

#[test]
fn init_refuses_to_touch_an_existing_wiki() {
    let wiki = fresh();
    let before = common::snapshot(&wiki.root());
    let err = init_wiki(&wiki.root(), &options()).unwrap_err();
    assert!(err.to_string().contains("not empty"), "{err}");
    assert_eq!(common::snapshot(&wiki.root()), before);
    let bad = InitOptions {
        tags: vec!["!!!".into()],
        ..options()
    };
    assert!(init_wiki(&TempWiki::new().root(), &bad).is_err());
}

const SOURCE: &str = "- op: add_raw\n  path: raw/articles/greenhouse.md\n  \
    source_url: https://example.org/greenhouse\n  text: |\n    # Greenhouse\n\n    Plants like light.\n\n    ^light\n";

fn page(slug: &str, links: &str) -> String {
    format!(
        "- op: create_page\n  type: concept\n  slug: {slug}\n  title: {slug}\n  summary: A page\n  \
         metadata: {{tags: [gardening], confidence: medium}}\n  \
         intro: [{{text: '{links}'}}]\n  \
         sections:\n    - heading: Evidence\n      passages:\n        \
         - {{text: Plants like light., citations: ['raw/articles/greenhouse.md#^light']}}\n"
    )
}

#[test]
fn the_first_pages_of_a_new_wiki_can_be_created() {
    let wiki = fresh();
    // The first page of an empty wiki has nothing to link to, so the rule of
    // two links to other pages rejects it on its own.
    let alone = wiki
        .editor()
        .check(&batch(&format!(
            "{SOURCE}{}",
            page("light", "Nothing to link yet.")
        )))
        .unwrap();
    assert!(!alone.is_acceptable());
    assert!(
        errors(&alone).contains("link(s) to other pages"),
        "{}",
        errors(&alone)
    );

    // Three interlinked pages in one batch are accepted.
    let ops = format!(
        "{SOURCE}{}{}{}",
        page("light", "See [[concepts/water]] and [[concepts/soil]]."),
        page("water", "See [[concepts/light]] and [[concepts/soil]]."),
        page("soil", "See [[concepts/light]] and [[concepts/water]]."),
    );
    let outcome = wiki.editor().apply(&batch(&ops)).unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert!(wiki.read("index.md").contains("Total pages: 3\n"));
    assert!(wiki.editor().conformance().unwrap().is_conformant());
}
