//! Checking and applying batches (see `docs/EDITOR-SPEC.md`).
//!
//! A batch is planned entirely in memory: operations run in order against
//! the vocabulary as the batch changes it, then every changed page is
//! re-read as the index would read it and checked. A problem the batch
//! introduces rejects it; one the page already had is a warning, so a wiki
//! with old problems stays editable. Only an acceptable batch is written,
//! all files or none, followed by a `log.md` entry and an index sync.

use crate::batch::{Batch, BatchOutcome, Op, OpError, Rewrite};
use crate::check::{Checker, StagedPage};
use crate::commit::{self, Write};
use crate::conformance;
use crate::editor::Editor;
use crate::error::{Error, Result};
use crate::frontmatter::Document;
use crate::pages;
use crate::passages;

use crate::suggest::Lookup;
use crate::vocabulary::{Extension, KeySpec, VOCABULARY_PATH, ValueKind, Vocabulary, base_key};
use leaf_common::parse;
use leaf_common::publication::{entity_path, entity_slug, page_path};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

mod archive_ops;
mod page_ops;
mod passage_ops;
mod raw_ops;
mod state;
mod vocab_ops;

/// Pages longer than this many lines get a warning to split them.
const LONG_PAGE_LINES: usize = 200;

/// Keys the editor sets itself.
const MANAGED: [&str; 6] = [
    "created",
    "updated",
    "type",
    "archived",
    "archived-reason",
    "archived-summary",
];

impl Editor {
    /// Check a batch without writing anything: the outcome says whether it
    /// would be applied, and why not.
    pub fn check(&self, batch: &Batch) -> Result<BatchOutcome> {
        Ok(Planner::plan(self, batch)?.outcome)
    }

    /// Apply a batch: all of it or none of it, then sync the index.
    pub fn apply(&mut self, batch: &Batch) -> Result<BatchOutcome> {
        let plan = Planner::plan(self, batch)?;
        let mut outcome = plan.outcome;
        if !outcome.is_acceptable() {
            return Ok(outcome);
        }
        commit::commit(&self.root, &plan.writes)?;
        self.vocabulary = plan.vocabulary;
        outcome.applied = true;
        outcome.sync = Some(self.index.sync()?);
        Ok(outcome)
    }
}

struct Plan {
    outcome: BatchOutcome,
    writes: Vec<Write>,
    vocabulary: Vocabulary,
}

/// A page being changed.
struct Staged {
    file: PathBuf,
    original: String,
    doc: Document,
}

/// Why a planner step stopped.
enum Stop {
    /// The operation is rejected: its errors are recorded in the outcome and
    /// the rest of the batch is still checked, to report every problem.
    Rejected,
    /// The wiki could not be read: the whole check fails.
    Failed(Error),
}

impl From<Error> for Stop {
    fn from(err: Error) -> Self {
        Self::Failed(err)
    }
}

impl From<leaf_viewer::Error> for Stop {
    fn from(err: leaf_viewer::Error) -> Self {
        Self::Failed(err.into())
    }
}

/// The result of a planner step.
type Step<T = ()> = std::result::Result<T, Stop>;

/// Go on after a rejected step, to find more problems: `rejected` notes that
/// one was; only a failure to read the wiki stops.
fn carry_on<T>(step: Step<T>, rejected: &mut bool) -> Step<Option<T>> {
    match step {
        Ok(value) => Ok(Some(value)),
        Err(Stop::Rejected) => {
            *rejected = true;
            Ok(None)
        }
        Err(failed) => Err(failed),
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Set,
    Add,
    Remove,
}

/// A batch being planned: the operations run against this state, in
/// memory, until the batch is accepted or rejected as a whole.
struct Planner<'a> {
    editor: &'a Editor,
    /// The vocabulary as the batch changes it.
    vocabulary: Vocabulary,
    declarations: state::Declarations,
    pages: state::PageSet,
    archive: state::Archive,
    raw: state::RawFiles,
    index: state::IndexState,
    /// Removed passages with their reasons, for `log.md`.
    removed_passages: Vec<String>,
    outcome: BatchOutcome,
}

