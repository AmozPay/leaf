//! `leaf editor …`: validated edits (see `docs/EDITOR-SPEC.md`).

use crate::cli::{Cli, EditorCommand};
use crate::output::json;
use crate::{EXIT_INCOMPLETE, EXIT_REJECTED};
use anyhow::{Context, Result};
use leaf_editor::{
    Batch, BatchOutcome, ConformanceReport, Editor, EditorConfig, InitOptions, Lookup, MetadataKey,
    PageView, init_wiki,
};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

pub fn run(cli: &Cli, command: &EditorCommand) -> Result<ExitCode> {
    let mut out = io::stdout().lock();
    if let EditorCommand::Init { domain, tags } = command {
        let options = InitOptions {
            domain: domain.clone(),
            tags: tags.clone(),
            today: None,
        };
        let root = cli.wiki_root()?;
        let files = init_wiki(&root, &options)?;
        init_report(&mut out, &files, cli.json)?;
        return Ok(ExitCode::SUCCESS);
    }
    let open = || -> Result<Editor> {
        let config = EditorConfig::new(cli.wiki_root()?, cli.database()?);
        Ok(Editor::open(config)?)
    };
    match command {
        EditorCommand::Init { .. } => unreachable!("handled before the wiki is opened"),
        EditorCommand::Check { batch } => {
            let batch = read_batch(batch)?;
            let outcome = open()?.check(&batch)?;
            outcome_report(&mut out, &outcome, cli.json)?;
            Ok(exit_for(&outcome))
        }
        EditorCommand::Apply { batch } => {
            let batch = read_batch(batch)?;
            let outcome = open()?.apply(&batch)?;
            outcome_report(&mut out, &outcome, cli.json)?;
            Ok(exit_for(&outcome))
        }
        EditorCommand::Metadata => {
            metadata(&mut out, &open()?.list_metadata()?, cli.json)?;
            Ok(ExitCode::SUCCESS)
        }
        EditorCommand::Suggest { key, value } => {
            let editor = open()?;
            let lookup = match value {
                Some(value) => editor.check_value(key, value)?,
                None => editor.check_key(key)?,
            };
            lookup_report(&mut out, &lookup, cli.json)?;
            Ok(ExitCode::SUCCESS)
        }
        EditorCommand::Conformance => {
            conformance(&mut out, &open()?.conformance()?, cli.json)?;
            Ok(ExitCode::SUCCESS)
        }
        EditorCommand::Page { page } => {
            page_report(&mut out, &open()?.page(page)?, cli.json)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// A batch from a file or, for `-`, from standard input.
fn read_batch(path: &Path) -> Result<Batch> {
    let text = if path == Path::new("-") {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("cannot read the batch from standard input")?;
        text
    } else {
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?
    };
    Batch::parse(&text).map_err(anyhow::Error::msg)
}

/// What `init` created. The wiki's folder is left out on purpose: an agent
/// that runs this command is not told where the wiki lives.
fn init_report(out: &mut impl Write, files: &[String], as_json: bool) -> io::Result<()> {
    if as_json {
        return json(out, &serde_json::json!({"created": files}));
    }
    writeln!(out, "created a new wiki")?;
    for file in files {
        writeln!(out, "created  {file}")?;
    }
    writeln!(
        out,
        "The first pages need at least two links to other pages: create three interlinked pages in one batch."
    )
}

fn exit_for(outcome: &BatchOutcome) -> ExitCode {
    if !outcome.is_acceptable() {
        ExitCode::from(EXIT_REJECTED)
    } else if outcome
        .sync
        .as_ref()
        .is_some_and(|sync| !sync.is_complete())
    {
        ExitCode::from(EXIT_INCOMPLETE)
    } else {
        ExitCode::SUCCESS
    }
}

fn outcome_report(out: &mut impl Write, outcome: &BatchOutcome, as_json: bool) -> io::Result<()> {
    if as_json {
        return json(out, outcome);
    }
    if outcome.is_acceptable() {
        writeln!(
            out,
            "{}",
            if outcome.applied {
                "applied"
            } else {
                "acceptable (nothing written)"
            }
        )?;
    } else {
        writeln!(
            out,
            "rejected: {} error(s); nothing was written",
            outcome.errors.len()
        )?;
    }
    for error in &outcome.errors {
        match error.op {
            Some(op) => writeln!(out, "error    op {op}: {}", error.message)?,
            None => writeln!(out, "error    {}", error.message)?,
        }
        if !error.suggestions.is_empty() {
            writeln!(
                out,
                "         did you mean: {}",
                error.suggestions.join(", ")
            )?;
        }
    }
    for warning in &outcome.warnings {
        writeln!(out, "warning  {warning}")?;
    }
    for rewrite in &outcome.rewrites {
        writeln!(
            out,
            "rewrite  op {}: `{}` → `{}`",
            rewrite.op, rewrite.from, rewrite.to
        )?;
    }
    if outcome.is_acceptable() {
        let verb = if outcome.applied {
            "changed"
        } else {
            "would change"
        };
        for path in &outcome.changed {
            writeln!(out, "{verb}  {path}")?;
        }
        for path in &outcome.removed {
            writeln!(out, "removed  {path}")?;
        }
        for passage in &outcome.passages {
            writeln!(out, "passage  op {}: {}", passage.op, passage.id)?;
        }
    }
    if let Some(sync) = &outcome.sync {
        writeln!(
            out,
            "synced: added {}, updated {}, removed {}, unchanged {}",
            sync.added, sync.updated, sync.removed, sync.skipped
        )?;
        for issue in &sync.errors {
            writeln!(out, "error    {}: {}", issue.path, issue.message)?;
        }
    }
    Ok(())
}

fn metadata(out: &mut impl Write, keys: &[MetadataKey], as_json: bool) -> io::Result<()> {
    if as_json {
        return json(out, &keys);
    }
    for key in keys {
        let kind = key.spec.as_ref().map_or_else(
            || "undeclared".to_string(),
            |spec| format!("{:?}", spec.values).to_lowercase(),
        );
        writeln!(out, "{} ({kind}, {} pages)", key.key, key.pages)?;
        if let Some(description) = key.spec.as_ref().and_then(|spec| spec.description.as_ref()) {
            writeln!(out, "    {description}")?;
        }
        for value in &key.values {
            let flag = if value.declared {
                ""
            } else {
                "  [not declared]"
            };
            let note = value
                .description
                .as_deref()
                .map_or(String::new(), |d| format!(": {d}"));
            writeln!(out, "    {} ({}){note}{flag}", value.value, value.pages)?;
        }
    }
    Ok(())
}

fn lookup_report(out: &mut impl Write, lookup: &Lookup, as_json: bool) -> io::Result<()> {
    if as_json {
        return json(out, lookup);
    }
    match lookup {
        Lookup::Known { value } => writeln!(out, "known: {value}"),
        Lookup::Rewritten { value, from, .. } => writeln!(out, "rewritten: `{from}` → `{value}`"),
        Lookup::Unknown { value, suggestions } => {
            writeln!(out, "unknown: {value}")?;
            for suggestion in suggestions {
                writeln!(out, "    did you mean: {}", suggestion.value)?;
            }
            Ok(())
        }
        Lookup::Invalid { value, reason } => writeln!(out, "invalid: {value}: {reason}"),
    }
}

fn conformance(out: &mut impl Write, report: &ConformanceReport, as_json: bool) -> io::Result<()> {
    if as_json {
        return json(out, report);
    }
    write!(out, "{}", report.to_text())
}

fn page_report(out: &mut impl Write, page: &PageView, as_json: bool) -> io::Result<()> {
    if as_json {
        return json(out, page);
    }
    writeln!(out, "{}  {}", page.path, page.title)?;
    for (key, values) in &page.metadata {
        writeln!(out, "  {key}: {}", values.join(", "))?;
    }
    writeln!(out, "headings: {}", page.headings.join(" | "))?;
    for passage in &page.passages {
        writeln!(
            out,
            "\n{}\n  [{}] {}",
            passage.id, passage.heading, passage.text
        )?;
        for citation in &passage.citations {
            writeln!(out, "  cites {citation}")?;
        }
    }
    Ok(())
}
