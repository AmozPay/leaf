//! Locating, writing, inserting, replacing and removing passages in a page.
//!
//! Positions come from the viewer's parser (`parse::Passage::span`/`block`,
//! `parse::Heading::range`), so the editor changes exactly what the index
//! reads. A passage is written as one line of plain text followed by its
//! citations as `[[raw/…]]` links.

use crate::frontmatter::Document;
use leaf_common::{Citation, parse};
use std::ops::Range;

/// A passage found in a page, with ranges relative to the page body.
pub(crate) struct Located {
    pub heading: String,
    pub text: String,
    pub citations: Vec<Citation>,
    pub span: Range<usize>,
    pub block: Range<usize>,
    pub in_item: bool,
}

/// The parsed page of a document and where its body starts.
fn parsed(doc: &Document) -> Result<(parse::Page, usize), String> {
    let content = doc.render();
    let body_start = content.len() - doc.body.len();
    Ok((parse::page(&content)?, body_start))
}

/// The locators of every passage of a page, in order.
pub(crate) fn locators(doc: &Document) -> Result<Vec<String>, String> {
    Ok(parsed(doc)?
        .0
        .passages
        .into_iter()
        .map(|p| p.locator)
        .collect())
}

/// Find a passage by locator (`p-<hash>[-n]`).
pub(crate) fn find(doc: &Document, locator: &str) -> Result<Option<Located>, String> {
    let (page, start) = parsed(doc)?;
    Ok(page
        .passages
        .into_iter()
        .find(|p| p.locator == locator)
        .map(|p| Located {
            heading: p.heading,
            text: p.text,
            citations: p.citations,
            span: p.span.start - start..p.span.end - start,
            block: p.block.start - start..p.block.end - start,
            in_item: p.in_item,
        }))
}

/// The locator of the passage with this heading, text and citations.
pub(crate) fn locator_of(
    doc: &Document,
    heading: &str,
    text: &str,
    citations: &[Citation],
) -> Result<Option<String>, String> {
    let (page, _) = parsed(doc)?;
    Ok(page
        .passages
        .into_iter()
        .find(|p| p.heading == heading && p.text == text && same_citations(&p.citations, citations))
        .map(|p| p.locator))
}

/// Where a new passage goes under `heading` (body offset), and the heading
/// it will be read under; the page title means its introduction.
pub(crate) fn end_of_heading(
    doc: &Document,
    heading: &str,
) -> Result<Result<(usize, String), Vec<String>>, String> {
    let (page, start) = parsed(doc)?;
    let wanted = heading.trim().to_lowercase();
    let matching: Vec<&parse::Heading> = page
        .headings
        .iter()
        .filter(|h| h.text.to_lowercase() == wanted)
        .collect();
    let title = page.title.to_lowercase() == wanted;
    let name = match (matching.as_slice(), title) {
        ([one], _) => one.text.clone(),
        ([], true) => page.title.clone(),
        _ => {
            let all = page.headings.iter().map(|h| h.text.clone()).collect();
            return Ok(Err(all));
        }
    };
    // After the last passage read under this heading, else right after the
    // heading itself (or at the start of the body for an introduction).
    let after = page
        .passages
        .iter()
        .filter(|p| p.heading == name)
        .map(|p| p.block.end)
        .max()
        .or_else(|| matching.first().map(|h| h.range.end))
        .unwrap_or(start);
    Ok(Ok((after.saturating_sub(start), name)))
}

/// A heading found in a page, with ranges relative to the page body.
pub(crate) struct HeadingAt {
    pub text: String,
    pub level: usize,
    pub range: Range<usize>,
    /// Where its section ends: the next heading of the same or a higher
    /// level, or the end of the page.
    pub section_end: usize,
}

/// The one heading named `wanted` (ignoring case), or the page's headings
/// when there is no single one.
pub(crate) fn heading(
    doc: &Document,
    wanted: &str,
) -> Result<Result<HeadingAt, Vec<String>>, String> {
    let (page, start) = parsed(doc)?;
    let wanted = wanted.trim().to_lowercase();
    let matching: Vec<usize> = (0..page.headings.len())
        .filter(|&i| page.headings[i].text.to_lowercase() == wanted)
        .collect();
    let [found] = matching.as_slice() else {
        return Ok(Err(page.headings.iter().map(|h| h.text.clone()).collect()));
    };
    let this = &page.headings[*found];
    let section_end = page.headings[found + 1..]
        .iter()
        .find(|next| next.level <= this.level)
        .map_or(doc.body.len(), |next| next.range.start - start);
    Ok(Ok(HeadingAt {
        text: this.text.clone(),
        level: usize::from(this.level),
        range: this.range.start - start..this.range.end - start,
        section_end,
    }))
}

