mod common;

use common::{Fixture, cite, page};
use leaf_viewer::{Error, SearchRequest};
use std::fs;

fn query(index: &leaf_viewer::WikiIndex, text: &str) -> Vec<String> {
    index
        .search(&SearchRequest::new(text, 10, 10_000))
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.page_path)
        .collect()
}

#[test]
fn indexes_only_published_pages_with_headings_and_citations() {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "- [[concepts/greenhouse-cluster-design|Greenhouse cluster]] — routing\n\
         - [[_meta/notes]] and [[raw/chatgpt/a]] are never published\n\
         - [[SCHEMA]]\n",
    );
    wiki.write("concepts/draft.md", &page("Draft", "Secret zebra phrase."));
    wiki.write("_meta/notes.md", &page("Notes", "Secret zebra phrase."));
    wiki.write(
        "raw/chatgpt/a.md",
        "### user · 2025\n\nSecret zebra phrase.\n\n^message-abc\n",
    );
    wiki.write(
        "concepts/greenhouse-cluster-design.md",
        &page(
            "Greenhouse cluster design",
            &format!(
                "## Networking\n\nNetwork naming worked locally. {}\n\nThe cluster remained a plan.",
                cite("a.md", "abc", "Report")
            ),
        ),
    );

    let index = wiki.synced();
    assert_eq!(index.stats().unwrap().pages, 1);
    let response = index
        .search(&SearchRequest::new("network naming", 3, 1000))
        .unwrap();
    // The sibling paragraph also matches through its "Networking" heading,
    // but the body match ranks first.
    assert!(
        response
            .hits
            .iter()
            .all(|h| h.page_path == "concepts/greenhouse-cluster-design.md")
    );
    let hit = &response.hits[0];
    assert_eq!(hit.title, "Greenhouse cluster design");
    assert_eq!(hit.heading, "Networking");
    assert_eq!(hit.text, "Network naming worked locally.");
    assert_eq!(hit.confidence.as_deref(), Some("medium"));
    assert_eq!(hit.citations.len(), 1);
    assert_eq!(hit.citations[0].raw_path, "raw/chatgpt/a.md");
    assert_eq!(
        hit.citations[0].to_string(),
        "raw/chatgpt/a.md#^message-abc"
    );
    assert_eq!(hit.citations[0].label, "Report");
    assert!(query(&index, "zebra").is_empty());
}

#[test]
fn accepts_wikilink_and_markdown_link_forms_and_deduplicates() {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[concepts/alpha]] [[concepts/alpha.md|Alpha]] [[concepts/alpha#Section]]\n\
         [Beta](concepts/beta.md) [[entities/Gamma Person|Gamma]]\n",
    );
    wiki.write("concepts/alpha.md", &page("Alpha", "Alpha orchard."));
    wiki.write("concepts/beta.md", &page("Beta", "Beta orchard."));
    wiki.write("entities/Gamma Person.md", &page("Gamma", "Gamma orchard."));
    let mut index = wiki.open();
    let report = index.sync().unwrap();
    assert_eq!(report.added, 3, "{report:?}");
    assert!(report.is_complete());
    assert_eq!(query(&index, "orchard").len(), 3);
}

#[test]
fn links_in_index_code_fences_are_not_published() {
    let wiki = Fixture::new();
    wiki.write("index.md", "```\n[[concepts/alpha]]\n```\n");
    wiki.write("concepts/alpha.md", &page("Alpha", "Alpha orchard."));
    let mut index = wiki.open();
    assert_eq!(index.sync().unwrap().added, 0);
}

#[test]
fn unsafe_index_links_are_reported_and_not_indexed() {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[concepts/../_meta/notes]] [[concepts/./alpha]] [[concepts//alpha]] [[concepts/a\\b]]\n",
    );
    wiki.write("_meta/notes.md", &page("Notes", "Secret zebra phrase."));
    wiki.write("concepts/alpha.md", &page("Alpha", "Secret zebra phrase."));
    let mut index = wiki.open();
    let report = index.sync().unwrap();
    assert_eq!(report.added, 0);
    assert_eq!(report.errors.len(), 4, "{report:?}");
    assert!(!report.is_complete());
    assert!(query(&index, "zebra").is_empty());
}

#[cfg(unix)]
#[test]
fn symlinked_pages_escaping_published_directories_are_rejected() {
    use std::os::unix::fs::symlink;
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[concepts/outside]] [[concepts/to-raw]] [[concepts/to-meta]]",
    );
    fs::write(
        wiki.base.join("outside.md"),
        page("Outside", "Secret zebra phrase."),
    )
    .unwrap();
    wiki.write("raw/chatgpt/r.md", &page("Raw", "Secret zebra phrase."));
    wiki.write("_meta/m.md", &page("Meta", "Secret zebra phrase."));
    symlink(
        wiki.base.join("outside.md"),
        wiki.root.join("concepts/outside.md"),
    )
    .unwrap();
    symlink(
        wiki.root.join("raw/chatgpt/r.md"),
        wiki.root.join("concepts/to-raw.md"),
    )
    .unwrap();
    symlink(
        wiki.root.join("_meta/m.md"),
        wiki.root.join("concepts/to-meta.md"),
    )
    .unwrap();
    let mut index = wiki.open();
    let report = index.sync().unwrap();
    assert_eq!(report.added, 0);
    assert_eq!(report.errors.len(), 3, "{report:?}");
    assert!(query(&index, "zebra").is_empty());
}

#[test]
fn missing_index_fails_without_unpublishing_everything() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/alpha]]");
    wiki.write("concepts/alpha.md", &page("Alpha", "Alpha orchard."));
    let mut index = wiki.synced();
    fs::remove_file(wiki.root.join("index.md")).unwrap();
    assert!(matches!(index.sync(), Err(Error::Io { .. })));
    assert_eq!(query(&index, "orchard").len(), 1);

    fs::write(wiki.root.join("index.md"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(index.sync().is_err());
    assert_eq!(query(&index, "orchard").len(), 1);
}

#[test]
fn readable_index_without_links_unpublishes_all_pages() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/alpha]]");
    wiki.write("concepts/alpha.md", &page("Alpha", "Alpha orchard."));
    let mut index = wiki.synced();
    wiki.write("index.md", "# Empty\n");
    assert_eq!(index.sync().unwrap().removed, 1);
    assert!(query(&index, "orchard").is_empty());
}
