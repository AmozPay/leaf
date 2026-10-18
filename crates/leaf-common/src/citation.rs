//! Raw citation authorization and bounded extraction of the cited part.
//!
//! A cited file may be of any type. Its media type is sniffed from the
//! content (`infer`), then the extension (`mime_guess`). Text is served as:
//!
//! * **Whole document** (no anchor): the file, without Markdown frontmatter.
//! * **Message** (`#^message-<id>`): an anchor line `^message-<id>` (outside
//!   code blocks) in one of two layouts. *Trailing anchor* (ChatGPT
//!   transcripts): the message starts at the last `### ` heading after the
//!   previous anchor and ends before its own anchor. *Leading anchor* (direct
//!   confirmations): no such heading exists and the anchor is immediately
//!   followed by text; the message runs until the next anchor or heading.
//! * **Block** (`#^<id>`): the Obsidian block marked `^<id>`, either at the end
//!   of a paragraph or list item, or alone after a block (table, list, …).
//! * **Heading** (`#Heading#Subheading`): that heading's section, up to the
//!   next heading of the same or a higher level.
//!
//! Code blocks, headings and blocks are recognized by the shared Markdown
//! parser. A file that is not text is described by its metadata only.
//! Anything else is rejected rather than guessed.

use crate::markdown;
use crate::model::{Anchor, Citation, CitationExcerpt, Error, Result};
use crate::publication::is_plain_relative;
use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Bytes read to decide whether a source is text and which type it is.
const SNIFF_BYTES: u64 = 8 * 1024;

/// Check the shape of a citation path before any lookup.
pub fn check_path(raw_path: &str) -> Result<()> {
    let unsafe_path = |why: &str| Err(Error::UnsafePath(format!("{raw_path:?}: {why}")));
    if !raw_path.starts_with("raw/") {
        return unsafe_path("must start with raw/");
    }
    if !is_plain_relative(raw_path) {
        return unsafe_path("must be a plain relative path");
    }
    Ok(())
}

/// Canonicalize a checked path and confine it to `root/raw/`.
pub fn resolve(root: &Path, raw_path: &str) -> Result<PathBuf> {
    let raw_dir = root.join("raw");
    let raw_dir = fs::canonicalize(&raw_dir).map_err(|err| Error::io(raw_dir, err))?;
    let source = root.join(raw_path);
    let canonical = fs::canonicalize(&source).map_err(|err| Error::io(&source, err))?;
    if !canonical.starts_with(&raw_dir) {
        return Err(Error::UnsafePath(format!(
            "{raw_path:?} resolves outside raw/"
        )));
    }
    if !canonical.is_file() {
        return Err(Error::UnsafePath(format!(
            "{raw_path:?} is not a regular file"
        )));
    }
    Ok(canonical)
}

pub fn read(source: &Path, citation: &Citation, max_chars: usize) -> Result<CitationExcerpt> {
    let io = |err| Error::io(source, err);
    let size_bytes = fs::metadata(source).map_err(io)?.len();
    let mut head = Vec::new();
    File::open(source)
        .map_err(io)?
        .take(SNIFF_BYTES)
        .read_to_end(&mut head)
        .map_err(io)?;
    let (mut media_type, is_text) = sniff(source, &head);
    let content = if is_text {
        match fs::read_to_string(source) {
            Ok(content) => Some(content),
            // Valid UTF-8 at the start but not throughout: not text.
            Err(err) if err.kind() == ErrorKind::InvalidData => {
                media_type = "application/octet-stream".into();
                None
            }
            Err(err) => return Err(io(err)),
        }
    } else {
        None
    };
    let Some(content) = content else {
        if let Some(anchor) = &citation.anchor {
            return Err(Error::UnsupportedSource(format!(
                "{} is {media_type}, not text, so `#{anchor}` cannot be located",
                citation.raw_path
            )));
        }
        return Ok(CitationExcerpt {
            citation: citation.clone(),
            media_type,
            size_bytes,
            text: None,
            truncated: false,
        });
    };
    let text = locate(
        &content,
        media_type == "text/markdown",
        citation.anchor.as_ref(),
    )?;
    let truncated = text.chars().count() > max_chars;
    Ok(CitationExcerpt {
        citation: citation.clone(),
        media_type,
        size_bytes,
        text: Some(text.chars().take(max_chars).collect()),
        truncated,
    })
}