impl<'a> Planner<'a> {
    fn plan(editor: &'a Editor, batch: &Batch) -> Result<Plan> {
        let published = editor.index.pages()?.into_iter().map(|p| p.path).collect();
        let mut planner = Planner {
            editor,
            vocabulary: editor.vocabulary.clone(),
            declarations: state::Declarations::new(Extension::load(&editor.root)?),
            pages: state::PageSet::new(published),
            archive: state::Archive::default(),
            raw: state::RawFiles::default(),
            index: state::IndexState::default(),
            removed_passages: Vec::new(),
            outcome: BatchOutcome::default(),
        };
        planner.check_batch(batch);
        if planner.outcome.errors.is_empty() {
            for (number, op) in batch.ops.iter().enumerate() {
                match planner.op(number, op) {
                    Ok(()) | Err(Stop::Rejected) => {}
                    Err(Stop::Failed(err)) => return Err(err),
                }
                planner.refresh_staged()?;
            }
        }
        let writes = if planner.outcome.errors.is_empty() {
            planner.finish(batch)?
        } else {
            Vec::new()
        };
        if !planner.outcome.errors.is_empty() {
            planner.outcome.changed.clear();
            planner.outcome.removed.clear();
        }
        Ok(Plan {
            outcome: planner.outcome,
            writes,
            vocabulary: planner.vocabulary,
        })
    }

