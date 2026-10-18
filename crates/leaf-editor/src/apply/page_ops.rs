//! Page operations: creating pages, titles, summaries and headings, and
//! the lines they keep in `index.md`.

use super::{Mode, Planner, Staged, Step, Stop, carry_on};
use crate::batch::{NewPage, NewPassage, PassageId};
use crate::error::{Error, Result};
use crate::frontmatter::{Document, Frontmatter};
use crate::index_md::IndexDoc;
use crate::pages;
use crate::passages;
use crate::vocabulary::base_key;
use indexmap::IndexMap;
use leaf_common::parse;
use leaf_common::publication::entity_slug;
use std::fs;

/// Where and as what a new page is created.
struct Target {
    /// The page type's name, as written in `type:`.
    page_type: String,
    path: String,
    title: String,
    summary: String,
}

/// The checked passages of a new page, as Markdown.
struct Body {
    intro: Vec<String>,
    sections: Vec<(String, Vec<String>)>,
    /// The raw files they cite, in order of appearance.
    cited: Vec<String>,
}

impl Body {
    fn passages(&self) -> usize {
        self.intro.len() + self.sections.iter().map(|(_, p)| p.len()).sum::<usize>()
    }
}

/// Keys the editor sets on every new page itself.
const NEW_PAGE_MANAGED: [&str; 4] = ["title", "type", "created", "updated"];

