//! Creating a new wiki that conforms to the baseline: the folders, a
//! `SCHEMA.md` written from the baseline vocabulary, `index.md`, `log.md`
//! and `_meta/vocabulary.yaml`. Nothing is overwritten: the target must be
//! empty (or not exist yet).

use crate::error::{Error, Result};
use crate::vocabulary::{Extension, Term, VERSION, VOCABULARY_PATH, Vocabulary};
use std::fs;
use std::path::Path;

/// The order `index.md` lists page types in, when the baseline has them.
const SECTION_ORDER: [&str; 4] = ["entity", "concept", "comparison", "query"];

/// What a new wiki is about and starts with.
#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    /// One line on what the wiki covers, written into `SCHEMA.md` and the
    /// first `log.md` entry.
    pub domain: Option<String>,
    /// The wiki's starting tag taxonomy (declared in `_meta/vocabulary.yaml`).
    pub tags: Vec<String>,
    /// The date written into the files; today when `None`.
    pub today: Option<jiff::civil::Date>,
}

/// Create the wiki at `root`; returns the files written, relative to it.
pub fn init_wiki(root: &Path, options: &InitOptions) -> Result<Vec<String>> {
    match fs::read_dir(root) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(Error::Init {
                    path: root.to_path_buf(),
                    message: "the folder is not empty; a new wiki needs an empty one".into(),
                });
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::io(root, err)),
    }
    let baseline = Vocabulary::baseline();
    let today = options
        .today
        .unwrap_or_else(|| jiff::Zoned::now().date())
        .to_string();
    let domain = options
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let tags = tags(options)?;

    let mut folders: Vec<String> = baseline
        .page_types
        .values()
        .map(|t| t.folder.clone())
        .collect();
    folders.extend(baseline.raw_collections.keys().map(|c| format!("raw/{c}")));
    folders.push("_meta".into());
    for folder in &folders {
        let path = root.join(folder);
        fs::create_dir_all(&path).map_err(|err| Error::io(&path, err))?;
    }

    let mut extension = Extension {
        version: VERSION,
        ..Extension::default()
    };
    if !tags.is_empty() {
        let labels = tags
            .iter()
            .map(|tag| (tag.clone(), Term::default()))
            .collect();
        extension.labels.insert("tags".into(), labels);
    }
    let vocabulary = format!(
        "# This wiki's additions to Leaf's baseline vocabulary.\n\
         # Edited by leaf-editor; see docs/EDITOR-SPEC.md.\n{}",
        extension.to_yaml()
    );
    let files = [
        ("SCHEMA.md", schema(&baseline, domain, &tags)),
        ("index.md", index(&baseline, &today)),
        ("log.md", log(domain, &today)),
        (VOCABULARY_PATH, vocabulary),
    ];
    let mut written = Vec::new();
    for (name, content) in files {
        let path = root.join(name);
        fs::write(&path, content).map_err(|err| Error::io(&path, err))?;
        written.push(name.to_string());
    }
    Ok(written)
}

/// The starting tags as slugs, without duplicates.
fn tags(options: &InitOptions) -> Result<Vec<String>> {
    let mut tags: Vec<String> = Vec::new();
    for tag in &options.tags {
        let slug = slug::slugify(tag);
        if slug.is_empty() {
            return Err(Error::Init {
                path: Default::default(),
                message: format!("`{tag}` is not a usable tag"),
            });
        }
        if !tags.contains(&slug) {
            tags.push(slug);
        }
    }
    Ok(tags)
}

/// The page types in the order `index.md` lists them.
fn ordered_types(baseline: &Vocabulary) -> Vec<(&String, &crate::vocabulary::PageType)> {
    let mut types: Vec<_> = baseline.page_types.iter().collect();
    types.sort_by_key(|(name, _)| {
        SECTION_ORDER
            .iter()
            .position(|known| known == name)
            .unwrap_or(SECTION_ORDER.len())
    });
    types
}

fn index(baseline: &Vocabulary, today: &str) -> String {
    let mut text = format!(
        "# Wiki Index\n\n> Content catalog: every page, with a one-line summary, by type.\n\
         > Last updated: {today} | Total pages: 0\n"
    );
    for (_, kind) in ordered_types(baseline) {
        text.push_str(&format!("\n## {}\n", kind.index_section));
    }
    text
}

fn log(domain: Option<&str>, today: &str) -> String {
    let mut text = format!("# Wiki Log\n\n## [{today}] create | Wiki initialized\n");
    if let Some(domain) = domain {
        text.push_str(&format!("- Domain: {domain}\n"));
    }
    text
}

fn schema(baseline: &Vocabulary, domain: Option<&str>, tags: &[String]) -> String {
    let mut text = String::from("# Wiki Schema\n\n## Domain\n\n");
    text.push_str(domain.unwrap_or("Describe what this wiki covers."));
    text.push_str("\n\n## Structure\n\n");
    text.push_str(
        "- `raw/`: immutable sources. Files are only ever added (never edited or removed), \
         under one of the collections below.\n",
    );
    for (name, rules) in &baseline.raw_collections {
        let media = if rules.media.is_empty() {
            "any type".to_string()
        } else {
            rules.media.join(", ")
        };
        text.push_str(&format!("  - `raw/{name}/`: {media}\n"));
    }
    for (name, kind) in ordered_types(baseline) {
        text.push_str(&format!("- `{}/`: `type: {name}` pages\n", kind.folder));
    }
    text.push_str(
        "- `index.md`: one line per page, by type. `log.md`: one entry per change. \
         Both are kept by Leaf's editor.\n\
         - `_meta/vocabulary.yaml`: this wiki's additions to Leaf's baseline vocabulary (tags, places, keys).\n",
    );
    text.push_str(
        "\n## Page rules\n\nEnforced by Leaf's editor: a batch that breaks one is rejected.\n\n",
    );
    text.push_str(&format!(
        "- Frontmatter on every page: {}.\n",
        baseline
            .required_keys
            .iter()
            .map(|k| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    text.push_str(&format!(
        "- A new page also states {} (`high`, `medium`, `low`).\n",
        baseline
            .required_on_create
            .iter()
            .map(|k| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    text.push_str(
        "- Every passage cites a raw source (`[[raw/<collection>/<file>#^<anchor>]]`), \
         except the introduction under the page title and passages under a summary heading.\n\
         - A page links to at least two other pages (`[[folder/page|label]]`), and every link must resolve.\n\
         - Tags must be declared in `_meta/vocabulary.yaml` before a page uses them.\n",
    );
    text.push_str(&format!(
        "- A passage is one line of plain text, at most {} characters; a batch has at most {} operations.\n",
        baseline.limits.passage_chars, baseline.limits.ops_per_batch
    ));
    text.push_str("\n## Tag taxonomy\n\n");
    if tags.is_empty() {
        text.push_str("None yet: declare a tag (`declare_value`) before using it.\n");
    } else {
        for tag in tags {
            text.push_str(&format!("- `{tag}`\n"));
        }
    }
    text.push_str(
        "\n## Editing\n\nChange pages only through Leaf's editor (`check`, then `apply`, a batch of \
         operations): never by hand, so the conventions above hold.\n",
    );
    text
}