/// The cited part of a text source: the whole document without its
/// frontmatter (`anchor` is `None`), or the message, block or section the
/// anchor names, which must exist exactly once. For writers checking content
/// that is not on disk yet.
pub fn locate(content: &str, markdown: bool, anchor: Option<&Anchor>) -> Result<String> {
    Ok(match anchor {
        None => document(content, markdown),
        Some(Anchor::Message(id)) => extract(content, id)?,
        Some(Anchor::Block(id)) => block(content, id)?,
        Some(Anchor::Heading(path)) => section(content, path)?,
    })
}

/// Every message and block anchor a text source declares (`^message-<id>`
/// lines, `^<id>` markers), once per occurrence, so duplicates show.
pub fn anchor_ids(content: &str) -> Vec<Anchor> {
    let mut anchors: Vec<Anchor> = classify(content)
        .into_iter()
        .filter_map(|(_, kind)| match kind {
            Kind::Anchor(id) => Some(Anchor::Message(id.to_string())),
            _ => None,
        })
        .collect();
    // A marker ends a paragraph, list item or heading, or is a paragraph of
    // its own; nested elements can end with the same marker.
    let mut blocks: std::collections::BTreeMap<usize, String> = std::collections::BTreeMap::new();
    for (event, range) in markdown::events(content) {
        if !matches!(
            event,
            Event::Start(Tag::Paragraph | Tag::Item | Tag::Heading { .. })
        ) {
            continue;
        }
        let text = content[range.clone()].trim_end();
        let token = text.rsplit(char::is_whitespace).next().unwrap_or(text);
        let at = text.len() - token.len();
        let Some(id) = token.strip_prefix('^') else {
            continue;
        };
        if id.is_empty() || id.starts_with("message-") {
            continue;
        }
        blocks.insert(range.start + at, id.to_string());
    }
    anchors.extend(blocks.into_values().map(Anchor::Block));
    anchors
}

/// Media type and whether the source is text, from its first bytes (magic
/// numbers, then UTF-8 without NUL bytes) and else its extension.
pub fn sniff(source: &Path, head: &[u8]) -> (String, bool) {
    let guessed = mime_guess::from_path(source).first_raw();
    if let Some(kind) = infer::get(head) {
        let is_text = kind.matcher_type() == infer::MatcherType::Text;
        return (kind.mime_type().to_string(), is_text);
    }
    // A multi-byte character may be cut at the end of the sniffed bytes.
    let utf8 = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(err) => err.error_len().is_none(),
    };
    if utf8 && !head.contains(&0) {
        (guessed.unwrap_or("text/plain").to_string(), true)
    } else {
        (
            guessed.unwrap_or("application/octet-stream").to_string(),
            false,
        )
    }
}

/// The whole text; Markdown frontmatter (source metadata) is left out.
fn document(content: &str, markdown: bool) -> String {
    let body = match markdown::events(content).next() {
        Some((Event::Start(Tag::MetadataBlock(_)), range)) if markdown && range.start == 0 => {
            &content[range.end..]
        }
        _ => content,
    };
    body.trim().to_string()
}

