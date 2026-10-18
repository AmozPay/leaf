//! Read-only smoke run against a real wiki:
//! `cargo run -p leaf-viewer --example smoke -- <wiki-root> <db-path> <query> [raw/<path>[#anchor] ...]`

use leaf_viewer::{Citation, SearchRequest, WikiConfig, WikiIndex};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let usage = "usage: smoke <wiki-root> <db-path> <query> [raw/<path>[#anchor] ...]";
    let root = args.next().ok_or(usage)?;
    let db = args.next().ok_or(usage)?;
    let query = args.next().ok_or(usage)?;

    let mut wiki = WikiIndex::open(WikiConfig::new(root, db))?;
    let report = wiki.sync()?;
    println!(
        "sync: added={} updated={} removed={} skipped={} errors={} warnings={} complete={}",
        report.added,
        report.updated,
        report.removed,
        report.skipped,
        report.errors.len(),
        report.warnings.len(),
        report.is_complete()
    );
    for issue in report.errors.iter().chain(&report.warnings) {
        println!("  {}: {}", issue.path, issue.message);
    }
    println!("stats: {:?}", wiki.stats()?);

    let result = wiki.search(&SearchRequest::new(query, 3, 1200))?;
    for hit in &result.hits {
        println!(
            "#{} score={:.2} {} / {} [{}]\n  {}\n  citations: {}",
            hit.rank,
            hit.score,
            hit.page_path,
            hit.heading,
            hit.passage_id,
            hit.text,
            hit.citations
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!(
        "hits={} chars={} truncated={}",
        result.hits.len(),
        result.chars_used,
        result.truncated
    );

    for reference in args {
        let citation: Citation = reference.parse()?;
        match wiki.read_citation(&citation, 300) {
            Ok(excerpt) => println!(
                "citation {reference} ({}, truncated={}):\n{}",
                excerpt.media_type,
                excerpt.truncated,
                excerpt.text.as_deref().unwrap_or("(not text)")
            ),
            Err(err) => println!("citation {reference}: error: {err}"),
        }
    }
    Ok(())
}
