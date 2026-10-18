//! Print a page as the editor sees it (title, metadata, headings, passages
//! with their IDs and citations) as JSON:
//! `cargo run -p leaf-editor --example page -- <wiki-root> <cache.db> <page>`
//! Read-only.

use leaf_editor::{Editor, EditorConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, db, page] = args.as_slice() else {
        return Err("usage: page <wiki-root> <cache.db> <page>".into());
    };
    let editor = Editor::open(EditorConfig::new(root, db))?;
    println!("{}", serde_json::to_string_pretty(&editor.page(page)?)?);
    Ok(())
}
