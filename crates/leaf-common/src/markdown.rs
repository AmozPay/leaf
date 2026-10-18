//! Frontmatter, headings, and paragraph/citation extraction for wiki pages.
//!
//! Markdown structure comes from `pulldown-cmark` (CommonMark plus YAML
//! metadata blocks, wikilinks and tables); the frontmatter is deserialized by
//! `serde-saphyr`. Paragraphs and list items (each item is its own passage)
//! are indexed under the nearest ATX or setext heading. Fenced and indented
//! code, tables, blockquotes, HTML blocks and later metadata blocks are skipped
//! and reported as warnings rather than misindexed. Inline markup such as
//! emphasis is reduced to its text; page wikilinks are kept as `[[target|label]]`.

use crate::facet;
use crate::model::Citation;
use crate::reference;
use pulldown_cmark::{
    BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd,
};
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::LazyLock;

/// Extensions enabled for every Markdown file Leaf reads.
const OPTIONS: Options = Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
    .union(Options::ENABLE_WIKILINKS)
    .union(Options::ENABLE_TABLES);

/// Markdown events of `text` with their byte ranges.
pub fn events(text: &str) -> impl Iterator<Item = (Event<'_>, Range<usize>)> {
    // `[raw/…]` has no link definition; resolving it here lets `^[raw/…]`
    // provenance markers (the baseline LLM-wiki citation style) arrive as
    // links. Other undefined references stay plain text.
    let raw_references = |link: BrokenLink<'_>| {
        link.reference
            .starts_with("raw/")
            .then(|| (link.reference.to_string().into(), CowStr::from("")))
    };
    Parser::new_with_broken_link_callback(text, OPTIONS, Some(raw_references)).into_offset_iter()
}

static SEPARATORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[ \t]*·[ \t]*){2,}").unwrap());
static NAV_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{N} '’/-]{1,60}:").unwrap());

#[derive(Debug)]
pub struct ParsedPage {
    pub title: String,
    pub confidence: Option<String>,
    pub contested: bool,
    /// Frontmatter facets (every key but `title`), cleaned by [`facet::clean_values`].
    pub facets: BTreeMap<String, Vec<String>>,
    pub passages: Vec<ParsedPassage>,
    pub headings: Vec<ParsedHeading>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct ParsedHeading {
    pub text: String,
    /// 1 to 6.
    pub level: u8,
    /// Byte range of the heading line(s) in the page.
    pub range: Range<usize>,
}

#[derive(Debug)]
pub struct ParsedPassage {
    /// Content-derived locator, unique within the page: `p-<hash>[-n]`.
    pub locator: String,
    pub heading: String,
    /// Claim text with raw citation links removed.
    pub text: String,
    /// Text used for ranking: page links reduced to their labels.
    pub search_text: String,
    /// Page-link targets, ranked with low weight.
    pub links: String,
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

/// Parse one published page. `Err` means the page must not be indexed.
pub fn parse_page(content: &str) -> Result<ParsedPage, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut events = events(content);
    match events.next() {
        Some((Event::Start(Tag::MetadataBlock(_)), range)) if range.start == 0 => {}
        _ if content.starts_with("---") => {
            return Err("YAML frontmatter is unclosed or empty".into());
        }
        _ => return Err("missing YAML frontmatter".into()),
    }
    let mut yaml = String::new();
    for (event, _) in events.by_ref() {
        match event {
            Event::Text(text) => yaml.push_str(&text),
            Event::End(TagEnd::MetadataBlock(_)) => break,
            _ => {}
        }
    }
    let meta = parse_frontmatter(&yaml)?;
    let title = meta.title.ok_or("frontmatter `title` is required")?;
    let mut body = Body::new(content, title.clone());
    body.warnings.extend(meta.warnings);
    for (event, range) in events {
        body.event(event, range);
    }
    body.flush();
    Ok(ParsedPage {
        title,
        confidence: meta.confidence,
        contested: meta.contested,
        facets: meta.facets,
        passages: body.passages,
        headings: body.headings,
        warnings: body.warnings,
    })
}

/// One frontmatter value: a list of scalars, a scalar, nothing, a map of
/// those (e.g. `relationship: {alice: [friend]}`), or anything deeper, which
/// is not a facet.
#[derive(Deserialize)]
#[serde(untagged)]
enum Field {
    List(Vec<Scalar>),
    One(Scalar),
    Empty(()),
    Map(BTreeMap<String, Leaf>),
    Other(serde::de::IgnoredAny),
}

/// A map entry's value.
#[derive(Deserialize)]
#[serde(untagged)]
enum Leaf {
    List(Vec<Scalar>),
    One(Scalar),
    Empty(()),
}

impl Leaf {
    fn into_values(self) -> Vec<String> {
        match self {
            Self::List(values) => values.into_iter().map(Scalar::into_text).collect(),
            Self::One(value) => vec![value.into_text()],
            Self::Empty(()) => Vec::new(),
        }
    }
}

/// Any YAML scalar, read as text (`2024` is "2024"). Needed because untagged
/// enums see typed values rather than raw scalars.
#[derive(Deserialize)]
#[serde(untagged)]
enum Scalar {
    Text(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
}

impl Scalar {
    fn into_text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Integer(n) => n.to_string(),
            Self::Float(x) => x.to_string(),
            Self::Bool(b) => b.to_string(),
        }
    }
}

