//! `leaf-mcp`: an MCP server over stdio for a local wiki.
//!
//! Tools: `search`, `cite`, `metadata`, `people`, `stats` (read-only). Built
//! with `--features editor`, it also has `editor_check`, `editor_apply`,
//! `editor_page`, `editor_metadata`, `editor_suggest` and
//! `editor_conformance`, the only way an agent changes the wiki. Logs go to
//! stderr; stdout carries the protocol.

use anyhow::{Context, Result};
use clap::Parser;
use leaf_mcp::Leaf;
use rmcp::{ServiceExt, transport::stdio};
use std::path::PathBuf;

/// MCP server for a Leaf wiki, over stdio.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// Wiki root, containing index.md and raw/ [default: ~/wiki]
    #[arg(long, env = "LEAF_WIKI")]
    wiki: Option<PathBuf>,

    /// Cache database; must be outside raw/ [default: ~/.cache/leaf/leaf.db]
    #[arg(long, env = "LEAF_DB")]
    db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let wiki = match args.wiki {
        Some(wiki) => wiki,
        None => leaf_viewer::default_wiki_root().context("pass --wiki")?,
    };
    let db = match args.db {
        Some(db) => db,
        None => leaf_viewer::default_database_path().context("pass --db")?,
    };
    let server = Leaf::open(&wiki, &db)?;
    eprintln!(
        "leaf-mcp: serving {} ({})",
        wiki.display(),
        if cfg!(feature = "editor") {
            "read and edit"
        } else {
            "read-only"
        }
    );
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
