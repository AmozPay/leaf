//! `leaf`: command-line access to a Leaf index.
//!
//! Exit status: 0 on success, 1 on error, 2 on invalid arguments, 3 when
//! `sync` could not index every published page, 4 when `editor check` or
//! `editor apply` rejects the batch.

// Argument doc comments are `--help` text, where `<path>` is a placeholder,
// not HTML.
#![allow(rustdoc::invalid_html_tags)]

mod cli;
#[cfg(feature = "editor")]
mod editor;
mod output;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command, ViewerCommand};
use leaf_viewer::{Citation, Error, WikiConfig, WikiIndex};
use std::io::{self, ErrorKind};
use std::process::ExitCode;

/// `sync` left at least one published page unindexed.
const EXIT_INCOMPLETE: u8 = 3;
/// The editor rejected the batch.
#[cfg(feature = "editor")]
const EXIT_REJECTED: u8 = 4;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            report_error(&err);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode> {
    match &cli.command {
        Command::Viewer { command } => run_viewer(cli, command),
        #[cfg(feature = "editor")]
        Command::Editor { command } => editor::run(cli, command),
        #[cfg(not(feature = "editor"))]
        Command::Editor { .. } => anyhow::bail!(
            "the editor is not built into this binary; rebuild with `cargo install --path . --features editor`"
        ),
    }
}

fn run_viewer(cli: &Cli, command: &ViewerCommand) -> Result<ExitCode> {
    if let ViewerCommand::Meta {
        suggested: true, ..
    } = command
    {
        let mut out = io::stdout().lock();
        output::suggested_keys(&mut out, leaf_viewer::dictionary::dictionary(), cli.json)?;
        return Ok(ExitCode::SUCCESS);
    }
    let config = WikiConfig::new(cli.wiki_root()?, cli.database()?)
        .rebuild_incompatible_cache(!cli.no_rebuild);
    let mut index = WikiIndex::open(config)?;
    if let Some(old) = index.rebuilt_from() {
        eprintln!(
            "leaf: rebuilt the cache for this version (schema {old} → {}); every page is re-indexed",
            WikiIndex::schema_version()
        );
    }
    let mut out = io::stdout().lock();
    match command {
        ViewerCommand::Sync => {
            let report = index.sync()?;
            output::sync(&mut out, &report, cli.json)?;
            if !report.is_complete() {
                return Ok(ExitCode::from(EXIT_INCOMPLETE));
            }
        }
        ViewerCommand::Search(args) => {
            sync_quietly(cli, &mut index)?;
            let response = index.search(&args.request())?;
            output::search(&mut out, &response, args.max_chars, cli.json)?;
        }
        ViewerCommand::Cite {
            reference,
            max_chars,
        } => {
            let citation: Citation = reference.parse()?;
            sync_quietly(cli, &mut index)?;
            let excerpt = index.read_citation(&citation, *max_chars)?;
            output::citation(&mut out, &mut io::stderr(), &excerpt, cli.json)?;
        }
        ViewerCommand::Tags => {
            sync_quietly(cli, &mut index)?;
            output::counts(&mut out, &index.tags()?, "no tagged pages", cli.json)?;
        }
        ViewerCommand::People => {
            sync_quietly(cli, &mut index)?;
            output::people(&mut out, &index.people()?, cli.json)?;
        }
        ViewerCommand::Places => {
            sync_quietly(cli, &mut index)?;
            output::counts(&mut out, &index.places()?, "no related places", cli.json)?;
        }
        ViewerCommand::Meta { kind, .. } => {
            sync_quietly(cli, &mut index)?;
            match kind {
                Some(kind) => {
                    let values = index.facet_values(kind)?;
                    output::counts(&mut out, &values, "no values for this kind", cli.json)?;
                }
                None => output::counts(
                    &mut out,
                    &index.facets()?,
                    "no frontmatter metadata",
                    cli.json,
                )?,
            }
        }
        ViewerCommand::Stats => output::stats(&mut out, &index.stats()?, cli.json)?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Sync before reading, mentioning (not failing on) pages that could not be
/// indexed: they are simply absent from the results.
fn sync_quietly(cli: &Cli, index: &mut WikiIndex) -> Result<()> {
    if cli.no_sync {
        return Ok(());
    }
    let report = index.sync()?;
    if !report.is_complete() {
        eprintln!(
            "leaf: warning: {} published page(s) could not be indexed; run `leaf viewer sync` for details",
            report.errors.len()
        );
    }
    Ok(())
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.downcast_ref::<io::Error>()
        .is_some_and(|err| err.kind() == ErrorKind::BrokenPipe)
}

fn report_error(err: &anyhow::Error) {
    eprintln!("leaf: error: {err:#}");
    let hint = match err.downcast_ref::<Error>() {
        Some(Error::IncompatibleSchema { .. }) => "rerun without --no-rebuild to rebuild the cache",
        Some(Error::CitationNotIndexed) => {
            "only citations attached to indexed passages can be read; copy one from `leaf viewer search`"
        }
        Some(Error::Io { path, .. }) if path.ends_with("index.md") => {
            "check --wiki (or LEAF_WIKI): the wiki root must contain index.md"
        }
        _ => return,
    };
    eprintln!("hint: {hint}");
}