struct Frontmatter {
    title: Option<String>,
    confidence: Option<String>,
    contested: bool,
    facets: BTreeMap<String, Vec<String>>,
    warnings: Vec<String>,
}

fn parse_frontmatter(yaml: &str) -> Result<Frontmatter, String> {
    let fields: BTreeMap<String, Field> = serde_saphyr::from_str(yaml).map_err(|err| {
        let err = err.to_string();
        let first = err.lines().next().unwrap_or_default();
        format!("invalid YAML frontmatter: {first}")
    })?;
    let mut title = None;
    let mut confidence = None;
    let mut contested = false;
    let mut facets = BTreeMap::new();
    let mut warnings = Vec::new();
    for (key, field) in fields {
        let key = facet::normalize_key(&key);
        if key == "title" {
            title = match field {
                Field::One(value) => Some(value.into_text()),
                Field::Empty(()) => None,
                _ => return Err("frontmatter `title` must be text".into()),
            };
            continue;
        }
        let values: Vec<String> = match field {
            Field::List(values) => values.into_iter().map(Scalar::into_text).collect(),
            Field::One(value) => vec![value.into_text()],
            Field::Empty(()) => Vec::new(),
            // `key: {a: [x, y]}` is the facet `key` = [a] plus `key.a` = [x, y].
            Field::Map(_) if facet::WARN_IF_MALFORMED.contains(&key.as_str()) => {
                warnings.push(format!(
                    "frontmatter `{key}` is not a list of strings and was ignored"
                ));
                continue;
            }
            Field::Map(entries) => {
                let mut names = Vec::new();
                for (name, leaf) in entries {
                    let name = facet::normalize_key(&name);
                    let values = facet::clean_values(&name, leaf.into_values());
                    facets.insert(format!("{key}.{name}"), values);
                    names.push(name);
                }
                names
            }
            Field::Other(_) => {
                if facet::WARN_IF_MALFORMED.contains(&key.as_str()) {
                    warnings.push(format!(
                        "frontmatter `{key}` is not a list of strings and was ignored"
                    ));
                }
                continue;
            }
        };
        match key.as_str() {
            "confidence" => confidence = values.first().map(|c| c.trim().to_string()),
            "contested" => {
                contested = match values.first().map(|v| v.trim().to_lowercase()).as_deref() {
                    None | Some("false" | "no" | "off") => false,
                    Some("true" | "yes" | "on") => true,
                    Some(other) => {
                        return Err(format!("`contested` must be true or false, not `{other}`"));
                    }
                }
            }
            _ => {}
        }
        let values = facet::clean_values(&key, values);
        facets.insert(key, values);
    }
    let title = title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    Ok(Frontmatter {
        title,
        confidence: confidence.filter(|c| !c.is_empty()),
        contested,
        facets,
        warnings,
    })
}