impl Planner<'_> {
    /// `index.md`, loaded on first use.
    pub(super) fn index_doc(&mut self) -> Result<&mut IndexDoc> {
        if self.index.doc.is_none() {
            let path = self.editor.root.join("index.md");
            let text = fs::read_to_string(&path).map_err(|err| Error::io(&path, err))?;
            self.index.doc = Some(IndexDoc::parse(&text));
        }
        Ok(self.index.doc.as_mut().expect("loaded"))
    }

    /// The `index.md` section for pages of this folder.
    pub(super) fn index_section(&self, path: &str) -> Option<String> {
        let folder = path.split('/').next().unwrap_or_default();
        self.vocabulary
            .page_types
            .values()
            .find(|kind| kind.folder == folder)
            .map(|kind| kind.index_section.clone())
    }

    /// Write a page's `index.md` line (keeping its summary when none is
    /// given); the operation is rejected if `index.md` cannot take it.
    pub(super) fn index_line(
        &mut self,
        number: usize,
        path: &str,
        title: &str,
        summary: Option<&str>,
    ) -> Step {
        let Some(section) = self.index_section(path) else {
            return Err(self.reject(
                number,
                format!("`{path}` is not in a folder of a page type"),
            ));
        };
        let written = {
            let index = self.index_doc()?;
            index.entry(path).and_then(|entry| {
                let summary = summary
                    .map(str::to_string)
                    .or_else(|| entry.map(|entry| entry.summary))
                    .unwrap_or_default();
                index.set(path, &section, title, &summary)
            })
        };
        self.check(number, written)?;
        self.index.changed = true;
        Ok(())
    }

    // ---- create_page ----

    pub(super) fn create_page(&mut self, number: usize, spec: &NewPage) -> Step {
        let target = self.new_page_target(number, spec)?;
        // Passages and metadata are both checked, so one reply lists every problem.
        let body = self.new_page_body(number, spec, &target.title);
        let cited = body.as_ref().map(|b| b.cited.clone()).unwrap_or_default();
        let entries = self.new_page_metadata(number, &spec.metadata, &cited);
        let (body, entries) = (body?, entries?);
        self.stage_new_page(number, &target, &entries, &body)?;
        self.index_line(number, &target.path, &target.title, Some(&target.summary))
    }

    /// The page type, path, title and summary of a new page.
    fn new_page_target(&mut self, number: usize, spec: &NewPage) -> Step<Target> {
        let wanted = spec.page_type.trim().to_lowercase();
        let Some(kind) = self.vocabulary.page_types.get(&wanted).cloned() else {
            let types = self.vocabulary.page_types.keys().cloned().collect();
            return Err(self.reject_with(
                number,
                format!("`{}` is not a page type", spec.page_type),
                types,
            ));
        };
        let slug = spec.slug.trim();
        if !pages::is_slug(slug) {
            return Err(self.reject_with(
                number,
                format!("`{slug}` is not a slug: lowercase letters, digits and single hyphens"),
                vec![slug::slugify(slug)],
            ));
        }
        let path = format!("{}/{slug}.md", kind.folder);
        if self.pages.published.contains(&path) || self.editor.root.join(&path).exists() {
            return Err(self.reject(number, format!("`{path}` already exists")));
        }
        let title = self.check_heading(number, "title", &spec.title, 1)?;
        let summary = pages::check_summary(&spec.summary);
        let summary = self.check(number, summary)?;
        Ok(Target {
            page_type: wanted,
            path,
            title,
            summary,
        })
    }

    /// Checked passages of a new page as Markdown, and the raw files cited.
    /// Every bad passage and heading is reported before the step is rejected.
    fn new_page_body(&mut self, number: usize, spec: &NewPage, title: &str) -> Step<Body> {
        let mut rejected = false;
        let mut cited = Vec::new();
        let intro = self.new_passages(number, &spec.intro, &mut cited, &mut rejected)?;
        let mut headings = vec![title.to_lowercase()];
        let mut sections = Vec::new();
        for section in &spec.sections {
            let heading = self.check_heading(number, "heading", &section.heading, 2);
            let Some(heading) = carry_on(heading, &mut rejected)? else {
                continue;
            };
            if headings.contains(&heading.to_lowercase()) {
                self.error(
                    Some(number),
                    format!("the heading `{heading}` is used twice (or is the title)"),
                    Vec::new(),
                );
                rejected = true;
                continue;
            }
            headings.push(heading.to_lowercase());
            let passages =
                self.new_passages(number, &section.passages, &mut cited, &mut rejected)?;
            sections.push((heading, passages));
        }
        if rejected {
            return Err(Stop::Rejected);
        }
        Ok(Body {
            intro,
            sections,
            cited,
        })
    }

    /// Checked passages as Markdown; the raw files they cite are added to
    /// `cited`. A bad passage is reported and skipped (`rejected` notes it).
    fn new_passages(
        &mut self,
        number: usize,
        passages: &[NewPassage],
        cited: &mut Vec<String>,
        rejected: &mut bool,
    ) -> Step<Vec<String>> {
        let mut markdown = Vec::new();
        for passage in passages {
            let citations = self.citations(number, &passage.citations);
            let Some(citations) = carry_on(citations, rejected)? else {
                continue;
            };
            let text = self.text(number, &passage.text, &citations);
            let Some(text) = carry_on(text, rejected)? else {
                continue;
            };
            for citation in &citations {
                if !cited.contains(&citation.raw_path) {
                    cited.push(citation.raw_path.clone());
                }
            }
            markdown.push(passages::markdown(&text, &citations));
        }
        Ok(markdown)
    }

    /// The metadata of a new page: canonical keys, with `sources` completed
    /// from the citations, in the vocabulary's order.
    fn new_page_metadata(
        &mut self,
        number: usize,
        metadata: &IndexMap<String, Vec<String>>,
        cited: &[String],
    ) -> Step<Vec<(String, Vec<String>)>> {
        let mut rejected = false;
        let mut entries: Vec<(String, Vec<String>)> = Vec::new();
        for (key, values) in metadata {
            let key = self.key(number, key);
            let Some(key) = carry_on(key, &mut rejected)? else {
                continue;
            };
            if NEW_PAGE_MANAGED.contains(&base_key(&key)) {
                self.error(
                    Some(number),
                    format!("`{key}` is set by the editor (`type` and `title` are fields of create_page)"),
                    Vec::new(),
                );
                rejected = true;
                continue;
            }
            match entries.iter_mut().find(|(known, _)| *known == key) {
                Some((_, known)) => known.extend(values.iter().cloned()),
                None => entries.push((key, values.clone())),
            }
        }
        if rejected {
            return Err(Stop::Rejected);
        }
        if self.vocabulary.keys.contains_key("sources") {
            match entries.iter_mut().find(|(key, _)| key == "sources") {
                Some((_, sources)) => {
                    sources.extend(
                        cited
                            .iter()
                            .filter(|raw| !sources.contains(raw))
                            .cloned()
                            .collect::<Vec<_>>(),
                    );
                }
                None => entries.push(("sources".into(), cited.to_vec())),
            }
        }
        let order = self.new_page_key_order();
        entries.sort_by_key(|(key, _)| order.iter().position(|k| k == key).unwrap_or(order.len()));
        Ok(entries)
    }

    /// The keys a new page has first, in order: the vocabulary's required keys.
    fn new_page_key_order(&self) -> Vec<String> {
        self.vocabulary
            .required_keys
            .iter()
            .chain(&self.vocabulary.required_on_create)
            .cloned()
            .collect()
    }

    /// Stage the page with its frontmatter and body: metadata is set with
    /// the same checks as `set_metadata` (values, single, mirrored
    /// relationships), every required key must be there, and the page must
    /// read back with the passages given.
    fn stage_new_page(
        &mut self,
        number: usize,
        target: &Target,
        entries: &[(String, Vec<String>)],
        body: &Body,
    ) -> Step {
        let path = &target.path;
        if let Some(slug) = entity_slug(path)
            && entries.iter().any(|(key, values)| {
                base_key(key) == "relationship"
                    || (key == "related-people"
                        && values.iter().any(|v| v.trim().eq_ignore_ascii_case(slug)))
            })
        {
            self.pages.person_hints.insert(path.clone());
        }
        let today = self.editor.today().to_string();
        let mut frontmatter = Frontmatter::empty();
        frontmatter.set("title", vec![target.title.clone()], true);
        frontmatter.set("type", vec![target.page_type.clone()], true);
        frontmatter.set("created", vec![today.clone()], true);
        frontmatter.set("updated", vec![today], true);
        self.pages.loaded.insert(
            path.clone(),
            Staged {
                file: self.editor.root.join(path),
                original: String::new(),
                doc: Document {
                    frontmatter,
                    body: String::new(),
                },
            },
        );
        self.pages.published.insert(path.clone());
        self.pages.created.push(path.clone());
        self.refresh_staged()?;

        let mut rejected = false;
        for (key, values) in entries {
            let set = self.metadata(number, path, key, Some(values), Mode::Set);
            carry_on(set, &mut rejected)?;
            self.refresh_staged()?;
        }
        for key in self.new_page_key_order() {
            let given = entries.iter().any(|(entry, _)| *entry == key);
            if !NEW_PAGE_MANAGED.contains(&key.as_str())
                && !given
                && !self.doc(path).frontmatter.has(&key)
            {
                self.error(
                    Some(number),
                    format!("`{key}` is required on a new page; give it in `metadata`"),
                    Vec::new(),
                );
                rejected = true;
            }
        }
        if rejected {
            return Err(Stop::Rejected);
        }

        let doc = self.doc_mut(path);
        doc.body = pages::body(&target.title, &body.intro, &body.sections);
        let read = parse::page(&doc.render());
        let page = self.in_page(number, path, read)?;
        if page.passages.len() != body.passages() {
            return Err(self.reject(
                number,
                "the page would not read back with the passages given",
            ));
        }
        for passage in page.passages {
            self.outcome.passages.push(PassageId {
                op: number,
                id: format!("{path}#{}", passage.locator),
            });
        }
        Ok(())
    }

    // ---- titles, summaries, headings ----

    /// A checked title or heading (see [`pages::check_heading`]).
    fn check_heading(
        &mut self,
        number: usize,
        what: &str,
        text: &str,
        level: usize,
    ) -> Step<String> {
        let checked = pages::check_heading(what, text, level);
        self.check(number, checked)
    }

    pub(super) fn set_title(&mut self, number: usize, page: &str, title: &str) -> Step {
        let path = self.page(number, page)?;
        let title = self.check_heading(number, "title", title, 1)?;
        let old = self
            .doc(&path)
            .frontmatter
            .values("title")
            .into_iter()
            .next()
            .unwrap_or_default();
        if old == title {
            return Err(self.reject(number, format!("`{path}` is already titled `{title}`")));
        }
        let before = passages::locators(self.doc(&path));
        let before = self.in_page(number, &path, before)?;
        let doc = self.doc_mut(&path);
        doc.frontmatter.set("title", vec![title.clone()], true);
        // The top heading that carries the old title follows it.
        let rendered = doc.render();
        if let Ok(page) = parse::page(&rendered)
            && let Some(top) = page.headings.iter().find(|h| h.level == 1 && h.text == old)
        {
            let offset = rendered.len() - doc.body.len();
            let range = top.range.start - offset..top.range.end - offset;
            doc.body.replace_range(range, &format!("# {title}\n"));
        }
        self.report_changed_ids(number, &path, &before);
        self.index_line(number, &path, &title, None)
    }

    pub(super) fn add_heading(
        &mut self,
        number: usize,
        page: &str,
        heading: &str,
        level: Option<u8>,
        after: Option<&str>,
    ) -> Step {
        let path = self.page(number, page)?;
        let level = usize::from(level.unwrap_or(2));
        if !(2..=6).contains(&level) {
            return Err(self.reject(
                number,
                format!("a heading level is 2 to 6, not {level} (level 1 is the title)"),
            ));
        }
        let heading = self.check_heading(number, "heading", heading, level)?;
        self.require_new_heading(number, &path, &heading)?;
        let at = match after {
            None => self.doc(&path).body.len(),
            Some(after) => self.heading(number, &path, after)?.section_end,
        };
        let markdown = format!("{} {heading}", "#".repeat(level));
        passages::insert(self.doc_mut(&path), at, &markdown);
        Ok(())
    }

    pub(super) fn rename_heading(
        &mut self,
        number: usize,
        page: &str,
        heading: &str,
        to: &str,
    ) -> Step {
        let path = self.page(number, page)?;
        let found = self.heading(number, &path, heading)?;
        if found.level == 1 {
            return Err(self.reject(
                number,
                format!("`{}` is the page title; use set_title", found.text),
            ));
        }
        let to = self.check_heading(number, "heading", to, found.level)?;
        if to == found.text {
            return Err(self.reject(number, format!("the heading is already `{to}`")));
        }
        // A change of case alone is the same heading, not a second one.
        if to.to_lowercase() != found.text.to_lowercase() {
            self.require_new_heading(number, &path, &to)?;
        }
        let before = passages::locators(self.doc(&path));
        let before = self.in_page(number, &path, before)?;
        let markdown = format!("{} {to}", "#".repeat(found.level));
        passages::replace_heading(self.doc_mut(&path), found.range, &markdown);
        self.report_changed_ids(number, &path, &before);
        Ok(())
    }

    /// The page must not already have a heading (or title) named `heading`.
    fn require_new_heading(&mut self, number: usize, path: &str, heading: &str) -> Step {
        let exists = passages::has_heading(self.doc(path), heading);
        if self.in_page(number, path, exists)? {
            return Err(self.reject(
                number,
                format!("`{path}` already has a heading `{heading}`"),
            ));
        }
        Ok(())
    }

    /// Report the new ID of every passage whose ID differs from `before`
    /// (the page's locators, in order) after a change of headings.
    pub(super) fn report_changed_ids(&mut self, number: usize, path: &str, before: &[String]) {
        let Ok(after) = passages::locators(self.doc(path)) else {
            return;
        };
        for (old, new) in before.iter().zip(&after) {
            if old != new {
                self.outcome.passages.push(PassageId {
                    op: number,
                    id: format!("{path}#{new}"),
                });
            }
        }
    }

    pub(super) fn set_summary(&mut self, number: usize, page: &str, summary: &str) -> Step {
        let path = self.published_path(number, page)?;
        let summary = pages::check_summary(summary);
        let summary = self.check(number, summary)?;
        let title = match self.pages.loaded.get(&path) {
            Some(staged) => staged.doc.frontmatter.values("title").into_iter().next(),
            None => self
                .editor
                .index
                .pages()?
                .into_iter()
                .find(|p| p.path == path)
                .map(|p| p.title),
        };
        let entry = self.index_doc()?.entry(&path);
        let labelled = self.check(number, entry)?.and_then(|entry| entry.title);
        let title = labelled.or(title).unwrap_or_else(|| path.clone());
        self.index_line(number, &path, &title, Some(&summary))
    }
}
