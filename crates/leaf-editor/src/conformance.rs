//! How far an existing wiki is from its vocabulary (the baseline plus its
//! own extension), and what it would need to declare.
//!
//! The report never turns a wiki's contents into rules wholesale. It lists
//! **issues** a human should fix (missing required keys, a `type` that does
//! not match the folder, spellings to rewrite, references to pages that do
//! not exist, uncited passages, one-sided relationships) and proposes only
//! genuinely new **declarations** (the wiki's own tags, places, keys and raw
//! collections) as an [`Extension`] to review before saving it.
//! Reads the wiki; writes nothing to it.

use crate::check::Checker;
use crate::editor::Editor;
use crate::error::{Error, Result};
use crate::suggest::Lookup;
use crate::vocabulary::{Extension, KeySpec, RawCollection, Term, VERSION, ValueKind, base_key};
use leaf_common::parse;
use leaf_common::publication::{entity_path, entity_slug};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// The outcome of checking a wiki against its vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConformanceReport {
    /// Published pages checked.
    pub pages: usize,
    /// Problems to fix in pages; declarations cannot fix them.
    pub issues: Vec<Issue>,
    /// Facts worth knowing that are not problems (e.g. unrated pages).
    pub notes: Vec<String>,
    /// Declarations the wiki needs, to review and add to `_meta/vocabulary.yaml`.
    pub extension: Extension,
}

/// A problem in one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub path: String,
    pub problem: String,
}

impl ConformanceReport {
    /// Whether nothing needs fixing or declaring.
    pub fn is_conformant(&self) -> bool {
        self.issues.is_empty() && self.extension == empty_extension()
    }

    /// The report as readable text, ending with the proposed extension.
    pub fn to_text(&self) -> String {
        let mut text = format!("Checked {} published pages.\n", self.pages);
        if self.issues.is_empty() {
            text.push_str("No issues.\n");
        } else {
            text.push_str(&format!("\n{} issues:\n", self.issues.len()));
            for issue in &self.issues {
                text.push_str(&format!("- {}: {}\n", issue.path, issue.problem));
            }
        }
        if !self.notes.is_empty() {
            text.push_str("\nNotes:\n");
            for note in &self.notes {
                text.push_str(&format!("- {note}\n"));
            }
        }
        if self.extension != empty_extension() {
            text.push_str(
                "\nProposed additions to _meta/vocabulary.yaml (review before saving):\n\n",
            );
            text.push_str(&self.extension.to_yaml());
        }
        text
    }
}

fn empty_extension() -> Extension {
    Extension {
        version: VERSION,
        ..Extension::default()
    }
}

/// One published page as read for the checks.
pub(crate) struct Page {
    path: String,
    title: String,
    facets: BTreeMap<String, Vec<String>>,
    /// Headings with uncited passages, and how many.
    uncited: Vec<(String, usize)>,
}

impl Editor {
    /// Check the wiki against its vocabulary; see the module docs.
    pub fn conformance(&self) -> Result<ConformanceReport> {
        self.checker().conformance()
    }
}

/// A page's content as the checks read it; `Err` if the index would refuse it.
pub(crate) fn read_page(
    path: &str,
    content: &str,
) -> std::result::Result<(Page, Vec<String>), String> {
    let page = parse::page(content)?;
    let mut uncited: Vec<(String, usize)> = Vec::new();
    for passage in page.passages.iter().filter(|p| p.citations.is_empty()) {
        match uncited.last_mut() {
            Some((heading, count)) if *heading == passage.heading => *count += 1,
            _ => uncited.push((passage.heading.clone(), 1)),
        }
    }
    Ok((
        Page {
            path: path.to_string(),
            title: page.title,
            facets: page.facets,
            uncited,
        },
        page.warnings,
    ))
}