/// The byte range of a file's frontmatter block (from the opening `---` to
/// after the closing line) and its YAML text; `None` when it has none.
pub fn frontmatter_block(content: &str) -> Option<(Range<usize>, String)> {
    let mut events = events(content);
    let (Event::Start(Tag::MetadataBlock(_)), range) = events.next()? else {
        return None;
    };
    if range.start != 0 {
        return None;
    }
    let mut yaml = String::new();
    for (event, _) in events {
        match event {
            Event::Text(text) => yaml.push_str(&text),
            Event::End(TagEnd::MetadataBlock(_)) => break,
            _ => {}
        }
    }
    // Include the closing line's line ending.
    let rest = &content[range.end..];
    let end = range.end + rest.len()
        - rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or(rest)
            .len();
    Some((range.start..end, yaml))
}

/// The frontmatter of any Markdown file (a page or a raw source) as facets,
/// `title` included; `Ok(None)` when the file has none.
pub fn frontmatter(content: &str) -> Result<Option<BTreeMap<String, Vec<String>>>, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut events = events(content);
    match events.next() {
        Some((Event::Start(Tag::MetadataBlock(_)), range)) if range.start == 0 => {}
        _ => return Ok(None),
    }
    let mut yaml = String::new();
    for (event, _) in events {
        match event {
            Event::Text(text) => yaml.push_str(&text),
            Event::End(TagEnd::MetadataBlock(_)) => break,
            _ => {}
        }
    }
    let meta = parse_frontmatter(&yaml)?;
    let mut facets = meta.facets;
    if let Some(title) = meta.title {
        facets.insert("title".into(), vec![title]);
    }
    Ok(Some(facets))
}

/// A paragraph or list item being collected.
#[derive(Default)]
struct Passage {
    text: String,
    search_text: String,
    /// Text outside page links, for navigation detection.
    prose: String,
    links: Vec<String>,
    citations: Vec<Citation>,
    span: Option<Range<usize>>,
}

impl Passage {
    fn extend_span(&mut self, range: Range<usize>) {
        self.span = Some(match self.span.take() {
            Some(span) => span.start.min(range.start)..span.end.max(range.end),
            None => range,
        });
    }
}

/// A link whose label is being collected.
struct Link {
    wiki: bool,
    has_label: bool,
    /// An undefined `[raw/…]` reference, a citation when written `^[raw/…]`.
    raw_reference: bool,
    dest: String,
    label: String,
}

struct Body<'a> {
    source: &'a str,
    heading: String,
    /// Text of the heading being read.
    pending_heading: Option<String>,
    /// Level and range of the heading being read.
    heading_start: Option<(u8, Range<usize>)>,
    headings: Vec<ParsedHeading>,
    /// The open paragraph's range, and the open list items' ranges.
    paragraph: Option<Range<usize>>,
    items: Vec<Range<usize>>,
    passage: Option<Passage>,
    link: Option<Link>,
    /// Nesting depth inside a skipped block; 0 when not skipping.
    skipping: usize,
    seen: HashMap<String, usize>,
    passages: Vec<ParsedPassage>,
    warnings: Vec<String>,
}

