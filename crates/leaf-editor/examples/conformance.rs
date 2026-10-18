//! Check a wiki against its vocabulary (the baseline plus its own extension),
//! read-only: `cargo run -p leaf-editor --example conformance -- <wiki-root> <cache.db>`
//! The cache must be outside the wiki; nothing in the wiki is written.

use leaf_editor::{Editor, EditorConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: conformance <wiki-root> <cache.db>";
    let root = args.next().ok_or(usage)?;
    let db = args.next().ok_or(usage)?;
    let editor = Editor::open(EditorConfig::new(root, db))?;
    print!("{}", editor.conformance()?.to_text());
    Ok(())
}