/// Whether the page has a heading named `text` (ignoring case) or is titled so.
pub(crate) fn has_heading(doc: &Document, text: &str) -> Result<bool, String> {
    let (page, _) = parsed(doc)?;
    let wanted = text.trim().to_lowercase();
    Ok(page.title.to_lowercase() == wanted
        || page
            .headings
            .iter()
            .any(|h| h.text.to_lowercase() == wanted))
}

/// Replace a heading's line by `markdown` (one line, no newline).
pub(crate) fn replace_heading(doc: &mut Document, range: Range<usize>, markdown: &str) {
    doc.body.replace_range(range, &format!("{markdown}\n"));
}

/// A passage as Markdown: its text, then its citations.
pub(crate) fn markdown(text: &str, citations: &[Citation]) -> String {
    let mut line = text.trim().to_string();
    for citation in citations {
        match citation.label.trim() {
            "" => line.push_str(&format!(" [[{citation}]]")),
            label => line.push_str(&format!(" [[{citation}|{label}]]")),
        }
    }
    line
}

/// Check new passage text: one line of plain text that reads back exactly as
/// written, within `limit` characters, citations given separately. Returns
/// the text as the index will read it.
pub(crate) fn check_text(
    text: &str,
    citations: &[Citation],
    limit: usize,
) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("the passage text is empty".into());
    }
    if trimmed.contains('\n') {
        return Err("a passage is one paragraph on one line; add several passages instead".into());
    }
    let length = trimmed.chars().count();
    if length > limit {
        return Err(format!(
            "the passage has {length} characters; the limit is {limit}"
        ));
    }
    if trimmed.contains("[[raw/") || trimmed.contains("^[raw/") {
        return Err("put citations in `citations`, not in the text".into());
    }
    let page = parse::page(&format!(
        "---\ntitle: Check\n---\n{}\n",
        markdown(trimmed, citations)
    ))?;
    let normalized = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    match page.passages.as_slice() {
        [passage]
            if page.warnings.is_empty()
                && passage.text == normalized
                && same_citations(&passage.citations, citations) =>
        {
            Ok(passage.text.clone())
        }
        _ => Err(
            "the text must be plain paragraph text (no headings, lists, quotes, code, HTML or Markdown formatting; page links as [[folder/page|label]])"
                .into(),
        ),
    }
}

fn same_citations(a: &[Citation], b: &[Citation]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_string() == y.to_string())
}

/// Insert a passage as its own paragraph after body offset `at`.
pub(crate) fn insert(doc: &mut Document, at: usize, markdown: &str) {
    let before = doc.body[..at].trim_end_matches('\n');
    let after = doc.body[at..].trim_start_matches('\n');
    let mut body = String::new();
    if !before.is_empty() {
        body.push_str(before);
        body.push_str("\n\n");
    }
    body.push_str(markdown);
    body.push('\n');
    if !after.is_empty() {
        body.push('\n');
        body.push_str(after);
    }
    doc.body = body;
}

/// Replace a passage's content (its marker and nested items stay).
pub(crate) fn replace(doc: &mut Document, span: Range<usize>, markdown: &str) {
    doc.body.replace_range(span, markdown);
}

/// Remove a passage's block, keeping one blank line between neighbors (or
/// none inside a list).
pub(crate) fn remove(doc: &mut Document, located: &Located) {
    let mut block = located.block.clone();
    if located.in_item {
        // A nested item's range starts at its marker; take its indentation too.
        let line_start = doc.body[..block.start].rfind('\n').map_or(0, |n| n + 1);
        if doc.body[line_start..block.start].trim().is_empty() {
            block.start = line_start;
        }
    }
    let before = &doc.body[..block.start];
    let after = &doc.body[block.end..];
    doc.body = if located.in_item {
        format!("{before}{after}")
    } else {
        let before = before.trim_end_matches('\n');
        let after = after.trim_start_matches('\n');
        match (before.is_empty(), after.is_empty()) {
            (true, _) => after.to_string(),
            (false, true) => format!("{before}\n"),
            (false, false) => format!("{before}\n\n{after}"),
        }
    };
}