/// The Obsidian block marked `^id`: a paragraph, list item or heading ending
/// with the marker, or the block right before a paragraph that is only the
/// marker.
fn block(content: &str, id: &str) -> Result<String> {
    let marker = format!("^{id}");
    let marked_at = |range: &Range<usize>| {
        let text = content[range.clone()].trim_end();
        let rest = text.strip_suffix(marker.as_str())?;
        (rest.is_empty() || rest.ends_with(char::is_whitespace)).then(|| range.start + rest.len())
    };
    // Every block-level element, and the elements ending with the marker,
    // keyed by the marker's offset (nested elements can share one marker).
    let mut blocks: Vec<Range<usize>> = Vec::new();
    let mut marked: HashMap<usize, Range<usize>> = HashMap::new();
    let mut standalone: Vec<usize> = Vec::new();
    for (event, range) in markdown::events(content) {
        let Event::Start(tag) = event else {
            continue;
        };
        match tag {
            Tag::Paragraph if content[range.clone()].trim() == marker => {
                standalone.push(range.start);
                continue;
            }
            Tag::Paragraph | Tag::Item | Tag::Heading { .. } => {
                if let Some(at) = marked_at(&range) {
                    // Keep the outermost element, e.g. the item, not its paragraph.
                    let widest = marked.entry(at).or_insert_with(|| range.clone());
                    if range.start < widest.start {
                        *widest = range.clone();
                    }
                }
            }
            Tag::BlockQuote(_)
            | Tag::Table(_)
            | Tag::CodeBlock(_)
            | Tag::List(_)
            | Tag::HtmlBlock => {}
            _ => continue,
        }
        blocks.push(range);
    }
    let found: BTreeSet<usize> = marked
        .keys()
        .copied()
        .chain(standalone.iter().copied())
        .collect();
    match found.len() {
        0 => return Err(Error::AnchorNotFound),
        1 => {}
        _ => return Err(Error::AmbiguousAnchor),
    }
    if let Some((at, range)) = marked.into_iter().next() {
        return Ok(content[range.start..at].trim().to_string());
    }
    // A lone marker names the block that ends last before it (the outermost
    // one when several end together, e.g. a list and its last item).
    let at = standalone[0];
    let text_end = |range: &Range<usize>| range.start + content[range.clone()].trim_end().len();
    let target = blocks
        .iter()
        .filter(|range| range.end <= at)
        .max_by_key(|range| (text_end(range), std::cmp::Reverse(range.start)))
        .ok_or(Error::AnchorNotFound)?;
    Ok(content[target.clone()].trim().to_string())
}

/// The section of the heading path `Heading#Subheading`: from the heading to
/// the next heading of the same or a higher level. Matching ignores case.
fn section(content: &str, path: &str) -> Result<String> {
    struct Heading {
        start: usize,
        level: HeadingLevel,
        text: String,
    }
    let mut headings: Vec<Heading> = Vec::new();
    let mut open: Option<Heading> = None;
    for (event, range) in markdown::events(content) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                open = Some(Heading {
                    start: range.start,
                    level,
                    text: String::new(),
                });
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = &mut open {
                    heading.text.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => headings.extend(open.take()),
            _ => {}
        }
    }
    let mut range = 0..content.len();
    let mut parent: Option<HeadingLevel> = None;
    for part in path.split('#') {
        let part = part.trim().to_lowercase();
        let mut matches = headings.iter().enumerate().filter(|(_, h)| {
            range.contains(&h.start)
                && parent.is_none_or(|level| h.level > level)
                && h.text.trim().to_lowercase() == part
        });
        let (index, heading) = matches.next().ok_or(Error::AnchorNotFound)?;
        if matches.next().is_some() {
            return Err(Error::AmbiguousAnchor);
        }
        let end = headings[index + 1..]
            .iter()
            .find(|next| next.start < range.end && next.level <= heading.level)
            .map_or(range.end, |next| next.start);
        range = heading.start..end;
        parent = Some(heading.level);
    }
    Ok(content[range].trim().to_string())
}

#[derive(PartialEq)]
enum Kind<'a> {
    Anchor(&'a str),
    Heading,
    MessageHeader,
    Text,
    Blank,
}

/// Classify each line. Lines inside code blocks are plain text; a heading
/// counts only when its `#` starts the line.
fn classify(content: &str) -> Vec<(&str, Kind<'_>)> {
    let mut code: Vec<Range<usize>> = Vec::new();
    let mut headings = HashMap::new();
    for (event, range) in markdown::events(content) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code.push(range),
            Event::Start(Tag::Heading { level, .. }) if content[range.start..].starts_with('#') => {
                headings.insert(range.start, level);
            }
            _ => {}
        }
    }
    let mut code = code.into_iter().peekable();
    let mut offset = 0;
    content
        .split_inclusive('\n')
        .map(|chunk| {
            let start = offset;
            offset += chunk.len();
            let line = chunk.strip_suffix('\n').unwrap_or(chunk);
            let line = line.strip_suffix('\r').unwrap_or(line);
            while code.next_if(|block| block.end <= start).is_some() {}
            let kind = if code.peek().is_some_and(|block| block.start <= start) {
                Kind::Text
            } else if line.trim().is_empty() {
                Kind::Blank
            } else if let Some(id) = line.trim().strip_prefix("^message-") {
                Kind::Anchor(id)
            } else if let Some(level) = headings.get(&start) {
                if *level == HeadingLevel::H3 {
                    Kind::MessageHeader
                } else {
                    Kind::Heading
                }
            } else {
                Kind::Text
            };
            (line, kind)
        })
        .collect()
}

