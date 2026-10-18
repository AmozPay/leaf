//! Archiving: moving a page to `_archive/` and back, and making sure no
//! page is left referring to an archived one.

use super::{Planner, Staged, Step};
use crate::error::{Error, Result};
use crate::frontmatter::Document;
use crate::pages;
use crate::vocabulary::{ValueKind, base_key};
use leaf_common::parse;
use leaf_common::publication::{entity_slug, page_path};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

/// A page moved to `_archive/`.
pub(super) struct Moved {
    pub op: usize,
    pub from: String,
    pub from_file: PathBuf,
    pub to: String,
    pub reason: String,
    pub doc: Document,
}

impl Planner<'_> {
    pub(super) fn archive_page(&mut self, number: usize, page: &str, reason: &str) -> Step {
        let Ok(reason) = pages::check_summary(reason) else {
            return Err(self.reject(number, "give a one-line `reason` for archiving a page"));
        };
        let path = self.page(number, page)?;
        if self.pages.created.contains(&path) {
            return Err(self.reject(
                number,
                format!("`{path}` is created in this batch; leave it out instead"),
            ));
        }
        let to = format!("_archive/{path}");
        if self.editor.root.join(&to).exists() || self.archive.moved.iter().any(|m| m.to == to) {
            return Err(self.reject(number, format!("`{to}` already exists")));
        }
        // The line leaves index.md; keep its summary so a restore can bring it back.
        let entry = self.index_doc()?.entry(&path);
        let summary = self
            .check(number, entry)?
            .map(|e| e.summary)
            .unwrap_or_default();
        let removed = self.index_doc()?.remove(&path);
        self.check(number, removed)?;
        self.index.changed = true;
        let mut staged = self.pages.loaded.remove(&path).expect("a staged page");
        let today = self.editor.today().to_string();
        let frontmatter = &mut staged.doc.frontmatter;
        frontmatter.set("updated", vec![today.clone()], true);
        frontmatter.set("archived", vec![today], true);
        frontmatter.set("archived-reason", vec![reason.clone()], true);
        if !summary.is_empty() {
            frontmatter.set("archived-summary", vec![summary], true);
        }
        self.pages.published.remove(&path);
        self.pages.gone.push(path.clone());
        self.archive.moved.push(Moved {
            op: number,
            from: path,
            from_file: staged.file,
            to,
            reason,
            doc: staged.doc,
        });
        Ok(())
    }

    pub(super) fn restore_page(
        &mut self,
        number: usize,
        page: &str,
        summary: Option<&str>,
    ) -> Step {
        let given = page_path(page);
        let archived = if given.starts_with("_archive/") {
            given
        } else {
            format!("_archive/{given}")
        };
        let dest = archived.trim_start_matches("_archive/").to_string();
        let file = self.editor.root.join(&archived);
        if !file.is_file() || self.archive.moved.iter().any(|m| m.to == archived) {
            return Err(self.reject(
                number,
                format!("`{archived}` is not an archived page (archived in an earlier batch)"),
            ));
        }
        let folder = dest.split('/').next().unwrap_or_default();
        let in_a_folder = self
            .vocabulary
            .page_types
            .values()
            .any(|kind| kind.folder == folder);
        if !in_a_folder || dest.matches('/').count() != 1 {
            return Err(self.reject(
                number,
                format!("`{dest}` is not in a folder of a page type"),
            ));
        }
        if self.pages.published.contains(&dest) || self.editor.root.join(&dest).exists() {
            return Err(self.reject(
                number,
                format!("`{dest}` already exists; it must be free to restore"),
            ));
        }
        let content = fs::read_to_string(&file).map_err(|err| Error::io(&file, err))?;
        let parsed = Document::parse(&content);
        let mut doc = self.check(
            number,
            parsed.map_err(|message| format!("`{archived}`: {message}")),
        )?;
        let kept = doc
            .frontmatter
            .values("archived-summary")
            .into_iter()
            .next();
        let given = summary.map(pages::check_summary).transpose();
        let Some(summary) = self.check(number, given)?.or(kept) else {
            return Err(self.reject(
                number,
                format!("`{archived}` kept no summary for index.md; give one in `summary`"),
            ));
        };
        for key in ["archived", "archived-reason", "archived-summary"] {
            doc.frontmatter.remove(key);
        }
        doc.frontmatter
            .set("updated", vec![self.editor.today().to_string()], true);
        let title = doc
            .frontmatter
            .values("title")
            .into_iter()
            .next()
            .unwrap_or_else(|| dest.clone());
        self.pages.loaded.insert(
            dest.clone(),
            Staged {
                file: self.editor.root.join(&dest),
                original: String::new(),
                doc,
            },
        );
        self.pages.published.insert(dest.clone());
        self.pages.fresh.insert(dest.clone());
        self.archive.restored.push((archived, dest.clone(), file));
        self.index_line(number, &dest, &title, Some(&summary))
    }

    /// An archived page must not leave links or metadata pointing at it:
    /// every page that is still published is read as the batch leaves it.
    pub(super) fn check_archived_references(&mut self) -> Result<()> {
        if self.archive.moved.is_empty() {
            return Ok(());
        }
        let mut found: BTreeMap<usize, Vec<(String, String)>> = BTreeMap::new();
        for path in self.pages.published.clone() {
            let content = match self.pages.loaded.get(&path) {
                Some(staged) => staged.doc.render(),
                None => {
                    let file = parse::published_file(&self.editor.root, &path)
                        .map_err(|message| Error::io(&path, std::io::Error::other(message)))?;
                    fs::read_to_string(&file).map_err(|err| Error::io(&file, err))?
                }
            };
            let Ok(page) = parse::page(&content) else {
                continue;
            };
            let body = super::staged_body(&content);
            let (links, _) = parse::published_pages(&body);
            for (position, moved) in self.archive.moved.iter().enumerate() {
                for what in self.references(&page.facets, &links, &moved.from) {
                    found
                        .entry(position)
                        .or_default()
                        .push((path.clone(), what));
                }
            }
        }
        for (position, references) in found {
            let moved = &self.archive.moved[position];
            let list: Vec<String> = references
                .iter()
                .map(|(page, what)| format!("`{page}` ({what})"))
                .collect();
            let pages: Vec<String> = references.into_iter().map(|(page, _)| page).collect();
            let (op, from) = (moved.op, moved.from.clone());
            self.error(
                Some(op),
                format!(
                    "`{from}` is still referred to by {}; remove those references in the same batch",
                    list.join(", ")
                ),
                pages,
            );
        }
        Ok(())
    }

    /// How a page refers to `target`: links, and metadata that names it.
    fn references(
        &self,
        facets: &BTreeMap<String, Vec<String>>,
        links: &BTreeSet<String>,
        target: &str,
    ) -> Vec<String> {
        let mut what = Vec::new();
        if links.contains(target) {
            what.push("a link".to_string());
        }
        let slug = entity_slug(target);
        let stem = target.trim_end_matches(".md");
        for (key, values) in facets {
            let Some(spec) = self.vocabulary.key(base_key(key)) else {
                continue;
            };
            let names = |value: &String| {
                let value = value.trim();
                match spec.values {
                    ValueKind::Person | ValueKind::Organization | ValueKind::Entity => {
                        slug.is_some_and(|slug| value.eq_ignore_ascii_case(slug))
                    }
                    ValueKind::Page => value.trim_end_matches(".md") == stem,
                    _ => false,
                }
            };
            let entry_names_it = key.split_once('.').is_some_and(|(_, entry)| {
                spec.map.is_some() && slug.is_some_and(|slug| entry.eq_ignore_ascii_case(slug))
            });
            if entry_names_it || values.iter().any(names) {
                what.push(format!("`{key}`"));
            }
        }
        what
    }
}