/// Whether the block of `located` holds other passages (a list item with
/// nested items), which removing it would also remove.
pub(crate) fn holds_others(doc: &Document, located: &Located) -> Result<bool, String> {
    let (page, start) = parsed(doc)?;
    Ok(page.passages.iter().any(|p| {
        let span = p.span.start - start..p.span.end - start;
        span != located.span && located.block.start <= span.start && span.end <= located.block.end
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\ntitle: Garden\n---\n# Garden\n\nAn introduction.\n\n## Beds\n\n\
                        Beds are raised. [[raw/a.md]]\n\n- First **item**\n  - Nested\n- Second\n\n## Soil\n\nSoil is dark.\n";

    fn doc() -> Document {
        Document::parse(PAGE).unwrap()
    }

    fn locator(doc: &Document, text: &str) -> String {
        let (page, _) = parsed(doc).unwrap();
        page.passages
            .iter()
            .find(|p| p.text == text)
            .unwrap()
            .locator
            .clone()
    }

    #[test]
    fn passages_are_found_with_body_ranges() {
        let doc = doc();
        let found = find(&doc, &locator(&doc, "First item")).unwrap().unwrap();
        assert_eq!(&doc.body[found.span.clone()], "First **item**");
        assert!(found.in_item);
        assert!(holds_others(&doc, &found).unwrap());
        assert!(find(&doc, "p-000000000000").unwrap().is_none());
    }

    #[test]
    fn inserting_under_a_heading_and_after_a_passage() {
        let mut doc = doc();
        let (at, heading) = end_of_heading(&doc, "beds").unwrap().unwrap();
        assert_eq!(heading, "Beds");
        insert(&mut doc, at, "Beds face south. [[raw/b.md]]");
        assert!(
            doc.body
                .contains("- Second\n\nBeds face south. [[raw/b.md]]\n\n## Soil"),
            "{}",
            doc.body
        );
        let (at, heading) = end_of_heading(&doc, "Garden").unwrap().unwrap();
        assert_eq!(heading, "Garden");
        insert(&mut doc, at, "A second line of introduction.");
        assert!(
            doc.body.starts_with(
                "# Garden\n\nAn introduction.\n\nA second line of introduction.\n\n## Beds"
            ),
            "{}",
            doc.body
        );
        let unknown = end_of_heading(&doc, "Roots").unwrap().unwrap_err();
        assert_eq!(unknown, ["Garden", "Beds", "Soil"]);
    }

    #[test]
    fn replacing_keeps_markers_and_removing_keeps_spacing() {
        let mut doc = doc();
        let second = find(&doc, &locator(&doc, "Second")).unwrap().unwrap();
        replace(
            &mut doc,
            second.span.clone(),
            "Second, revised [[raw/c.md]]",
        );
        assert!(
            doc.body
                .contains("  - Nested\n- Second, revised [[raw/c.md]]\n")
        );
        let nested = find(&doc, &locator(&doc, "Nested")).unwrap().unwrap();
        remove(&mut doc, &nested);
        assert!(
            doc.body.contains("- First **item**\n- Second, revised"),
            "{}",
            doc.body
        );
        let soil = find(&doc, &locator(&doc, "Soil is dark."))
            .unwrap()
            .unwrap();
        remove(&mut doc, &soil);
        assert!(doc.body.ends_with("## Soil\n"), "{}", doc.body);
        let beds = find(&doc, &locator(&doc, "Beds are raised."))
            .unwrap()
            .unwrap();
        remove(&mut doc, &beds);
        assert!(doc.body.contains("## Beds\n\n- First"), "{}", doc.body);
    }

    #[test]
    fn new_text_must_read_back_exactly() {
        let cite: Vec<Citation> = vec!["raw/a.md#^message-m1".parse().unwrap()];
        assert_eq!(
            check_text(
                "  Beds face   south, see [[concepts/sun|Sun]].  ",
                &cite,
                2000
            )
            .unwrap(),
            "Beds face south, see [[concepts/sun|Sun]]."
        );
        for bad in [
            "",
            "Two\nlines",
            "# A heading",
            "- a list item",
            "> a quote",
            "Some **bold** text",
            "Cites [[raw/a.md]] inline",
            "<div>html</div>",
        ] {
            assert!(check_text(bad, &cite, 2000).is_err(), "{bad:?}");
        }
        assert!(check_text("Long enough", &cite, 5).is_err());
    }
}