fn extract(content: &str, message_id: &str) -> Result<String> {
    let lines = classify(content);
    let mut anchors = lines
        .iter()
        .enumerate()
        .filter(|(_, (_, kind))| *kind == Kind::Anchor(message_id))
        .map(|(index, _)| index);
    let at = anchors.next().ok_or(Error::AnchorNotFound)?;
    if anchors.next().is_some() {
        return Err(Error::AmbiguousAnchor);
    }
    let previous = lines[..at]
        .iter()
        .rposition(|(_, kind)| matches!(kind, Kind::Anchor(_)))
        .map_or(0, |index| index + 1);
    let range = if let Some(header) = lines[previous..at]
        .iter()
        .rposition(|(_, kind)| *kind == Kind::MessageHeader)
    {
        previous + header..at
    } else if lines
        .get(at + 1)
        .is_some_and(|(_, kind)| *kind == Kind::Text)
    {
        let end = lines[at + 1..]
            .iter()
            .position(|(_, kind)| {
                matches!(kind, Kind::Anchor(_) | Kind::Heading | Kind::MessageHeader)
            })
            .map_or(lines.len(), |offset| at + 1 + offset);
        at + 1..end
    } else {
        return Err(Error::AnchorNotFound);
    };
    let text = lines[range]
        .iter()
        .map(|(line, _)| *line)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_anchor_without_previous_anchor_starts_at_last_header() {
        let content = "# Title\n\n## Branch\n\n### user · t\n\nHello\n\n^message-a\n\n### assistant\n\nBye\n\n^message-b\n";
        assert_eq!(extract(content, "a").unwrap(), "### user · t\n\nHello");
        assert_eq!(extract(content, "b").unwrap(), "### assistant\n\nBye");
    }

    #[test]
    fn anchor_with_neither_layout_is_rejected() {
        assert!(matches!(
            extract("# T\n\n^message-a\n\n### next\n\ntext\n", "a"),
            Err(Error::AnchorNotFound)
        ));
    }

    #[test]
    fn anchors_are_listed_once_per_occurrence() {
        let content = "---\nsource: x\n---\n### user\n\nHello\n\n^message-a\n\n\
                       A fact. ^fact-1\n\n- Item ^item-2\n\n```\n^message-in-code ^code\n```\n\n^message-a\n\n\
                       Table below.\n\n| a |\n|---|\n\n^table-3\n";
        let ids = anchor_ids(content);
        let messages: Vec<&str> = ids
            .iter()
            .filter_map(|a| match a {
                Anchor::Message(id) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let blocks: Vec<&str> = ids
            .iter()
            .filter_map(|a| match a {
                Anchor::Block(id) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(messages, ["a", "a"]);
        assert_eq!(blocks, ["fact-1", "item-2", "table-3"]);
        assert!(anchor_ids("# Only a heading\n\nPlain text.\n").is_empty());
    }

    #[test]
    fn content_not_yet_on_disk_can_be_located() {
        let content = "---\nsource: x\n---\n### user\n\nHello\n\n^message-a\n";
        assert_eq!(
            locate(content, true, Some(&Anchor::Message("a".into()))).unwrap(),
            "### user\n\nHello"
        );
        assert_eq!(
            locate(content, true, None).unwrap(),
            "### user\n\nHello\n\n^message-a"
        );
        assert!(matches!(
            locate(content, true, Some(&Anchor::Message("b".into()))),
            Err(Error::AnchorNotFound)
        ));
    }

    #[test]
    fn sniffing() {
        let md = Path::new("raw/a.md");
        assert_eq!(
            sniff(md, "# Héllo".as_bytes()),
            ("text/markdown".into(), true)
        );
        assert_eq!(
            sniff(Path::new("raw/x"), b"plain"),
            ("text/plain".into(), true)
        );
        assert_eq!(
            sniff(Path::new("raw/p.txt"), b"%PDF-1.7\n%\xe2\xe3"),
            ("application/pdf".into(), false)
        );
        assert_eq!(
            sniff(Path::new("raw/i.bin"), b"\x89PNG\r\n\x1a\n\0\0"),
            ("image/png".into(), false)
        );
        assert_eq!(
            sniff(Path::new("raw/d.dat"), b"ab\0cd"),
            ("application/octet-stream".into(), false)
        );
        // A character cut at the sniffing boundary is still text.
        assert!(sniff(md, &"é".as_bytes()[..1]).1);
    }

    #[test]
    fn whole_documents_drop_markdown_frontmatter_only() {
        let page = "---\nsource_url: x\n---\n# Site\n\nBody.\n";
        assert_eq!(document(page, true), "# Site\n\nBody.");
        assert_eq!(document(page, false), page.trim());
        assert_eq!(
            document("Intro\n\n---\nk: v\n---\n", true),
            "Intro\n\n---\nk: v\n---"
        );
    }

    #[test]
    fn obsidian_blocks() {
        let content = "Intro.\n\nThe claim continues\nhere. ^claim-1\n\n- one\n- two ^item\n\n\
                       | a | b |\n|---|---|\n| 1 | 2 |\n\n^table\n\n- x\n- y\n\n^list\n\n\
                       > quoted ^quote\n\n```\nnot ^code\n```\n\nDup ^dup\n\nDup ^dup\n";
        assert_eq!(
            block(content, "claim-1").unwrap(),
            "The claim continues\nhere."
        );
        assert_eq!(block(content, "item").unwrap(), "- two");
        assert_eq!(
            block(content, "table").unwrap(),
            "| a | b |\n|---|---|\n| 1 | 2 |"
        );
        assert_eq!(block(content, "list").unwrap(), "- x\n- y");
        assert_eq!(block(content, "quote").unwrap(), "quoted");
        assert!(matches!(block(content, "code"), Err(Error::AnchorNotFound)));
        assert!(matches!(block(content, "dup"), Err(Error::AmbiguousAnchor)));
        assert!(matches!(
            block(content, "claim"),
            Err(Error::AnchorNotFound)
        ));
        assert!(matches!(
            block("^lonely\n", "lonely"),
            Err(Error::AnchorNotFound)
        ));
    }

    #[test]
    fn heading_sections() {
        let content = "# Top\n\nIntro.\n\n## Setup\n\nA.\n\n### Detail\n\nB.\n\n## Usage\n\nC.\n\n\
                       # Other\n\n## Setup\n\nD.\n\n```\n# not a heading\n```\n";
        assert_eq!(
            section(content, "Top#Setup").unwrap(),
            "## Setup\n\nA.\n\n### Detail\n\nB."
        );
        assert_eq!(
            section(content, "top#setup#detail").unwrap(),
            "### Detail\n\nB."
        );
        assert_eq!(section(content, "Usage").unwrap(), "## Usage\n\nC.");
        assert!(section(content, "Other").unwrap().ends_with("```"));
        assert!(matches!(
            section(content, "Setup"),
            Err(Error::AmbiguousAnchor)
        ));
        assert!(matches!(
            section(content, "Other#Detail"),
            Err(Error::AnchorNotFound)
        ));
        assert!(matches!(
            section(content, "not a heading"),
            Err(Error::AnchorNotFound)
        ));
    }

    #[test]
    fn path_shapes() {
        for good in [
            "raw/chatgpt/a.md",
            "raw/a.txt",
            "raw/papers/p.pdf",
            "raw/img/w.png",
        ] {
            assert!(check_path(good).is_ok(), "{good}");
        }
        for bad in [
            "raw/../a.md",
            "raw//a.md",
            "/raw/a.md",
            "raw/./a.md",
            "raw\\a.md",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }
}
