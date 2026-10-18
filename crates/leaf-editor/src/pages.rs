//! Whole-page helpers: checking headings, titles and slugs, assembling the
//! body of a new page, and the links a page makes to other pages.

use leaf_common::parse;
use std::collections::BTreeSet;

/// Heading (or title) text: one line of plain text that reads back exactly
/// as given as a heading of `level`. Returns it with whitespace collapsed.
pub(crate) fn check_heading(what: &str, text: &str, level: usize) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("the {what} is empty"));
    }
    if text.contains('\n') {
        return Err(format!("the {what} must be one line"));
    }
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let page = parse::page(&format!(
        "---\ntitle: Check\n---\n{} {normalized}\n",
        "#".repeat(level)
    ))?;
    match page.headings.as_slice() {
        [heading]
            if heading.text == normalized
                && usize::from(heading.level) == level
                && page.passages.is_empty()
                && page.warnings.is_empty() =>
        {
            Ok(normalized)
        }
        _ => Err(format!(
            "the {what} must be plain text (no Markdown formatting)"
        )),
    }
}

/// A one-line description for `index.md`.
pub(crate) fn check_summary(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the summary is empty".into());
    }
    if text.contains('\n') {
        return Err("the summary must be one line".into());
    }
    Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Whether `slug` is a lowercase, hyphenated file name: letters, digits and
/// single hyphens, not starting or ending with one.
pub(crate) fn is_slug(slug: &str) -> bool {
    !slug.is_empty()
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The body of a new page: its title, introduction paragraphs and
/// second-level sections, each paragraph already Markdown.
pub(crate) fn body(title: &str, intro: &[String], sections: &[(String, Vec<String>)]) -> String {
    let mut blocks = vec![format!("# {title}")];
    blocks.extend(intro.iter().cloned());
    for (heading, passages) in sections {
        blocks.push(format!("## {heading}"));
        blocks.extend(passages.iter().cloned());
    }
    let mut body = blocks.join("\n\n");
    body.push('\n');
    body
}

/// What is wrong with the links a page body makes to other pages, given the
/// pages that are published: unusable links, links to pages that do not
/// exist, and fewer than two links to other pages.
pub(crate) fn link_problems(path: &str, body: &str, published: &BTreeSet<String>) -> Vec<String> {
    let (linked, errors) = parse::published_pages(body);
    let mut problems: Vec<String> = errors
        .into_iter()
        .map(|issue| format!("link `{}`: {}", issue.path, issue.message))
        .collect();
    let mut resolved = 0;
    for page in linked.iter().filter(|page| page.as_str() != path) {
        if published.contains(page) {
            resolved += 1;
        } else {
            problems.push(format!("links to `{page}`, which is not a published page"));
        }
    }
    if resolved < 2 {
        problems.push(format!(
            "has {resolved} link(s) to other pages; at least 2 are required (`[[folder/page|label]]`)"
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_must_read_back() {
        assert_eq!(
            check_heading("title", "  Raised   beds ", 1).unwrap(),
            "Raised beds"
        );
        for bad in ["", "two\nlines", "**Bold**", "[[a]]", "`code`"] {
            assert!(check_heading("title", bad, 2).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn slugs_are_lowercase_and_hyphenated() {
        for good in ["a", "raised-beds", "route-66"] {
            assert!(is_slug(good), "{good}");
        }
        for bad in ["", "Raised", "a b", "-a", "a-", "a--b", "é"] {
            assert!(!is_slug(bad), "{bad}");
        }
    }

    #[test]
    fn links_need_targets_and_two_of_them() {
        let published: BTreeSet<String> = ["concepts/a.md", "concepts/b.md", "concepts/me.md"]
            .map(String::from)
            .into();
        let body = "See [[concepts/a|A]], [[concepts/b]] and [[concepts/me]].\n";
        assert!(link_problems("concepts/me.md", body, &published).is_empty());
        let one = link_problems("concepts/me.md", "Only [[concepts/a]].\n", &published);
        assert_eq!(one.len(), 1);
        assert!(one[0].contains("1 link"));
        let broken = link_problems(
            "concepts/me.md",
            "[[concepts/a]] [[concepts/b]] [[concepts/gone]]\n",
            &published,
        );
        assert_eq!(broken.len(), 1);
        assert!(broken[0].contains("concepts/gone.md"));
    }
}