impl Checker<'_> {
    /// A page's problems under this vocabulary, as text, including keys and
    /// values it would need to declare. Used to tell problems a batch
    /// introduces from ones the page already had.
    pub(crate) fn page_problems(&self, page: &Page, warnings: &[String]) -> Result<Vec<String>> {
        let mut report = ConformanceReport {
            pages: 1,
            issues: Vec::new(),
            notes: Vec::new(),
            extension: empty_extension(),
        };
        let mut undeclared = BTreeMap::new();
        let mut edges = BTreeSet::new();
        self.check_page(page, &mut report, &mut undeclared, &mut edges)?;
        let mut problems: Vec<String> = report.issues.into_iter().map(|i| i.problem).collect();
        problems.extend(warnings.iter().cloned());
        problems.extend(
            undeclared
                .into_keys()
                .map(|key| format!("`{key}` is not a declared key")),
        );
        for (key, labels) in report.extension.labels {
            for label in labels.into_keys() {
                problems.push(format!(
                    "`{key}`: `{label}` is not declared (use declare_value)"
                ));
            }
        }
        for place in report.extension.places.into_keys() {
            problems.push(format!(
                "place `{place}` is not declared (use declare_place)"
            ));
        }
        Ok(problems)
    }

    /// Check the wiki against this vocabulary; see the module docs.
    pub(crate) fn conformance(&self) -> Result<ConformanceReport> {
        let mut report = ConformanceReport {
            pages: 0,
            issues: Vec::new(),
            notes: Vec::new(),
            extension: empty_extension(),
        };
        let pages = self.read_pages(&mut report)?;
        report.pages = pages.len();
        let paths: BTreeSet<&str> = pages.iter().map(|p| p.path.as_str()).collect();
        let mut undeclared: BTreeMap<String, (BTreeSet<String>, bool)> = BTreeMap::new();
        let mut edges: BTreeSet<(String, String, String)> = BTreeSet::new();
        let mut unrated = 0;
        for page in &pages {
            self.check_page(page, &mut report, &mut undeclared, &mut edges)?;
            if self
                .vocabulary()
                .required_on_create
                .iter()
                .any(|key| !page.facets.contains_key(key))
            {
                unrated += 1;
            }
        }
        if unrated > 0 {
            report.notes.push(format!(
                "{unrated} pages lack a key new pages must have ({}); they are accepted as they are, e.g. unrated",
                self.vocabulary().required_on_create.join(", ")
            ));
        }
        self.check_reciprocity(&pages, &edges, &mut report);
        self.declare_keys(undeclared, &paths, &mut report);
        self.declare_raw(&mut report)?;
        Ok(report)
    }

    fn read_pages(&self, report: &mut ConformanceReport) -> Result<Vec<Page>> {
        let root = self.wiki_root();
        let index_path = root.join("index.md");
        let index = fs::read_to_string(&index_path).map_err(|err| Error::io(&index_path, err))?;
        let (published, errors) = parse::published_pages(&index);
        for error in errors {
            issue(
                report,
                "index.md",
                format!("link `{}`: {}", error.path, error.message),
            );
        }
        let mut pages = Vec::new();
        for path in published {
            let parsed = parse::published_file(root, &path)
                .and_then(|file| fs::read_to_string(file).map_err(|err| err.to_string()))
                .and_then(|content| read_page(&path, &content));
            match parsed {
                Ok((page, warnings)) => {
                    for warning in warnings {
                        issue(report, &path, warning);
                    }
                    pages.push(page);
                }
                Err(message) => issue(report, &path, format!("cannot be read: {message}")),
            }
        }
        Ok(pages)
    }

    fn check_page(
        &self,
        page: &Page,
        report: &mut ConformanceReport,
        undeclared: &mut BTreeMap<String, (BTreeSet<String>, bool)>,
        edges: &mut BTreeSet<(String, String, String)>,
    ) -> Result<()> {
        self.check_page_shape(page, report);
        for (key, values) in &page.facets {
            self.check_facet(page, key, values, report, undeclared, edges)?;
        }
        self.check_page_rules(page, report);
        Ok(())
    }

    /// The keys every page has, and a `type` that matches its folder.
    fn check_page_shape(&self, page: &Page, report: &mut ConformanceReport) {
        let vocabulary = self.vocabulary();
        let path = page.path.as_str();
        for key in &vocabulary.required_keys {
            if key != "title" && !page.facets.contains_key(key) {
                issue(report, path, format!("missing required `{key}`"));
            }
        }
        if let Some(page_type) = page.facets.get("type").and_then(|v| v.first()) {
            let folder = path.split('/').next().unwrap_or_default();
            match vocabulary.page_types.get(page_type.to_lowercase().as_str()) {
                Some(declared) if declared.folder != folder => issue(
                    report,
                    path,
                    format!(
                        "`type: {page_type}` belongs in {}/, not {folder}/",
                        declared.folder
                    ),
                ),
                Some(_) => {}
                None => issue(
                    report,
                    path,
                    format!(
                        "`type: {page_type}` is not a page type ({})",
                        vocabulary
                            .page_types
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
            }
        }
    }

    /// One frontmatter key of a page and its values.
    fn check_facet(
        &self,
        page: &Page,
        key: &str,
        values: &[String],
        report: &mut ConformanceReport,
        undeclared: &mut BTreeMap<String, (BTreeSet<String>, bool)>,
        edges: &mut BTreeSet<(String, String, String)>,
    ) -> Result<()> {
        let vocabulary = self.vocabulary();
        let path = page.path.as_str();
        let key = match self.check_key(key)? {
            Lookup::Known { value } => value,
            Lookup::Rewritten { value, .. } => {
                issue(report, path, format!("write `{value}` instead of `{key}`"));
                value
            }
            Lookup::Unknown { .. } if key.contains('.') => {
                issue(
                    report,
                    path,
                    format!("`{key}` names no known entry of `{}`", base_key(key)),
                );
                return Ok(());
            }
            Lookup::Unknown { .. } => {
                let entry = undeclared.entry(key.to_string()).or_default();
                entry
                    .0
                    .extend(values.iter().map(|v| v.trim().to_lowercase()));
                entry.1 |= values.len() > 1;
                return Ok(());
            }
            Lookup::Invalid { reason, .. } => {
                issue(report, path, format!("`{key}`: {reason}"));
                return Ok(());
            }
        };
        let spec = vocabulary.key(&key).cloned();
        if let Some(spec) = &spec {
            if spec.single && values.len() > 1 && !key.contains('.') {
                issue(
                    report,
                    path,
                    format!("`{key}` takes one value, not {}", values.len()),
                );
            }
            if let Some(only_on) = &spec.only_on {
                let allowed = if only_on == "person" {
                    self.is_person_page(path)?
                } else {
                    page.facets.get("type").and_then(|v| v.first()) == Some(only_on)
                };
                if !allowed && !key.contains('.') {
                    issue(report, path, format!("`{key}` is only for {only_on} pages"));
                }
            }
        }
        // A map key lists entry names, checked as `key.<entry>` keys.
        if spec.as_ref().is_some_and(|s| s.map.is_some()) && !key.contains('.') {
            return Ok(());
        }
        for value in values {
            self.check_value_of(path, &key, value, report)?;
            if let (Some(subject), Some(other)) = (
                entity_slug(path).map(str::to_string),
                key.split_once('.').map(|(_, o)| o),
            ) {
                edges.insert((subject, other.to_string(), value.trim().to_lowercase()));
            }
        }
        Ok(())
    }

    /// What the page's content must satisfy: a contested page names its
    /// contradictions, and only introductions and summaries go uncited.
    fn check_page_rules(&self, page: &Page, report: &mut ConformanceReport) {
        let vocabulary = self.vocabulary();
        let path = page.path.as_str();
        let contested = page
            .facets
            .get("contested")
            .and_then(|values| values.first())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"));
        if contested && page.facets.get("contradictions").is_none_or(Vec::is_empty) {
            issue(
                report,
                path,
                "`contested: true` needs `contradictions`: the pages it disagrees with".into(),
            );
        }
        for (heading, count) in &page.uncited {
            let intro = vocabulary.uncited_intro && *heading == page.title;
            let summary = vocabulary.summary_headings.iter().any(|h| h == heading);
            if !intro && !summary {
                issue(
                    report,
                    path,
                    format!(
                        "{count} uncited passage(s) under \"{heading}\": cite a raw source, or list the heading under summary_headings if it is a summary"
                    ),
                );
            }
        }
    }

    fn check_value_of(
        &self,
        path: &str,
        key: &str,
        value: &str,
        report: &mut ConformanceReport,
    ) -> Result<()> {
        match self.check_value(key, value)? {
            Lookup::Known { .. } => {}
            Lookup::Rewritten { value: better, .. } => {
                issue(
                    report,
                    path,
                    format!("`{key}`: write `{better}` instead of `{value}`"),
                );
            }
            Lookup::Unknown { suggestions, .. } => {
                let kind = self.vocabulary().key(key).map(|spec| spec.values);
                let declared = value.trim().to_lowercase();
                match kind {
                    Some(ValueKind::Label) => {
                        report
                            .extension
                            .labels
                            .entry(base_key(key).to_string())
                            .or_default()
                            .entry(declared)
                            .or_default();
                    }
                    Some(ValueKind::Place) => {
                        report.extension.places.entry(declared).or_default();
                    }
                    _ => {
                        let hint = suggestions
                            .first()
                            .map(|s| format!(" (did you mean `{}`?)", s.value))
                            .unwrap_or_default();
                        issue(
                            report,
                            path,
                            format!("`{key}`: `{value}` refers to no page{hint}"),
                        );
                    }
                }
            }
            Lookup::Invalid { reason, .. } => {
                issue(report, path, format!("`{key}`: `{value}` is {reason}"));
            }
        }
        Ok(())
    }

    fn is_person_page(&self, path: &str) -> Result<bool> {
        let Some(slug) = entity_slug(path) else {
            return Ok(false);
        };
        Ok(self
            .staged
            .iter()
            .any(|page| page.person && page.path == path)
            || self
                .index()
                .people()?
                .iter()
                .any(|person| person.slug == slug))
    }

    /// Symmetric labels must hold in both directions when both people have pages.
    fn check_reciprocity(
        &self,
        pages: &[Page],
        edges: &BTreeSet<(String, String, String)>,
        report: &mut ConformanceReport,
    ) {
        let subjects: BTreeSet<String> = pages
            .iter()
            .filter_map(|p| entity_slug(&p.path).map(str::to_string))
            .collect();
        for (subject, other, label) in edges {
            let symmetric = self
                .vocabulary()
                .keys
                .values()
                .any(|spec| spec.symmetric.contains(label));
            let reverse = (other.clone(), subject.clone(), label.clone());
            if symmetric && subjects.contains(other) && !edges.contains(&reverse) {
                issue(
                    report,
                    &entity_path(other),
                    format!("`{label}` with {subject} is recorded on {subject}'s page only"),
                );
            }
        }
    }

    /// Propose declarations for keys the vocabulary does not know.
    fn declare_keys(
        &self,
        undeclared: BTreeMap<String, (BTreeSet<String>, bool)>,
        paths: &BTreeSet<&str>,
        report: &mut ConformanceReport,
    ) {
        for (key, (values, multiple)) in undeclared {
            let kind = infer_kind(&key, &values, paths);
            if kind == ValueKind::Label {
                report.extension.labels.insert(
                    key.clone(),
                    values
                        .iter()
                        .map(|v| (v.clone(), Term::default()))
                        .collect(),
                );
            }
            if kind == ValueKind::Place {
                for place in &values {
                    report.extension.places.entry(place.clone()).or_default();
                }
            }
            report.extension.keys.insert(
                key,
                KeySpec {
                    values: kind,
                    map: None,
                    single: !multiple && matches!(kind, ValueKind::Date | ValueKind::Boolean),
                    only_on: None,
                    symmetric: Vec::new(),
                    description: None,
                },
            );
        }
    }

    /// Raw collections and a manifest the vocabulary does not declare.
    fn declare_raw(&self, report: &mut ConformanceReport) -> Result<()> {
        let root = self.wiki_root();
        if let Ok(entries) = fs::read_dir(root.join("raw")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() && !self.vocabulary().raw_collections.contains_key(&name) {
                    report.extension.raw_collections.insert(
                        name.clone(),
                        RawCollection {
                            addable: true,
                            ..RawCollection::default()
                        },
                    );
                    report.notes.push(format!(
                        "raw/{name}/ is not a declared collection; proposed with no rules (set `addable: false` for an import that must not grow)"
                    ));
                }
            }
        }
        if self.vocabulary().raw_manifest.is_none()
            && let Some(manifest) = find_manifest(root)
        {
            report
                .notes
                .push(format!("found a raw checksum manifest: {manifest}"));
            report.extension.raw_manifest = Some(manifest);
        }
        Ok(())
    }
}

