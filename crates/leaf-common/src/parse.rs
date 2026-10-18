//! Read-only access to the page parser, so other tools (such as
//! `leaf-editor`) read pages exactly as the index does.
//!
//! [`page`] and [`published_pages`] are pure functions over text;
//! [`published_file`] and [`check_citation`] only read the wiki.

use crate::citation;
use crate::markdown::{self, ParsedPage};
use crate::model::{Citation, SyncIssue};
use crate::publication;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Directories whose pages may be published through `index.md`.
pub const PUBLISHED_DIRS: [&str; 4] = publication::PUBLISHED_DIRS;

/// A parsed page, as the index sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    /// Every frontmatter key but `title`, cleaned as the index stores it
    /// (one-level maps also appear as `key.<entry>`).
    pub facets: BTreeMap<String, Vec<String>>,
    /// Indexed passages in document order.
    pub passages: Vec<Passage>,
    /// Headings in document order.
    pub headings: Vec<Heading>,
    /// Content that was not indexed (unsupported blocks, ignored links, …).
    pub warnings: Vec<String>,
}

/// One indexed paragraph or list item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// Locator unique within the page, `p-<hash>[-n]`; the index's passage
    /// ID is `<page path>#<locator>`.
    pub locator: String,
    pub heading: String,
    /// Claim text with citation links removed.
    pub text: String,
    pub citations: Vec<Citation>,
    /// Byte range of the passage's content in the page: its text, inline
    /// markup and citations, without list markers or nested items.
    pub span: Range<usize>,
    /// Byte range of the block holding it: its list item if it is in one,
    /// else its paragraph.
    pub block: Range<usize>,
    /// Whether the passage is a list item.
    pub in_item: bool,
}

/// A heading of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub text: String,
    /// 1 (`#`) to 6 (`######`).
    pub level: u8,
    /// Byte range of the heading in the page.
    pub range: Range<usize>,
}

/// Parse a page's content. `Err` means the index would refuse the page.
pub fn page(content: &str) -> Result<Page, String> {
    let parsed: ParsedPage = markdown::parse_page(content)?;
    Ok(Page {
        title: parsed.title,
        facets: parsed.facets,
        passages: parsed
            .passages
            .into_iter()
            .map(|p| Passage {
                locator: p.locator,
                heading: p.heading,
                text: p.text,
                citations: p.citations,
                span: p.span,
                block: p.block,
                in_item: p.in_item,
            })
            .collect(),
        headings: parsed
            .headings
            .into_iter()
            .map(|h| Heading {
                text: h.text,
                level: h.level,
                range: h.range,
            })
            .collect(),
        warnings: parsed.warnings,
    })
}

/// The frontmatter of any Markdown file (a page or a raw source) as facets,
/// `title` included; `Ok(None)` when the file has none.
pub fn frontmatter(content: &str) -> Result<Option<BTreeMap<String, Vec<String>>>, String> {
    markdown::frontmatter(content)
}

/// The byte range of a file's frontmatter block (opening `---` through the
/// closing line) and its YAML text, so a writer can replace exactly that block.
pub fn frontmatter_block(content: &str) -> Option<(std::ops::Range<usize>, String)> {
    markdown::frontmatter_block(content)
}

/// Pages published by an `index.md`, and links in it that are unusable.
pub fn published_pages(index: &str) -> (BTreeSet<String>, Vec<SyncIssue>) {
    let publication = publication::published_pages(index);
    (publication.pages, publication.errors)
}

/// The file of a published page, refusing paths that leave its published
/// directory (including through symlinks) or land in `raw/`.
pub fn published_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    publication::resolve_page(root, relative)
}

