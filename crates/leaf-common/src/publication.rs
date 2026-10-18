//! The `index.md` allowlist and safe resolution of published page paths.

use crate::markdown;
use crate::model::SyncIssue;
use pulldown_cmark::{Event, Tag};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Directories whose pages may be published through `index.md`.
pub const PUBLISHED_DIRS: [&str; 4] = ["entities", "concepts", "comparisons", "queries"];

/// The directory of entity pages (people, organizations, places, …).
pub const ENTITIES_DIR: &str = "entities/";

/// The slug of an entity page path: `entities/alice.md` → `alice`. `None`
/// for any other path, including pages in subdirectories of `entities/`.
pub fn entity_slug(path: &str) -> Option<&str> {
    path.strip_prefix(ENTITIES_DIR)
        .and_then(|rest| rest.strip_suffix(".md"))
        .filter(|slug| !slug.contains('/'))
}

/// The page path of an entity slug: `alice` → `entities/alice.md`.
pub fn entity_path(slug: &str) -> String {
    format!("{ENTITIES_DIR}{slug}.md")
}

/// The path of a page named the way an agent writes it (`concepts/x`,
/// `./concepts/x.md`): relative, ending in `.md`.
pub fn page_path(page: &str) -> String {
    let page = page.trim().trim_start_matches("./");
    if page.ends_with(".md") {
        page.to_string()
    } else {
        format!("{page}.md")
    }
}

/// Published pages listed in `index.md`, plus links that could not be accepted.
#[derive(Debug, Default)]
pub struct Publication {
    /// Wiki-relative page paths ending in `.md`, deduplicated.
    pub pages: BTreeSet<String>,
    pub errors: Vec<SyncIssue>,
}

/// Collect `[[dir/page]]`, `[[dir/page.md|Label]]` and `[Label](dir/page.md)`
/// links into a published directory. Other links (such as `[[SCHEMA]]`) are
/// not pages and are ignored, as are links in code, frontmatter and images.
pub fn published_pages(index: &str) -> Publication {
    let mut publication = Publication::default();
    for (event, _) in markdown::events(index) {
        let Event::Start(Tag::Link { dest_url, .. }) = event else {
            continue;
        };
        let target = dest_url.split('#').next().unwrap_or_default().trim();
        let first = target.split(['/', '\\']).next().unwrap_or_default();
        if !PUBLISHED_DIRS.contains(&first) {
            continue;
        }
        match normalize_page(target) {
            Ok(Some(path)) => {
                publication.pages.insert(path);
            }
            Ok(None) => {}
            Err(message) => publication.errors.push(SyncIssue {
                path: target.to_string(),
                message,
            }),
        }
    }
    publication
}

/// `Ok(None)` for links to non-Markdown files (e.g. images).
fn normalize_page(target: &str) -> Result<Option<String>, String> {
    if !is_plain_relative(target) {
        return Err("index link is not a plain relative page path".into());
    }
    let last = target.rsplit('/').next().unwrap_or_default();
    match last.rsplit_once('.') {
        Some((_, "md")) => Ok(Some(target.to_string())),
        Some((_, ext)) if !ext.contains(' ') && ext.len() <= 4 => Ok(None),
        _ => Ok(Some(format!("{target}.md"))),
    }
}

/// Every segment is a normal name: no `.`/`..`, empty segments, backslashes,
/// or control characters.
pub fn is_plain_relative(path: &str) -> bool {
    !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Resolve a published page, rejecting symlinks that leave its published
/// directory (or land in `raw/`).
pub fn resolve_page(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let first = relative.split('/').next().unwrap_or_default();
    let allowed = fs::canonicalize(root.join(first))
        .map_err(|err| format!("published directory unavailable: {err}"))?;
    let canonical =
        fs::canonicalize(root.join(relative)).map_err(|err| format!("cannot open page: {err}"))?;
    let in_raw = fs::canonicalize(root.join("raw")).is_ok_and(|raw| canonical.starts_with(raw));
    if !canonical.starts_with(&allowed) || in_raw {
        return Err(format!("page resolves outside {first}/"));
    }
    if !canonical.is_file() {
        return Err("page is not a regular file".into());
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_slugs_and_paths_round_trip() {
        assert_eq!(entity_slug("entities/alice.md"), Some("alice"));
        assert_eq!(entity_path("alice"), "entities/alice.md");
        assert_eq!(
            entity_slug(&entity_path("chloe-dubois")),
            Some("chloe-dubois")
        );
        for other in [
            "concepts/alice.md",
            "entities/alice.txt",
            "entities/sub/alice.md",
            "entities/.md/",
        ] {
            assert_eq!(entity_slug(other), None, "{other}");
        }
    }

    #[test]
    fn page_paths_end_in_md() {
        assert_eq!(page_path(" ./concepts/garden "), "concepts/garden.md");
        assert_eq!(page_path("concepts/garden.md"), "concepts/garden.md");
    }

    #[test]
    fn plain_relative_paths_have_no_odd_segments() {
        assert!(is_plain_relative("_meta/manifest.json"));
        for bad in ["", "/abs.json", "a//b", "./a", "a/../b", "a\\b", "a\u{0}b"] {
            assert!(!is_plain_relative(bad), "{bad:?}");
        }
    }
}