impl<'a> Body<'a> {
    fn new(source: &'a str, title: String) -> Self {
        Self {
            source,
            heading: title,
            pending_heading: None,
            heading_start: None,
            headings: Vec::new(),
            paragraph: None,
            items: Vec::new(),
            passage: None,
            link: None,
            skipping: 0,
            seen: HashMap::new(),
            passages: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn event(&mut self, event: Event, range: Range<usize>) {
        if self.skipping > 0 {
            match event {
                Event::Start(_) => self.skipping += 1,
                Event::End(_) => self.skipping -= 1,
                _ => {}
            }
            return;
        }
        // Inline content and markup, including link and emphasis markers,
        // belong to the passage's span.
        let inline = matches!(
            event,
            Event::Text(_)
                | Event::Code(_)
                | Event::InlineHtml(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::Start(
                    Tag::Link { .. }
                        | Tag::Image { .. }
                        | Tag::Emphasis
                        | Tag::Strong
                        | Tag::Strikethrough
                )
                | Event::End(
                    TagEnd::Link
                        | TagEnd::Image
                        | TagEnd::Emphasis
                        | TagEnd::Strong
                        | TagEnd::Strikethrough
                )
        );
        if inline && self.pending_heading.is_none() && self.link.is_none() {
            self.passage
                .get_or_insert_default()
                .extend_span(range.clone());
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Code(text) | Event::InlineHtml(text) => self.push(&text),
            Event::SoftBreak | Event::HardBreak => self.push(" "),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag, range: Range<usize>) {
        let offset = range.start;
        let unsupported = match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.pending_heading = Some(String::new());
                self.heading_start = Some((level as u8, range));
                return;
            }
            Tag::Paragraph => {
                self.flush();
                self.paragraph = Some(range);
                return;
            }
            Tag::Item => {
                self.flush();
                self.items.push(range);
                return;
            }
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                self.link = Some(Link {
                    wiki: matches!(link_type, LinkType::WikiLink { .. }),
                    has_label: matches!(link_type, LinkType::WikiLink { has_pothole: true }),
                    raw_reference: link_type == LinkType::ShortcutUnknown
                        && dest_url.starts_with("raw/"),
                    dest: dest_url.into_string(),
                    label: String::new(),
                });
                return;
            }
            Tag::CodeBlock(CodeBlockKind::Fenced(_)) => "fenced code block",
            Tag::CodeBlock(CodeBlockKind::Indented) => "indented code block",
            Tag::Table(_) => "table",
            Tag::BlockQuote(_) => "blockquote",
            Tag::HtmlBlock => "HTML block",
            Tag::MetadataBlock(_) => "metadata block after the frontmatter",
            // Inline markup (emphasis, images, …) contributes its text.
            _ => return,
        };
        self.flush();
        self.skipping = 1;
        self.warn(offset, unsupported);
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                if let Some(heading) = self.pending_heading.take() {
                    self.heading = clean(&heading);
                    if let Some((level, range)) = self.heading_start.take() {
                        self.headings.push(ParsedHeading {
                            text: self.heading.clone(),
                            level,
                            range,
                        });
                    }
                }
            }
            TagEnd::Paragraph => {
                self.flush();
                self.paragraph = None;
            }
            TagEnd::Item => {
                self.flush();
                self.items.pop();
            }
            TagEnd::Link => self.finish_link(),
            _ => {}
        }
    }

    fn push(&mut self, text: &str) {
        if let Some(link) = &mut self.link {
            link.label.push_str(text);
        } else if let Some(heading) = &mut self.pending_heading {
            heading.push_str(text);
        } else {
            let passage = self.passage.get_or_insert_default();
            passage.text.push_str(text);
            passage.search_text.push_str(text);
            passage.prose.push_str(text);
        }
    }

    fn finish_link(&mut self) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        if link.raw_reference {
            // `^[raw/…]` cites like `[[raw/…]]`; a bare `[raw/…]` stays text.
            let marked =
                self.pending_heading.is_none() && self.passage.as_mut().is_some_and(remove_marker);
            if !marked {
                let literal = format!("[{}]", link.label);
                return self.push(&literal);
            }
            link.wiki = true;
            link.has_label = false;
        }
        let label = link.label.trim();
        if link.wiki && link.dest.starts_with("raw/") {
            let label = if link.has_label { label } else { "" };
            let citation = reference::parse_target(&link.dest).map(|(raw_path, anchor)| Citation {
                raw_path,
                anchor,
                label: label.to_string(),
            });
            match (citation, &self.pending_heading) {
                (Some(_), Some(_)) => self.warnings.push(format!(
                    "citation `[[{}]]` in a heading was ignored",
                    link.dest
                )),
                (Some(citation), None) => {
                    let citations = &mut self.passage.get_or_insert_default().citations;
                    if !citations.contains(&citation) {
                        citations.push(citation);
                    }
                }
                (None, _) => self.warnings.push(format!(
                    "raw link `[[{}]]` is not a safe raw path with a supported anchor and was ignored",
                    link.dest
                )),
            }
            return;
        }
        if let Some(heading) = &mut self.pending_heading {
            heading.push_str(if link.wiki && !link.has_label {
                &link.dest
            } else {
                label
            });
            return;
        }
        let passage = self.passage.get_or_insert_default();
        if link.wiki {
            if link.has_label {
                passage.text.push_str(&format!("[[{}|{label}]]", link.dest));
                passage.search_text.push_str(label);
            } else {
                passage.text.push_str(&format!("[[{}]]", link.dest));
            }
            passage.links.push(link.dest);
        } else {
            passage.text.push_str(label);
            passage.search_text.push_str(label);
            passage.prose.push_str(label);
        }
    }

    fn warn(&mut self, offset: usize, kind: &str) {
        let line = self.source[..offset].matches('\n').count() + 1;
        self.warnings.push(format!(
            "line {line}: {kind} is not supported and was not indexed"
        ));
    }

    fn flush(&mut self) {
        let Some(passage) = self.passage.take() else {
            return;
        };
        let text = clean(&passage.text);
        if text.is_empty() {
            if !passage.citations.is_empty() {
                self.warnings
                    .push("citations without claim text were not indexed".into());
            }
            return;
        }
        if passage.citations.is_empty()
            && !passage.links.is_empty()
            && is_navigation(&passage.prose)
        {
            return;
        }
        let mut hasher = Sha256::new();
        hasher.update(format!("{}\u{1f}{text}", self.heading));
        for citation in &passage.citations {
            hasher.update(format!("\u{1f}{citation}"));
        }
        let hash: String = hasher.finalize()[..6]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let span = passage.span.clone().unwrap_or(0..0);
        let block = self
            .items
            .last()
            .or(self.paragraph.as_ref())
            .cloned()
            .unwrap_or_else(|| span.clone());
        let occurrence = self.seen.entry(hash.clone()).or_default();
        *occurrence += 1;
        let locator = match *occurrence {
            1 => format!("p-{hash}"),
            n => format!("p-{hash}-{n}"),
        };
        self.passages.push(ParsedPassage {
            locator,
            heading: self.heading.clone(),
            text,
            search_text: clean(&passage.search_text),
            links: passage.links.join(" "),
            citations: passage.citations,
            span,
            block,
            in_item: !self.items.is_empty(),
        });
    }
}

