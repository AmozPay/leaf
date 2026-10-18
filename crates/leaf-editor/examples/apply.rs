//! Check or apply a batch file (YAML or JSON) and print the outcome as JSON:
//! `cargo run -p leaf-editor --example apply -- [--check] <wiki-root> <cache.db> <batch.yaml>`
//! Without `--check` this WRITES to the wiki; point it at a copy to rehearse.

use leaf_editor::{Batch, Editor, EditorConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.first().is_some_and(|a| a == "--check");
    if check {
        args.remove(0);
    }
    let [root, db, file] = args.as_slice() else {
        return Err("usage: apply [--check] <wiki-root> <cache.db> <batch.yaml>".into());
    };
    let batch = Batch::parse(&std::fs::read_to_string(file)?)?;
    let mut editor = Editor::open(EditorConfig::new(root, db))?;
    let outcome = if check {
        editor.check(&batch)?
    } else {
        editor.apply(&batch)?
    };
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}