/// Whether a citation's target exists: a file inside `raw/` (no traversal
/// or symlink escape) and, if anchored, exactly one such message, block or
/// heading in it. Reads the source; writes nothing.
pub fn check_citation(root: &Path, citation: &Citation) -> Result<(), String> {
    citation::check_path(&citation.raw_path).map_err(|err| err.to_string())?;
    let source = citation::resolve(root, &citation.raw_path).map_err(|err| err.to_string())?;
    citation::read(&source, citation, 1)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// [`check_citation`] for a source that is not on disk yet: `content` is
/// its text (`markdown` says whether it is a Markdown file).
pub fn check_citation_text(
    citation: &Citation,
    markdown: bool,
    content: &str,
) -> Result<(), String> {
    citation::check_path(&citation.raw_path).map_err(|err| err.to_string())?;
    citation::locate(content, markdown, citation.anchor.as_ref())
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// Every message and block anchor a text source declares, once per
/// occurrence (so duplicates show).
pub fn anchors(content: &str) -> Vec<crate::Anchor> {
    citation::anchor_ids(content)
}

/// The media type of a file and whether it is text, from its first bytes and
/// else its name, as the viewer decides it when reading a citation.
pub fn media_type(path: &str, head: &[u8]) -> (String, bool) {
    citation::sniff(Path::new(path), head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passages_and_headings_have_ranges() {
        let text = "---\ntitle: T\n---\n# T\n\nIntro **bold** text. [[raw/a.md]]\n\n## Items\n\n\
                    - First item\n  - Nested item\n- Second\n";
        let page = page(text).unwrap();
        let span = |n: usize| &text[page.passages[n].span.clone()];
        let block = |n: usize| &text[page.passages[n].block.clone()];
        assert_eq!(span(0), "Intro **bold** text. [[raw/a.md]]");
        assert_eq!(block(0), "Intro **bold** text. [[raw/a.md]]\n");
        // A list item's span leaves out its marker and nested items.
        assert_eq!(span(1), "First item");
        assert!(block(1).starts_with("- First item\n  - Nested item"));
        assert_eq!(span(2), "Nested item");
        assert_eq!(span(3), "Second");
        assert!(!page.passages[0].in_item && page.passages[1].in_item);
        let headings: Vec<_> = page
            .headings
            .iter()
            .map(|h| (h.text.as_str(), h.level))
            .collect();
        assert_eq!(headings, [("T", 1), ("Items", 2)]);
        assert_eq!(&text[page.headings[1].range.clone()], "## Items\n");
    }

    #[test]
    fn citation_targets_are_checked_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("raw/notes")).unwrap();
        std::fs::write(
            root.join("raw/notes/a.md"),
            "### user\n\nHello.\n\n^message-m1\n",
        )
        .unwrap();
        let check = |reference: &str| check_citation(root, &reference.parse().unwrap());
        assert!(check("raw/notes/a.md").is_ok());
        assert!(check("raw/notes/a.md#^message-m1").is_ok());
        assert!(check("raw/notes/a.md#^message-m2").is_err());
        assert!(check("raw/notes/missing.md").is_err());
    }

    #[test]
    fn page_exposes_what_the_index_reads() {
        let parsed = page(
            "---\ntitle: T\ntags: [A]\n---\n# H\n\nClaim. [[raw/a.md#^message-1]]\n\n```\nx\n```\n",
        )
        .unwrap();
        assert_eq!(parsed.title, "T");
        assert_eq!(parsed.facets["tags"], ["a"]);
        assert_eq!(parsed.passages[0].heading, "H");
        assert_eq!(parsed.passages[0].text, "Claim.");
        assert_eq!(
            parsed.passages[0].citations[0].to_string(),
            "raw/a.md#^message-1"
        );
        assert_eq!(parsed.warnings.len(), 1);
        assert!(page("no frontmatter").is_err());
        let raw = frontmatter("---\nsource: x\nsha256: ab\n---\nBody\n^message-1\n")
            .unwrap()
            .unwrap();
        assert_eq!(raw["source"], ["x"]);
        assert!(!raw.contains_key("title"));
        assert_eq!(frontmatter("Body").unwrap(), None);
        assert!(frontmatter("---\na: [\n---\n").is_err());
        let text = "---\ntitle: T\n---\n# Body\n";
        let (range, yaml) = frontmatter_block(text).unwrap();
        assert_eq!(&text[range.end..], "# Body\n");
        assert_eq!(yaml, "title: T\n");
        assert_eq!(frontmatter_block("# No frontmatter\n"), None);
        let (pages, errors) = published_pages("[[concepts/a]] [[raw/x]] [[concepts/../b]]");
        assert_eq!(pages.into_iter().collect::<Vec<_>>(), ["concepts/a.md"]);
        assert_eq!(errors.len(), 1);
    }
}