/// Remove the `^` of a `^[raw/…]` provenance marker just collected, if any.
fn remove_marker(passage: &mut Passage) -> bool {
    let marked = passage.text.ends_with('^');
    if marked {
        for text in [
            &mut passage.text,
            &mut passage.search_text,
            &mut passage.prose,
        ] {
            if text.ends_with('^') {
                text.pop();
            }
        }
    }
    marked
}

/// Tidy text left behind after removing citation links (e.g. `claim. · ·`).
fn clean(text: &str) -> String {
    let collapsed = SEPARATORS.replace_all(text, " · ");
    let words: Vec<&str> = collapsed.split_whitespace().collect();
    words
        .join(" ")
        .trim_matches(|c: char| c == '·' || c.is_whitespace())
        .to_string()
}

/// A paragraph of page links with at most a short label, such as
/// `Related: [[a]], [[b]], and [[c]].`, is navigation rather than evidence.
/// `prose` is the paragraph's text outside its page links.
fn is_navigation(prose: &str) -> bool {
    let rest = NAV_LABEL.replace(prose.trim_start(), "");
    rest.split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | ';' | '·' | '&'))
        .all(|word| word.is_empty() || matches!(word.to_lowercase().as_str(), "and" | "or"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Anchor;

    fn passages(body: &str) -> Vec<ParsedPassage> {
        parse_page(&format!("---\ntitle: T\n---\n{body}"))
            .unwrap()
            .passages
    }

    fn texts(body: &str) -> Vec<String> {
        passages(body).into_iter().map(|p| p.text).collect()
    }

    #[test]
    fn frontmatter_scalars() {
        let page = parse_page(
            "\u{feff}---\ntitle: \"Quoted \\\"title\\\"\"\nconfidence: 'low'\ncontested: true\ntags: [a]\n---\nBody.\n",
        )
        .unwrap();
        assert_eq!(page.title, "Quoted \"title\"");
        assert_eq!(page.confidence.as_deref(), Some("low"));
        assert!(page.contested);
        assert!(parse_page("---\ntitle: A\ntitle: B\n---\n").is_err());
        assert!(parse_page("---\ntitle: \"open\n---\n").is_err());
        assert!(parse_page("---\ntitle:\n---\n").is_err());
    }

    #[test]
    fn frontmatter_yaml_edge_cases() {
        let page = |yaml: &str| parse_page(&format!("---\n{yaml}---\nBody.\n"));
        // Block scalars, numbers, comments and nested unknown keys are real YAML.
        assert_eq!(
            page("title: |\n  Block title\n").unwrap().title,
            "Block title"
        );
        assert_eq!(page("title: 1984\n").unwrap().title, "1984");
        assert_eq!(page("title: A # note\n").unwrap().title, "A");
        let nested =
            page("title: A\nconfidence: 0.8\nsources:\n  - {path: raw/x.md, ids: [1, 2]}\n")
                .unwrap();
        assert_eq!(nested.confidence.as_deref(), Some("0.8"));
        assert!(!nested.contested);
        // Malformed or ambiguous YAML is an error, never a guess.
        for bad in [
            "title: A\nx: 1\nx: 2\n",
            "title: [a]\n",
            "- title\n",
            "title: A\ncontested: maybe\n",
            "title: '   '\n",
        ] {
            assert!(page(bad).is_err(), "{bad:?}");
        }
        // Frontmatter must open the page.
        assert!(parse_page("Intro.\n\n---\ntitle: T\n---\n").is_err());
        assert!(parse_page("---\n---\nBody.\n").is_err());
        assert!(parse_page("---\n\ntitle: T\n---\n").is_err());
    }

    #[test]
    fn frontmatter_facets() {
        let facets = |yaml: &str| {
            let page = parse_page(&format!("---\ntitle: T\n{yaml}---\nBody.\n")).unwrap();
            (page.facets, page.warnings)
        };
        let (found, warnings) = facets(
            "tags: [Software, '#dns', software, ' ', 2024]\nrelated-people: [alice, Chloe-Dubois]\n\
             related-places: []\nAliases: Bobby\nconfidence: high\nnotes:\nsources:\n  - raw/a.md\n\
             nested: {a: [1]}\nrelationship:\n  Alice: [friend, colleague]\n  chloe-dubois: friend\n\
             deep: {a: {b: c}}\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let get = |key: &str| found.get(key).cloned().unwrap_or_default();
        assert_eq!(get("tags"), ["2024", "dns", "software"]);
        assert_eq!(get("related-people"), ["alice", "Chloe-Dubois"]);
        assert!(found.contains_key("related-places") && get("related-places").is_empty());
        assert_eq!(get("aliases"), ["Bobby"]);
        assert_eq!(get("confidence"), ["high"]);
        assert_eq!(get("sources"), ["raw/a.md"]);
        assert!(found.contains_key("notes"));
        assert_eq!(get("nested"), ["a"]);
        assert_eq!(get("nested.a"), ["1"]);
        assert_eq!(get("relationship"), ["alice", "chloe-dubois"]);
        assert_eq!(get("relationship.alice"), ["friend", "colleague"]);
        assert_eq!(get("relationship.chloe-dubois"), ["friend"]);
        assert!(!found.contains_key("deep") && !found.contains_key("title"));

        for malformed in ["tags: {a: b}\n", "related-people: [[nested]]\n"] {
            let (found, warnings) = facets(malformed);
            assert_eq!(warnings.len(), 1, "{malformed}");
            assert!(found.is_empty(), "{malformed}");
        }
        assert!(parse_page("---\ntitle: {a: b}\n---\n").is_err());
        assert!(
            parse_page("---\ntitle: T\ncontested: yes\n---\n")
                .unwrap()
                .contested
        );
    }

    #[test]
    fn headings_and_list_items() {
        let found = passages(
            "# Title #\n\nIntro line one\nline two.\n\n## Section\n- First item\n  continues.\n* Second\n1. Third\n",
        );
        let texts: Vec<_> = found.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Intro line one line two.",
                "First item continues.",
                "Second",
                "Third"
            ]
        );
        assert_eq!(found[0].heading, "Title");
        assert_eq!(found[1].heading, "Section");
    }

    #[test]
    fn markdown_edge_cases() {
        let found = passages(
            "Setext\n======\n\nUnder **bold** and _em_ with `code` and \\*escaped\\*.\n\n\
             - Parent\n  - Child [link](https://example.org) end\n\nNot a #heading line.\n",
        );
        assert_eq!(found[0].heading, "Setext");
        let texts: Vec<_> = found.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Under bold and em with code and *escaped*.",
                "Parent",
                "Child link end",
                "Not a #heading line."
            ]
        );
    }

    #[test]
    fn citations_are_extracted_and_removed_from_text() {
        let found = passages(
            "Claim [[concepts/x|X]] here. [[raw/a.md#^message-1|One]] · [[raw/b.md#^message-2]] · [[raw/b.md#^message-2|Dup]]\n",
        );
        assert_eq!(found[0].text, "Claim [[concepts/x|X]] here.");
        assert_eq!(found[0].search_text, "Claim X here.");
        assert_eq!(found[0].links, "concepts/x");
        let ids: Vec<_> = found[0].citations.iter().map(|c| c.to_string()).collect();
        assert_eq!(
            ids,
            [
                "raw/a.md#^message-1",
                "raw/b.md#^message-2",
                "raw/b.md#^message-2"
            ]
        );
        assert_eq!(found[0].citations[1].label, "");
    }

    #[test]
    fn citation_paths_with_markdown_punctuation() {
        let found =
            passages("Claim *with* emphasis. [[raw/chat_gpt/a_b*c.md#^message-x_1-y|Label *x*]]\n");
        assert_eq!(found[0].text, "Claim with emphasis.");
        assert_eq!(found[0].citations[0].raw_path, "raw/chat_gpt/a_b*c.md");
        assert_eq!(
            found[0].citations[0].anchor,
            Some(Anchor::Message("x_1-y".into()))
        );
    }

    #[test]
    fn citations_in_code_are_not_citations() {
        let page = parse_page(
            "---\ntitle: T\n---\nUse `[[raw/a.md#^message-1]]` here.\n\n```\n[[raw/b.md#^message-2]]\n```\n",
        )
        .unwrap();
        assert_eq!(page.passages.len(), 1);
        assert_eq!(page.passages[0].text, "Use [[raw/a.md#^message-1]] here.");
        assert!(page.passages[0].citations.is_empty());
        assert_eq!(page.warnings.len(), 1);
        assert!(page.warnings[0].starts_with("line 6: fenced code block"));
    }

    #[test]
    fn citations_in_headings_and_later_metadata_blocks_are_warned() {
        let page = parse_page(
            "---\ntitle: T\n---\n## H [[raw/a.md#^message-1]]\n\nClaim.\n\n---\nkey: v\n---\n",
        )
        .unwrap();
        assert_eq!(page.passages[0].heading, "H");
        assert!(page.passages[0].citations.is_empty());
        assert_eq!(page.warnings.len(), 2, "{:?}", page.warnings);
    }

    #[test]
    fn malformed_raw_links_are_warned_not_cited() {
        let page = parse_page(
            "---\ntitle: T\n---\nClaim. [[raw/../x.md#^message-1]] [[raw/a.md#^message-bad id]] [[raw/a.md#^bad_block]]\n",
        )
        .unwrap();
        assert!(page.passages[0].citations.is_empty());
        assert_eq!(page.passages[0].text, "Claim.");
        assert_eq!(page.warnings.len(), 3);
    }

    #[test]
    fn provenance_markers_cite_like_raw_wikilinks() {
        let found = passages(
            "Claim one.^[raw/articles/site.md] Claim two. ^[raw/papers/p.pdf#Intro]\n\n\
             Brackets [raw/notes.md] and [undefined] stay text.\n",
        );
        assert_eq!(found[0].text, "Claim one. Claim two.");
        let cited: Vec<_> = found[0].citations.iter().map(|c| c.to_string()).collect();
        assert_eq!(cited, ["raw/articles/site.md", "raw/papers/p.pdf#Intro"]);
        assert!(found[0].citations.iter().all(|c| c.label.is_empty()));
        assert_eq!(
            found[1].text,
            "Brackets [raw/notes.md] and [undefined] stay text."
        );
        assert!(found[1].citations.is_empty());
        // Unsafe targets are warned about like `[[raw/…]]` ones.
        let page = parse_page("---\ntitle: T\n---\nClaim.^[raw/../x.md]\n").unwrap();
        assert!(page.passages[0].citations.is_empty());
        assert_eq!(page.warnings.len(), 1);
    }

    #[test]
    fn any_raw_file_and_anchor_kind_can_be_cited() {
        let found = passages(
            "Claim. [[raw/articles/site]] [[raw/papers/p.pdf|Paper]] [[raw/n.md#^blk]] [[raw/n.md#Intro#Deeper|Section]]\n",
        );
        let cited: Vec<_> = found[0].citations.iter().map(|c| c.to_string()).collect();
        assert_eq!(
            cited,
            [
                "raw/articles/site.md",
                "raw/papers/p.pdf",
                "raw/n.md#^blk",
                "raw/n.md#Intro#Deeper"
            ]
        );
        assert_eq!(found[0].citations[1].label, "Paper");
        assert_eq!(found[0].text, "Claim.");
    }

    #[test]
    fn navigation_detection() {
        for nav in [
            "Related: [[a|A]], [[b]], and [[c]].",
            "Other thematic pages: [[a]] and [[b]].",
            "[[a]] · [[b]]",
            "Related seed-library work: [[concepts/seed-library-and-catalogue]].",
        ] {
            assert!(texts(nav).is_empty(), "{nav}");
        }
        for evidence in [
            "Related equipment context: [[concepts/lamp]]. Alice confirmed the lamp is the same.",
            "Tally was built by [[entities/alice]].",
            "Related work continued in 2024.",
            "Related: [[a]] and [[b]]. [[raw/a.md#^message-1]]",
        ] {
            assert_eq!(texts(evidence).len(), 1, "{evidence}");
        }
    }

    #[test]
    fn locators_are_content_derived_and_unique() {
        let first = passages("Same.\n\nSame.\n\nOther.\n");
        assert_eq!(first[0].locator.len(), "p-".len() + 12);
        assert_eq!(first[1].locator, format!("{}-2", first[0].locator));
        let shifted = passages("New.\n\nOther.\n");
        assert_eq!(shifted[1].locator, first[2].locator);
    }
}
