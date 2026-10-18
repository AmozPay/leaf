//! Citation reference syntax: `raw/<path>[#anchor]`, also as a
//! `[[raw/<path>[#anchor]|label]]` wikilink.
//!
//! Anchors: `^message-<id>` (transcript message), `^<id>` (Obsidian block),
//! anything else a heading path (`Heading` or `Heading#Subheading`). A path
//! without an extension is a Markdown file, as in Obsidian links.

use crate::model::{Anchor, Citation, Error};
use crate::publication::is_plain_relative;
use std::str::FromStr;

/// The source path and anchor of a `raw/…` link destination, or `None` when
/// it is not a safe raw path with a well-formed anchor.
pub fn parse_target(target: &str) -> Option<(String, Option<Anchor>)> {
    let (path, fragment) = match target.split_once('#') {
        Some((path, fragment)) => (path.trim(), Some(fragment)),
        None => (target.trim(), None),
    };
    if !path.starts_with("raw/") || !is_plain_relative(path) {
        return None;
    }
    let file = path.rsplit('/').next().unwrap_or_default();
    let path = if file.contains('.') {
        path.to_string()
    } else {
        format!("{path}.md")
    };
    let anchor = match fragment {
        Some(fragment) => Some(parse_anchor(fragment)?),
        None => None,
    };
    Some((path, anchor))
}

fn parse_anchor(fragment: &str) -> Option<Anchor> {
    let is_id = |id: &str, extra: &[char]| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || extra.contains(&c))
    };
    if let Some(id) = fragment.strip_prefix("^message-") {
        return is_id(id, &['_']).then(|| Anchor::Message(id.to_string()));
    }
    if let Some(id) = fragment.strip_prefix('^') {
        return is_id(id, &[]).then(|| Anchor::Block(id.to_string()));
    }
    let path: Vec<&str> = fragment.split('#').map(str::trim).collect();
    let valid = path
        .iter()
        .all(|part| !part.is_empty() && !part.chars().any(char::is_control));
    valid.then(|| Anchor::Heading(path.join("#")))
}

impl FromStr for Citation {
    type Err = Error;

    fn from_str(reference: &str) -> Result<Self, Error> {
        let trimmed = reference.trim();
        let inner = trimmed
            .strip_prefix("[[")
            .and_then(|rest| rest.strip_suffix("]]"))
            .unwrap_or(trimmed);
        let (target, label) = inner.split_once('|').unwrap_or((inner, ""));
        let (raw_path, anchor) = parse_target(target).ok_or_else(|| {
            Error::InvalidReference(format!(
                "`{reference}` is not raw/<path>, raw/<path>#^message-<id>, raw/<path>#^<block-id> or raw/<path>#<heading>"
            ))
        })?;
        Ok(Citation {
            raw_path,
            anchor,
            label: label.trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(reference: &str) -> (String, Option<Anchor>) {
        let citation: Citation = reference.parse().unwrap();
        (citation.raw_path, citation.anchor)
    }

    #[test]
    fn reference_forms() {
        let message = Some(Anchor::Message("m_1-x".into()));
        assert_eq!(
            parsed("raw/c/a.md#^message-m_1-x"),
            ("raw/c/a.md".into(), message.clone())
        );
        assert_eq!(
            parsed(" [[raw/c/a.md#^message-m_1-x|R]] "),
            ("raw/c/a.md".into(), message)
        );
        assert_eq!(
            parsed("raw/papers/p.pdf"),
            ("raw/papers/p.pdf".into(), None)
        );
        assert_eq!(
            parsed("raw/articles/site"),
            ("raw/articles/site.md".into(), None)
        );
        assert_eq!(
            parsed("raw/a.md#^abc-1"),
            ("raw/a.md".into(), Some(Anchor::Block("abc-1".into())))
        );
        assert_eq!(
            parsed("raw/a.md#Intro # Deeper"),
            (
                "raw/a.md".into(),
                Some(Anchor::Heading("Intro#Deeper".into()))
            )
        );
        let labelled: Citation = "[[raw/a.md|Report]]".parse().unwrap();
        assert_eq!(labelled.label, "Report");
    }

    #[test]
    fn display_round_trips() {
        for reference in [
            "raw/a.md",
            "raw/x/y.pdf",
            "raw/a.md#^message-m1",
            "raw/a.md#^blk",
            "raw/a.md#Intro#Deeper",
        ] {
            let citation: Citation = reference.parse().unwrap();
            assert_eq!(citation.to_string(), reference);
        }
    }

    #[test]
    fn malformed_references_are_rejected() {
        for bad in [
            "concepts/a.md",
            "raw/../a.md",
            "raw//a.md",
            "/raw/a.md",
            "raw\\a.md",
            "raw/a.md#",
            "raw/a.md#^",
            "raw/a.md#^message-",
            "raw/a.md#^message-bad id",
            "raw/a.md#^bad_block",
            "raw/a.md#A##B",
        ] {
            assert!(bad.parse::<Citation>().is_err(), "{bad}");
        }
    }
}