    fn checker(&self) -> Checker<'_> {
        Checker {
            vocabulary: &self.vocabulary,
            index: &self.editor.index,
            root: &self.editor.root,
            staged: &self.pages.checks,
            gone: &self.pages.gone,
            staged_raw: &self.raw.paths,
        }
    }

    /// Describe every page the batch has loaded as the checks need it, so
    /// later operations see pages created or retitled earlier in the batch.
    fn refresh_staged(&mut self) -> Result<()> {
        let named: BTreeSet<String> = self
            .pages
            .loaded
            .values()
            .flat_map(|staged| staged.doc.frontmatter.values("related-people"))
            .map(|slug| slug.to_lowercase())
            .collect();
        let known: BTreeSet<String> = self
            .editor
            .index
            .people()?
            .into_iter()
            .map(|person| person.slug)
            .collect();
        self.pages.checks = self
            .pages
            .loaded
            .iter()
            .map(|(path, staged)| {
                let frontmatter = &staged.doc.frontmatter;
                StagedPage {
                    path: path.clone(),
                    title: frontmatter
                        .values("title")
                        .into_iter()
                        .next()
                        .unwrap_or_default(),
                    aliases: frontmatter.values("aliases"),
                    person: entity_slug(path).is_some_and(|slug| {
                        self.pages.person_hints.contains(path)
                            || !frontmatter.values("relationship").is_empty()
                            || named.contains(slug)
                            || known.contains(slug)
                    }),
                }
            })
            .collect();
        Ok(())
    }

    /// Record an error without stopping: for problems found while going on
    /// to find more (and the batch-level ones, which have no operation).
    fn error(&mut self, op: Option<usize>, message: String, suggestions: Vec<String>) {
        self.outcome.errors.push(OpError {
            op,
            message,
            suggestions,
        });
    }

    /// Record an error for operation `number` and stop the operation.
    fn reject(&mut self, number: usize, message: impl Into<String>) -> Stop {
        self.reject_with(number, message, Vec::new())
    }

    /// [`Self::reject`], with suggestions of what to use instead.
    fn reject_with(
        &mut self,
        number: usize,
        message: impl Into<String>,
        suggestions: Vec<String>,
    ) -> Stop {
        self.error(Some(number), message.into(), suggestions);
        Stop::Rejected
    }

    /// The result of reading a staged page (`passages::*`), or the operation
    /// rejected with the page's name and the message.
    fn in_page<T>(
        &mut self,
        number: usize,
        path: &str,
        read: std::result::Result<T, String>,
    ) -> Step<T> {
        read.map_err(|message| self.reject(number, format!("{path}: {message}")))
    }

    /// A staged page (loaded by [`Self::page`]).
    fn doc(&self, path: &str) -> &Document {
        &self.pages.loaded[path].doc
    }

    /// A staged page, to change.
    fn doc_mut(&mut self, path: &str) -> &mut Document {
        &mut self.pages.loaded.get_mut(path).expect("a staged page").doc
    }

    /// The one heading of a staged page named `name`; the operation is
    /// rejected, listing the page's headings, if there is not exactly one.
    fn heading(&mut self, number: usize, path: &str, name: &str) -> Step<passages::HeadingAt> {
        let found = passages::heading(self.doc(path), name);
        match self.in_page(number, path, found)? {
            Ok(heading) => Ok(heading),
            Err(headings) => Err(self.reject_with(
                number,
                format!("`{path}` has no single heading `{name}`"),
                headings,
            )),
        }
    }

    /// A checked value, or the operation rejected with the check's message.
    fn check<T>(&mut self, number: usize, checked: std::result::Result<T, String>) -> Step<T> {
        checked.map_err(|message| self.reject(number, message))
    }

    fn check_batch(&mut self, batch: &Batch) {
        for (field, value) in [
            ("agent", &batch.agent),
            ("model", &batch.model),
            ("summary", &batch.summary),
        ] {
            if value.trim().is_empty() {
                self.error(None, format!("`{field}` is required"), Vec::new());
            } else if value.contains('\n') {
                self.error(None, format!("`{field}` must be one line"), Vec::new());
            }
        }
        if batch.notes.iter().any(|note| note.contains('\n')) {
            self.error(None, "each note must be one line".into(), Vec::new());
        }
        let limit = self.vocabulary.limits.ops_per_batch;
        if batch.ops.is_empty() {
            self.error(None, "the batch has no operations".into(), Vec::new());
        } else if batch.ops.len() > limit {
            self.error(
                None,
                format!(
                    "the batch has {} operations; split it into batches of at most {limit}",
                    batch.ops.len()
                ),
                Vec::new(),
            );
        }
    }

    fn op(&mut self, number: usize, op: &Op) -> Step {
        match op {
            Op::SetMetadata { page, key, values } => {
                self.metadata(number, page, key, Some(values), Mode::Set)
            }
            Op::AddMetadata { page, key, values } => {
                self.metadata(number, page, key, Some(values), Mode::Add)
            }
            Op::RemoveMetadata { page, key, values } => {
                self.metadata(number, page, key, values.as_ref(), Mode::Remove)
            }
            Op::CreatePage(page) => self.create_page(number, page),
            Op::SetTitle { page, title } => self.set_title(number, page, title),
            Op::SetSummary { page, summary } => self.set_summary(number, page, summary),
            Op::AddRaw {
                path,
                text,
                base64,
                source,
                source_url,
            } => self.add_raw(
                number,
                &raw_ops::RawFile {
                    path,
                    text: text.as_deref(),
                    base64: base64.as_deref(),
                    source: source.as_deref(),
                    source_url: source_url.as_deref(),
                },
            ),
            Op::ArchivePage { page, reason } => self.archive_page(number, page, reason),
            Op::RestorePage { page, summary } => {
                self.restore_page(number, page, summary.as_deref())
            }
            Op::AddHeading {
                page,
                heading,
                level,
                after,
            } => self.add_heading(number, page, heading, *level, after.as_deref()),
            Op::RenameHeading { page, heading, to } => {
                self.rename_heading(number, page, heading, to)
            }
            Op::DeclareValue {
                key,
                value,
                description,
                synonyms,
            } => self.declare_value(number, key, value, description, synonyms),
            Op::DeclarePlace {
                place,
                title,
                description,
                synonyms,
            } => self.declare_place(number, place, title, description, synonyms),
            Op::DeclareKey {
                key,
                values,
                map,
                single,
                only_on,
                description,
            } => {
                let spec = KeySpec {
                    values: *values,
                    map: *map,
                    single: *single,
                    only_on: only_on.clone(),
                    symmetric: Vec::new(),
                    description: Some(description.clone()),
                };
                self.declare_key(number, key, spec)
            }
            Op::AddSynonym {
                key,
                value,
                synonym,
            } => self.add_synonym(number, key, value, synonym),
            Op::AddPassage {
                page,
                under,
                after,
                text,
                citations,
            } => self.add_passage(
                number,
                page,
                under.as_deref(),
                after.as_deref(),
                text,
                citations,
            ),
            Op::ReplacePassage {
                passage,
                text,
                citations,
                drop_citations,
            } => self.replace_passage(number, passage, text, citations, drop_citations),
            Op::RemovePassage { passage, reason } => self.remove_passage(number, passage, reason),
            Op::AddCitation { passage, citation } => self.add_citation(number, passage, citation),
        }
    }

    /// The path of a published page (as an agent names it); the operation is
    /// rejected, with look-alikes suggested, if there is no such page.
    fn published_path(&mut self, number: usize, page: &str) -> Step<String> {
        let path = page_path(page);
        if self.pages.published.contains(&path) {
            return Ok(path);
        }
        let suggestions = self
            .pages
            .published
            .iter()
            .filter(|p| strsim::jaro_winkler(p, &path) > 0.9)
            .cloned()
            .collect();
        Err(self.reject_with(
            number,
            format!("`{page}` is not a published page"),
            suggestions,
        ))
    }

    /// Load a published page into the plan; the operation is rejected if it
    /// is not one.
    fn page(&mut self, number: usize, page: &str) -> Step<String> {
        let path = page_path(page);
        if self.pages.loaded.contains_key(&path) {
            return Ok(path);
        }
        let path = self.published_path(number, page)?;
        let file = parse::published_file(&self.editor.root, &path)
            .map_err(|message| Error::io(&path, std::io::Error::other(message)))?;
        let original = fs::read_to_string(&file).map_err(|err| Error::io(&file, err))?;
        match Document::parse(&original) {
            Ok(doc) => {
                self.pages.loaded.insert(
                    path.clone(),
                    Staged {
                        file,
                        original,
                        doc,
                    },
                );
                Ok(path)
            }
            Err(message) => Err(self.reject(number, format!("`{path}`: {message}"))),
        }
    }

    fn rewrite(&mut self, number: usize, from: &str, to: &str) {
        self.outcome.rewrites.push(Rewrite {
            op: number,
            from: from.to_string(),
            to: to.to_string(),
        });
    }

    /// The canonical key; the operation is rejected if it is not acceptable.
    fn key(&mut self, number: usize, key: &str) -> Step<String> {
        match self.checker().check_key(key)? {
            Lookup::Known { value } => Ok(value),
            Lookup::Rewritten { value, .. } => {
                self.rewrite(number, key, &value);
                Ok(value)
            }
            Lookup::Unknown { suggestions, .. } => {
                let suggestions = suggestions.into_iter().map(|s| s.value).collect();
                Err(self.reject_with(
                    number,
                    format!("unknown key `{key}`; use a suggestion or declare it with declare_key"),
                    suggestions,
                ))
            }
            Lookup::Invalid { reason, .. } => {
                Err(self.reject(number, format!("key `{key}`: {reason}")))
            }
        }
    }

    /// The canonical values for `key`; every value that is not acceptable is
    /// reported, then the operation is rejected.
    fn values(&mut self, number: usize, key: &str, values: &[String]) -> Step<Vec<String>> {
        let mut accepted = Vec::new();
        let mut ok = true;
        for value in values {
            match self.checker().check_value(key, value)? {
                Lookup::Known { value } => accepted.push(value),
                Lookup::Rewritten {
                    value: canonical, ..
                } => {
                    self.rewrite(number, value, &canonical);
                    accepted.push(canonical);
                }
                Lookup::Unknown { suggestions, .. } => {
                    ok = false;
                    let hint = match self.vocabulary.key(key).map(|spec| spec.values) {
                        Some(ValueKind::Label) => {
                            "; use a suggestion or declare it with declare_value"
                        }
                        Some(ValueKind::Place) => {
                            "; use a suggestion or declare it with declare_place"
                        }
                        _ => "; it must name an existing page",
                    };
                    let suggestions = suggestions.into_iter().map(|s| s.value).collect();
                    self.error(
                        Some(number),
                        format!("`{key}`: unknown value `{value}`{hint}"),
                        suggestions,
                    );
                }
                Lookup::Invalid { reason, .. } => {
                    ok = false;
                    self.error(
                        Some(number),
                        format!("`{key}`: `{value}` is {reason}"),
                        Vec::new(),
                    );
                }
            }
        }
        if ok {
            Ok(accepted)
        } else {
            Err(Stop::Rejected)
        }
    }

    fn metadata(
        &mut self,
        number: usize,
        page: &str,
        key: &str,
        values: Option<&Vec<String>>,
        mode: Mode,
    ) -> Step {
        let path = self.page(number, page)?;
        let key = self.key(number, key)?;
        if key == "title" || MANAGED.contains(&base_key(&key)) {
            return Err(self.reject(
                number,
                format!("`{key}` is set by the editor, not by metadata operations"),
            ));
        }
        let spec = self.vocabulary.key(&key).cloned().unwrap_or(KeySpec {
            values: ValueKind::Text,
            map: None,
            single: false,
            only_on: None,
            symmetric: Vec::new(),
            description: None,
        });
        if spec.map.is_some() && !key.contains('.') {
            return Err(self.reject(
                number,
                format!("`{key}` is a map: set its entries as `{key}.<name>`"),
            ));
        }
        let given = match values {
            Some(values) => Some(self.values(number, &key, values)?),
            None => None,
        };
        let current = self.pages.loaded[&path].doc.frontmatter.values(&key);
        let lower = |list: &[String]| list.iter().map(|v| v.to_lowercase()).collect::<Vec<_>>();
        let new: Option<Vec<String>> = match (mode, given) {
            (Mode::Set, Some(values)) => Some(dedup(values)),
            (Mode::Add, Some(values)) => {
                Some(dedup(current.iter().cloned().chain(values).collect()))
            }
            (Mode::Remove, Some(values)) => {
                let removed = lower(&values);
                Some(
                    current
                        .iter()
                        .filter(|v| !removed.contains(&v.to_lowercase()))
                        .cloned()
                        .collect(),
                )
            }
            (Mode::Remove, None) | (_, None) => None,
        };
        let single = spec.single && !key.contains('.');
        if single && new.as_ref().is_some_and(|v| v.len() > 1) {
            return Err(self.reject(number, format!("`{key}` takes one value")));
        }
        let frontmatter = &mut self.doc_mut(&path).frontmatter;
        match (&new, spec.values) {
            // Absent means false: `contested` is only ever written as true.
            (Some(values), ValueKind::Boolean) => match values.first().map(String::as_str) {
                Some("true") => frontmatter.set_bool(&key, true),
                _ => frontmatter.remove(&key),
            },
            (Some(values), _) if values.is_empty() && key.contains('.') => frontmatter.remove(&key),
            (Some(values), _) => frontmatter.set(&key, values.clone(), single),
            (None, _) => frontmatter.remove(&key),
        }
        self.mirror(
            number,
            &path,
            &key,
            &spec,
            &current,
            new.as_deref().unwrap_or_default(),
        )
    }

    /// Keep symmetric labels of a person map on both people's pages.
    fn mirror(
        &mut self,
        number: usize,
        path: &str,
        key: &str,
        spec: &KeySpec,
        before: &[String],
        after: &[String],
    ) -> Step {
        let Some((base, other)) = key.split_once('.') else {
            return Ok(());
        };
        if spec.map != Some(ValueKind::Person) || spec.symmetric.is_empty() {
            return Ok(());
        }
        let Some(subject) = entity_slug(path) else {
            return Err(self.reject(number, format!("`{base}` is only for person pages")));
        };
        let has = |list: &[String], label: &str| list.iter().any(|v| v.to_lowercase() == label);
        let added: Vec<&String> = spec
            .symmetric
            .iter()
            .filter(|l| has(after, l) && !has(before, l))
            .collect();
        let removed: Vec<&String> = spec
            .symmetric
            .iter()
            .filter(|l| has(before, l) && !has(after, l))
            .collect();
        if added.is_empty() && removed.is_empty() {
            return Ok(());
        }
        let other_path = self.page(number, &entity_path(other))?;
        let other_key = format!("{base}.{subject}");
        let frontmatter = &mut self
            .pages
            .loaded
            .get_mut(&other_path)
            .expect("staged")
            .doc
            .frontmatter;
        let mut labels: Vec<String> = frontmatter
            .values(&other_key)
            .into_iter()
            .filter(|v| !removed.iter().any(|r| r.as_str() == v.to_lowercase()))
            .collect();
        for label in added {
            if !has(&labels, label) {
                labels.push(label.clone());
            }
        }
        if labels.is_empty() {
            frontmatter.remove(&other_key);
        } else {
            frontmatter.set(&other_key, labels, false);
        }
        Ok(())
    }

    /// Bookkeeping and validation of the final state; the files to write.
    fn finish(&mut self, batch: &Batch) -> Result<Vec<Write>> {
        let today = self.editor.today().to_string();
        let mut writes = self.changed_pages(&today)?;
        self.check_archived_references()?;
        writes.extend(self.raw_writes()?);
        writes.extend(self.moves());
        if !self.outcome.errors.is_empty() {
            return Ok(Vec::new());
        }
        writes.extend(self.vocabulary_write());
        writes.extend(self.index_write(&today));
        if writes.is_empty() {
            self.error(None, "the batch changes nothing".into(), Vec::new());
            return Ok(Vec::new());
        }
        writes.push(self.log_write(batch, &today)?);
        Ok(writes)
    }

    /// Check every page the batch changed, as the index would read it: a
    /// problem the batch introduces is an error, one the page already had a
    /// warning. Returns the writes of the pages that changed, with `updated`
    /// set to today.
    fn changed_pages(&mut self, today: &str) -> Result<Vec<Write>> {
        let before = self.editor.checker();
        let mut writes = Vec::new();
        let paths: Vec<String> = self.pages.loaded.keys().cloned().collect();
        for path in paths {
            let staged = self.pages.loaded.get_mut(&path).expect("a staged page");
            if staged.doc.render() == staged.original {
                continue;
            }
            staged
                .doc
                .frontmatter
                .set("updated", vec![today.to_string()], true);
            let content = staged.doc.render();
            let file = staged.file.clone();
            let original = staged.original.clone();
            let after = match conformance::read_page(&path, &content) {
                Ok((page, warnings)) => {
                    let mut problems = self.checker().page_problems(&page, &warnings)?;
                    problems.extend(pages::link_problems(
                        &path,
                        &staged_body(&content),
                        &self.pages.published,
                    ));
                    problems
                }
                Err(message) => {
                    self.error(
                        None,
                        format!("{path} would not be readable: {message}"),
                        Vec::new(),
                    );
                    continue;
                }
            };
            let before = match conformance::read_page(&path, &original) {
                _ if self.pages.fresh.contains(&path) => Vec::new(),
                Ok((page, warnings)) => {
                    let mut problems = before.page_problems(&page, &warnings)?;
                    problems.extend(pages::link_problems(
                        &path,
                        &staged_body(&original),
                        &self.pages.initial_published,
                    ));
                    problems
                }
                Err(_) => Vec::new(),
            };
            for problem in after {
                if before.contains(&problem) {
                    self.outcome
                        .warnings
                        .push(format!("{path}: {problem} (already before this batch)"));
                } else {
                    self.error(None, format!("{path}: {problem}"), Vec::new());
                }
            }
            let lines = content.lines().count();
            if lines > LONG_PAGE_LINES {
                self.outcome.warnings.push(format!(
                    "{path}: {lines} lines; consider splitting the page (over ~{LONG_PAGE_LINES})"
                ));
            }
            writes.push(Write::file(file, content.into_bytes()));
            self.outcome.changed.push(path);
        }
        Ok(writes)
    }

    /// The writes and removals of archived and restored pages.
    fn moves(&mut self) -> Vec<Write> {
        let mut writes = Vec::new();
        for moved in &self.archive.moved {
            writes.push(Write::file(
                self.editor.root.join(&moved.to),
                moved.doc.render().into_bytes(),
            ));
            writes.push(Write::remove(moved.from_file.clone()));
            self.outcome.changed.push(moved.to.clone());
            self.outcome.removed.push(moved.from.clone());
        }
        for (archived, _, file) in &self.archive.restored {
            writes.push(Write::remove(file.clone()));
            self.outcome.removed.push(archived.clone());
        }
        writes
    }

    /// The wiki's vocabulary extension, if the batch declared anything.
    fn vocabulary_write(&mut self) -> Option<Write> {
        if !self.declarations.changed {
            return None;
        }
        let text = format!(
            "# This wiki's additions to Leaf's baseline vocabulary.\n\
             # Edited by leaf-editor; see docs/EDITOR-SPEC.md.\n{}",
            self.declarations.extension.to_yaml()
        );
        self.outcome.changed.push(VOCABULARY_PATH.to_string());
        Some(Write::file(
            self.editor.root.join(VOCABULARY_PATH),
            text.into_bytes(),
        ))
    }

    /// `index.md`, with its header updated, if the batch changed its lines.
    fn index_write(&mut self, today: &str) -> Option<Write> {
        if !self.index.changed {
            return None;
        }
        let total = self.pages.published.len();
        let index = self.index.doc.as_mut().expect("loaded");
        index.set_header(today, total);
        self.outcome.changed.push("index.md".into());
        Some(Write::file(
            self.editor.root.join("index.md"),
            index.render().into_bytes(),
        ))
    }

    /// `log.md` with this batch's entry appended.
    fn log_write(&mut self, batch: &Batch, today: &str) -> Result<Write> {
        let path = self.editor.root.join("log.md");
        let mut log = match fs::read_to_string(&path) {
            Ok(log) => log,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => "# Wiki Log\n".into(),
            Err(err) => return Err(Error::io(&path, err)),
        };
        if !log.ends_with('\n') {
            log.push('\n');
        }
        log.push_str(&self.log_entry(batch, today));
        self.outcome.changed.push("log.md".into());
        Ok(Write::file(path, log.into_bytes()))
    }

    fn log_entry(&self, batch: &Batch, today: &str) -> String {
        let mut entry = format!(
            "\n## [{today}] {} | {}\n- By {} ({}).\n",
            batch.kind.as_str(),
            batch.summary.trim(),
            batch.agent.trim(),
            batch.model.trim()
        );
        for note in &batch.notes {
            entry.push_str(&format!("- {}\n", note.trim()));
        }
        let pages: Vec<String> = self
            .outcome
            .changed
            .iter()
            .filter(|path| {
                path.as_str() != VOCABULARY_PATH
                    && !self.pages.created.contains(path)
                    && !self.raw.content.contains_key(*path)
                    && self.vocabulary.raw_manifest.as_ref() != Some(path)
                    && !self.archive.moved.iter().any(|moved| moved.to == **path)
                    && !self
                        .archive
                        .restored
                        .iter()
                        .any(|(_, restored, _)| restored == *path)
            })
            .map(|path| format!("`{path}`"))
            .collect();
        for moved in &self.archive.moved {
            entry.push_str(&format!(
                "- Archived: `{}` to `{}` ({}).\n",
                moved.from, moved.to, moved.reason
            ));
        }
        for (archived, restored, _) in &self.archive.restored {
            entry.push_str(&format!("- Restored: `{archived}` to `{restored}`.\n"));
        }
        if !self.raw.paths.is_empty() {
            let added: Vec<String> = self.raw.paths.iter().map(|p| format!("`{p}`")).collect();
            entry.push_str(&format!("- Added raw: {}.\n", added.join(", ")));
        }
        if !self.pages.created.is_empty() {
            let created: Vec<String> = self
                .pages
                .created
                .iter()
                .map(|p| format!("`{p}`"))
                .collect();
            entry.push_str(&format!("- Created: {}.\n", created.join(", ")));
        }
        if !pages.is_empty() {
            entry.push_str(&format!("- Changed: {}.\n", pages.join(", ")));
        }
        if !self.declarations.log.is_empty() {
            entry.push_str(&format!(
                "- Declared: {}.\n",
                self.declarations.log.join("; ")
            ));
        }
        if !self.removed_passages.is_empty() {
            entry.push_str(&format!(
                "- Removed passages: {}.\n",
                self.removed_passages.join("; ")
            ));
        }
        entry
    }
}

/// The part of a page after its frontmatter.
fn staged_body(content: &str) -> String {
    match parse::frontmatter_block(content) {
        Some((range, _)) => content[range.end..].to_string(),
        None => content.to_string(),
    }
}

/// Values in order, without case-insensitive duplicates.
fn dedup(values: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    values
        .into_iter()
        .filter(|value| {
            let lower = value.to_lowercase();
            let new = !seen.contains(&lower);
            seen.push(lower);
            new
        })
        .collect()
}
