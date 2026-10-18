//! Helpers the editor's integration tests share.

#![allow(dead_code)]

use leaf_editor::{Batch, BatchOutcome, Editor, EditorConfig};
pub use leaf_testkit::{TempWiki, snapshot};

/// The editor of a temporary wiki, with a fixed clock: 2026-10-17.
pub trait EditorExt {
    fn editor(&self) -> Editor;
}

impl EditorExt for TempWiki {
    fn editor(&self) -> Editor {
        let config =
            EditorConfig::new(self.root(), self.db()).with_today("2026-10-17".parse().unwrap());
        Editor::open(config).unwrap()
    }
}

/// A batch by `curator`, with these operations (YAML list items).
pub fn batch(ops: &str) -> Batch {
    Batch::parse(&format!(
        "agent: curator\nmodel: small-8b\nkind: update\nsummary: Test batch\nnotes: [Because.]\nops:\n{ops}"
    ))
    .unwrap()
}

/// The errors of an outcome, with their suggestions, one per line.
pub fn errors(outcome: &BatchOutcome) -> String {
    outcome
        .errors
        .iter()
        .map(|e| format!("{} {:?}", e.message, e.suggestions))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Apply a batch that must be rejected: every file stays as it was.
/// Returns its errors.
pub fn rejected(wiki: &TempWiki, ops: &str) -> String {
    let before = snapshot(&wiki.root());
    let outcome = wiki.editor().apply(&batch(ops)).unwrap();
    assert!(!outcome.applied, "{outcome:#?}");
    assert!(!outcome.errors.is_empty());
    assert!(outcome.changed.is_empty());
    assert_eq!(
        snapshot(&wiki.root()),
        before,
        "a rejected batch changed files"
    );
    errors(&outcome)
}