fn issue(report: &mut ConformanceReport, path: &str, problem: String) {
    report.issues.push(Issue {
        path: path.to_string(),
        problem,
    });
}

/// The kind of an undeclared key: from the built-in dictionary when the
/// key's last word names one (`associated-organizations` holds organizations),
/// else from its values (dates, booleans, raw paths, page paths), else labels.
fn infer_kind(key: &str, values: &BTreeSet<String>, pages: &BTreeSet<&str>) -> ValueKind {
    let dictionary = leaf_common::dictionary::dictionary();
    let last_word = key.rsplit('-').next().unwrap_or(key);
    let named = dictionary
        .canonical_key(last_word)
        .and_then(|canonical| dictionary.keys.get(canonical))
        .and_then(|entry| kind_named(&entry.values));
    if let Some(kind) = named.filter(|kind| !matches!(kind, ValueKind::Label | ValueKind::Text)) {
        return kind;
    }
    let all = |test: &dyn Fn(&str) -> bool| !values.is_empty() && values.iter().all(|v| test(v));
    if all(&|v| v.parse::<jiff::civil::Date>().is_ok()) {
        ValueKind::Date
    } else if all(&|v| matches!(v, "true" | "false")) {
        ValueKind::Boolean
    } else if all(&|v| v.starts_with("raw/")) {
        ValueKind::Raw
    } else if all(&|v| pages.contains(v) || pages.contains(format!("{v}.md").as_str())) {
        ValueKind::Page
    } else {
        named.unwrap_or(ValueKind::Label)
    }
}

fn kind_named(name: &str) -> Option<ValueKind> {
    serde_saphyr::from_str(name).ok()
}

/// A JSON file under `_meta/` whose `files` entries all have `path` and
/// `sha256`: the most complete one.
fn find_manifest(root: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    let mut pending = vec![root.join("_meta")];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json")
                && let Some(covered) = manifest_entries(&path)
                && best.as_ref().is_none_or(|(n, _)| covered > *n)
            {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                best = Some((covered, relative.to_string_lossy().into_owned()));
            }
        }
    }
    best.map(|(_, path)| path)
}

fn manifest_entries(path: &Path) -> Option<usize> {
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let files = json.get("files")?.as_array()?;
    let valid = files
        .iter()
        .filter(|f| {
            f.get("path").is_some_and(|p| p.is_string())
                && f.get("sha256").is_some_and(|s| s.is_string())
        })
        .count();
    (valid > 0 && valid == files.len()).then_some(valid)
}
