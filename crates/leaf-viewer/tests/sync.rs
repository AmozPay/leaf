mod common;

use common::{Fixture, page};
use leaf_viewer::{SearchRequest, WikiIndex};
use std::fs;

fn hits(index: &WikiIndex, text: &str) -> usize {
    index
        .search(&SearchRequest::new(text, 10, 10_000))
        .unwrap()
        .hits
        .len()
}

fn assert_fts_matches_passages(wiki: &Fixture) {
    assert_eq!(wiki.count("passage_fts"), wiki.count("passage"));
}

#[test]
fn incremental_add_skip_update_and_unpublish() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic|Topic]]");
    let topic = wiki.write(
        "concepts/topic.md",
        &page("Topic", "Old orchard.\n\nStable river."),
    );
    let mut index = wiki.open();

    let first = index.sync().unwrap();
    assert_eq!(
        (first.added, first.updated, first.removed, first.skipped),
        (1, 0, 0, 0)
    );
    let second = index.sync().unwrap();
    assert_eq!(
        (second.added, second.updated, second.removed, second.skipped),
        (0, 0, 0, 1)
    );

    fs::write(&topic, page("Topic", "New meadow.\n\nStable river.")).unwrap();
    let third = index.sync().unwrap();
    assert_eq!(
        (third.added, third.updated, third.removed, third.skipped),
        (0, 1, 0, 0)
    );
    assert_eq!(hits(&index, "orchard"), 0);
    assert_eq!(hits(&index, "meadow"), 1);
    assert_eq!(hits(&index, "river"), 1);
    assert_fts_matches_passages(&wiki);

    wiki.write("index.md", "[[concepts/topic]] [[concepts/second]]");
    wiki.write("concepts/second.md", &page("Second", "Second harbour."));
    let fourth = index.sync().unwrap();
    assert_eq!((fourth.added, fourth.skipped), (1, 1));

    wiki.write("index.md", "[[concepts/second]]");
    let fifth = index.sync().unwrap();
    assert_eq!((fifth.removed, fifth.skipped), (1, 1));
    assert_eq!(hits(&index, "meadow"), 0);
    assert_eq!(hits(&index, "river"), 0);
    assert_eq!(index.stats().unwrap().pages, 1);
    assert_fts_matches_passages(&wiki);
}

#[test]
fn malformed_page_is_reported_and_not_served_from_stale_rows() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]] [[concepts/other]]");
    let topic = wiki.write("concepts/topic.md", &page("Topic", "Old orchard."));
    wiki.write("concepts/other.md", &page("Other", "Other harbour."));
    let mut index = wiki.synced();
    assert_eq!(hits(&index, "orchard"), 1);

    fs::write(&topic, "No frontmatter here.\n\nOld orchard.\n").unwrap();
    let report = index.sync().unwrap();
    assert!(!report.is_complete());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].path, "concepts/topic.md");
    assert_eq!(report.skipped, 1);
    assert_eq!(hits(&index, "orchard"), 0);
    assert_eq!(hits(&index, "harbour"), 1);
    assert_fts_matches_passages(&wiki);

    // A repeated sync keeps reporting the broken page rather than hiding it.
    let again = index.sync().unwrap();
    assert_eq!(again.errors.len(), 1);

    fs::write(&topic, page("Topic", "Fixed orchard.")).unwrap();
    let fixed = index.sync().unwrap();
    assert!(fixed.is_complete());
    assert_eq!(fixed.added, 1);
    assert_eq!(hits(&index, "orchard"), 1);
}

#[test]
fn missing_published_page_is_reported_and_its_rows_removed() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    let topic = wiki.write("concepts/topic.md", &page("Topic", "Old orchard."));
    let mut index = wiki.synced();
    fs::remove_file(topic).unwrap();
    let report = index.sync().unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].path, "concepts/topic.md");
    assert_eq!(hits(&index, "orchard"), 0);
    assert_eq!(index.stats().unwrap().pages, 0);
    assert_fts_matches_passages(&wiki);
}

#[test]
fn missing_title_and_invalid_frontmatter_are_errors() {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[concepts/a]] [[concepts/b]] [[concepts/c]] [[concepts/d]]",
    );
    wiki.write("concepts/a.md", "---\nconfidence: low\n---\nNo title.\n");
    wiki.write("concepts/b.md", "---\ntitle: Unclosed\n\nBody.\n");
    wiki.write(
        "concepts/c.md",
        "---\ntitle: [Not, a, scalar]\n---\nBody.\n",
    );
    wiki.write(
        "concepts/d.md",
        "---\ntitle: D\ncontested: maybe\n---\nBody.\n",
    );
    let mut index = wiki.open();
    let report = index.sync().unwrap();
    assert_eq!(report.added, 0);
    let mut failed: Vec<_> = report.errors.iter().map(|e| e.path.as_str()).collect();
    failed.sort();
    assert_eq!(
        failed,
        [
            "concepts/a.md",
            "concepts/b.md",
            "concepts/c.md",
            "concepts/d.md"
        ]
    );
}

#[test]
fn unsupported_markdown_blocks_are_skipped_with_warnings() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write(
        "concepts/topic.md",
        &page(
            "Topic",
            "```\n# not a heading\nfenced zebra\n```\n\n\
             | a | b |\n|---|---|\n| table | zebra |\n\n> quoted zebra\n\n<div>html zebra</div>\n\n\
             Plain claim about orchards.",
        ),
    );
    let mut index = wiki.open();
    let report = index.sync().unwrap();
    assert_eq!(report.added, 1);
    assert!(report.is_complete());
    assert_eq!(report.warnings.len(), 4, "{report:?}");
    assert!(
        report
            .warnings
            .iter()
            .all(|w| w.path == "concepts/topic.md")
    );
    assert_eq!(hits(&index, "zebra"), 0);
    let orchard = index
        .search(&SearchRequest::new("orchards", 5, 10_000))
        .unwrap();
    assert_eq!(orchard.hits.len(), 1);
    assert_eq!(orchard.hits[0].heading, "Topic");
}

#[test]
fn frontmatter_is_not_searchable() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write(
        "concepts/topic.md",
        "---\ntitle: Topic\ntags: [zebra]\nsources:\n  - raw/chatgpt/zebra.md\n---\nPlain orchard.\n",
    );
    let index = wiki.synced();
    assert_eq!(hits(&index, "zebra"), 0);
    assert_eq!(hits(&index, "orchard"), 1);
}
