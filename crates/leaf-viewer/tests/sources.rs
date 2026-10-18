//! Citations of any raw file type, whole or by message, block or heading.

mod common;

use common::{Fixture, snapshot};
use leaf_viewer::{Anchor, Citation, Error, SearchRequest, WikiIndex};

const ARTICLE: &str = "---\nsource_url: https://example.org/\nsha256: abc\n---\n# Example site\n\nWe build orchards.\n";
const NOTES: &str = "# Notes\n\nIntro.\n\n## Setup\n\nInstall the pump. ^pump\n\n### Detail\n\nUse a filter.\n\n## Usage\n\nWater daily.\n";
const PDF: &[u8] = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\nbinary orchard";

fn fixture() -> (Fixture, WikiIndex) {
    let wiki = Fixture::new();
    wiki.write("raw/articles/site.md", ARTICLE);
    wiki.write("raw/notes/n.md", NOTES);
    wiki.write("raw/data/d.json", "{\"orchards\": 3}\n");
    std::fs::create_dir_all(wiki.root.join("raw/papers")).unwrap();
    std::fs::write(wiki.root.join("raw/papers/p.pdf"), PDF).unwrap();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write(
        "concepts/topic.md",
        "---\ntitle: Topic\n---\n\
         Site claim. [[raw/articles/site]]\n\n\
         Paper claim. [[raw/papers/p.pdf|Paper]] [[raw/papers/p.pdf#Intro]]\n\n\
         Data claim. [[raw/data/d.json]]\n\n\
         Pump claim. [[raw/notes/n.md#^pump]] [[raw/notes/n.md#Notes#Setup|Setup]]\n",
    );
    let mut index = wiki.open();
    let report = index.sync().unwrap();
    assert!(
        report.is_complete() && report.warnings.is_empty(),
        "{report:?}"
    );
    (wiki, index)
}

fn read(index: &WikiIndex, reference: &str) -> leaf_viewer::Result<leaf_viewer::CitationExcerpt> {
    index.read_citation(&reference.parse::<Citation>().unwrap(), 1_000)
}

#[test]
fn search_hits_carry_document_and_anchor_citations() {
    let (_wiki, index) = fixture();
    let response = index
        .search(&SearchRequest::new("paper pump", 5, 10_000))
        .unwrap();
    let mut cited: Vec<String> = response
        .hits
        .iter()
        .flat_map(|hit| hit.citations.iter().map(|c| c.to_string()))
        .collect();
    cited.sort();
    assert_eq!(
        cited,
        [
            "raw/notes/n.md#Notes#Setup",
            "raw/notes/n.md#^pump",
            "raw/papers/p.pdf",
            "raw/papers/p.pdf#Intro"
        ]
    );
}

#[test]
fn whole_markdown_documents_leave_out_their_frontmatter() {
    let (_wiki, index) = fixture();
    let excerpt = read(&index, "raw/articles/site.md").unwrap();
    assert_eq!(excerpt.media_type, "text/markdown");
    assert_eq!(excerpt.size_bytes, ARTICLE.len() as u64);
    assert_eq!(
        excerpt.text.as_deref(),
        Some("# Example site\n\nWe build orchards.")
    );
    let data = read(&index, "raw/data/d.json").unwrap();
    assert_eq!(data.media_type, "application/json");
    assert_eq!(data.text.as_deref(), Some("{\"orchards\": 3}"));
}

#[test]
fn binary_sources_are_described_not_read() {
    let (_wiki, index) = fixture();
    let excerpt = read(&index, "raw/papers/p.pdf").unwrap();
    assert_eq!(excerpt.media_type, "application/pdf");
    assert_eq!(excerpt.size_bytes, PDF.len() as u64);
    assert_eq!(excerpt.text, None);
    assert!(!excerpt.truncated);
    assert_eq!(excerpt.citation.label, "");

    assert!(matches!(
        read(&index, "raw/papers/p.pdf#Intro"),
        Err(Error::UnsupportedSource(_))
    ));
}

#[test]
fn blocks_and_heading_sections_are_cut_out() {
    let (_wiki, index) = fixture();
    assert_eq!(
        read(&index, "raw/notes/n.md#^pump")
            .unwrap()
            .text
            .as_deref(),
        Some("Install the pump.")
    );
    let setup = read(&index, "raw/notes/n.md#Notes#Setup").unwrap();
    assert_eq!(
        setup.text.as_deref(),
        Some("## Setup\n\nInstall the pump. ^pump\n\n### Detail\n\nUse a filter.")
    );
    assert_eq!(
        setup.citation.anchor,
        Some(Anchor::Heading("Notes#Setup".into()))
    );
}

#[test]
fn only_the_exact_cited_part_is_served() {
    let (_wiki, index) = fixture();
    for uncited in [
        "raw/notes/n.md",
        "raw/notes/n.md#Usage",
        "raw/notes/n.md#^other",
        "raw/articles/site.md#Example site",
        "raw/papers/other.pdf",
    ] {
        assert!(
            matches!(read(&index, uncited), Err(Error::CitationNotIndexed)),
            "{uncited}"
        );
    }
}

#[test]
fn reading_sources_never_writes_to_raw() {
    let (wiki, index) = fixture();
    let before = snapshot(&wiki.root.join("raw"));
    for reference in [
        "raw/articles/site.md",
        "raw/papers/p.pdf",
        "raw/data/d.json",
        "raw/notes/n.md#^pump",
        "raw/notes/n.md#Notes#Setup",
    ] {
        read(&index, reference).unwrap();
    }
    assert_eq!(snapshot(&wiki.root.join("raw")), before);
}
