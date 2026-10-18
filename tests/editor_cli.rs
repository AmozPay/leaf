#![cfg_attr(not(feature = "editor"), allow(dead_code, unused_imports))]
//! `leaf editor …` end to end on a temporary wiki. Needs the `editor`
//! feature (`cargo test --all-features`); without it, only the hint that the
//! editor is not built in is tested.

use assert_cmd::Command;
use leaf_testkit::{TempWiki, snapshot};
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

fn fixture() -> TempWiki {
    let wiki = TempWiki::new();
    wiki.write(
        "raw/notes/a.md",
        "---\nsource: direct statement\n---\n### user\n\nBeds are raised.\n\n^message-a\n",
    );
    wiki.write("index.md", "[[concepts/garden]]\n");
    wiki.write("log.md", "# Wiki Log\n");
    wiki.write(
        "_meta/vocabulary.yaml",
        "labels:\n  tags: {gardening: {}}\n",
    );
    wiki.write(
        "concepts/garden.md",
        "---\ntitle: Garden\ntype: concept\ncreated: 2026-10-01\nupdated: 2026-10-02\n\
         tags: [gardening]\nsources: [raw/notes/a.md]\nconfidence: medium\n---\n\
         # Garden\n\nAn introduction.\n\n## Beds\n\nBeds are raised. [[raw/notes/a.md#^message-a]]\n",
    );
    wiki
}

fn cmd(wiki: &TempWiki) -> Command {
    let mut cmd = assert_cmd::cargo_bin_cmd!("leaf");
    cmd.env_remove("LEAF_WIKI")
        .env_remove("LEAF_DB")
        .arg("--wiki")
        .arg(wiki.root())
        .arg("--db")
        .arg(wiki.db());
    cmd
}

fn write_batch(wiki: &TempWiki, name: &str, ops: &str) -> PathBuf {
    let path = wiki.path(name);
    fs::write(
        &path,
        format!("agent: curator\nmodel: small-8b\nkind: update\nsummary: CLI\nops:\n{ops}"),
    )
    .unwrap();
    path
}

const ADD: &str = "- {op: add_passage, page: concepts/garden, under: Beds, text: Beds face south., \
                   citations: ['raw/notes/a.md#^message-a']}\n";

#[cfg(not(feature = "editor"))]
#[test]
fn the_editor_is_not_built_in_by_default() {
    let wiki = fixture();
    cmd(&wiki)
        .args(["editor", "page", "concepts/garden"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--features editor"));
    cmd(&wiki)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("viewer"))
        .stdout(predicate::str::contains("editor").not());
}

#[cfg(feature = "editor")]
mod editor {
    use super::*;
    use serde_json::Value;

    #[test]
    fn init_creates_a_conformant_wiki_and_refuses_an_existing_one() {
        let wiki = TempWiki::new();
        cmd(&wiki)
            .args([
                "editor",
                "init",
                "--domain",
                "Greenhouse notes",
                "--tag",
                "gardening,soil",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("created  SCHEMA.md"))
            .stdout(predicate::str::contains("three interlinked pages"))
            // The folder is not echoed: whoever reads this output need not know it.
            .stdout(predicate::str::contains(wiki.root().display().to_string().as_str()).not());
        assert!(wiki.read("_meta/vocabulary.yaml").contains("gardening"));
        cmd(&wiki)
            .args(["editor", "conformance"])
            .assert()
            .success()
            .stdout(predicate::str::contains("No issues."));
        let before = snapshot(&wiki.root());
        let root = wiki.root().display().to_string();
        cmd(&wiki)
            .args(["editor", "init"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("not empty"))
            .stderr(predicate::str::contains(root.as_str()).not());
        assert_eq!(snapshot(&wiki.root()), before);
        let json = cmd(&TempWiki::new())
            .args(["--json", "editor", "init"])
            .assert()
            .success();
        let created: Value = serde_json::from_slice(&json.get_output().stdout).unwrap();
        assert_eq!(created["created"][0], "SCHEMA.md");
    }

    #[test]
    fn check_reports_without_writing() {
        let wiki = fixture();
        let batch = write_batch(&wiki, "batch.yaml", ADD);
        let before = snapshot(&wiki.root());
        cmd(&wiki)
            .args(["editor", "check"])
            .arg(&batch)
            .assert()
            .success()
            .stdout(predicate::str::contains("acceptable (nothing written)"))
            .stdout(predicate::str::contains("would change  concepts/garden.md"));
        // Opening the editor syncs the cache (outside the wiki) and nothing else.
        assert_eq!(snapshot(&wiki.root()), before);
        let output = cmd(&wiki)
            .args(["--json", "editor", "check"])
            .arg(&batch)
            .assert()
            .success();
        let outcome: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
        assert_eq!(outcome["applied"], false);
        assert_eq!(outcome["errors"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn apply_writes_and_a_rejected_batch_writes_nothing() {
        let wiki = fixture();
        let batch = write_batch(&wiki, "batch.yaml", ADD);
        cmd(&wiki)
            .args(["editor", "apply"])
            .arg(&batch)
            .assert()
            .success()
            .stdout(predicate::str::contains("applied"))
            .stdout(predicate::str::contains(
                "passage  op 0: concepts/garden.md#p-",
            ))
            .stdout(predicate::str::contains("synced:"));
        assert!(
            wiki.read("concepts/garden.md")
                .contains("Beds face south. [[raw/notes/a.md#^message-a]]")
        );
        assert!(wiki.read("log.md").contains("| CLI\n"));

        let bad = write_batch(
            &wiki,
            "bad.yaml",
            "- {op: add_metadata, page: concepts/garden, key: tags, values: [gardnening]}\n",
        );
        let before = snapshot(&wiki.root());
        cmd(&wiki)
            .args(["editor", "apply"])
            .arg(&bad)
            .assert()
            .code(4)
            .stdout(predicate::str::contains(
                "rejected: 1 error(s); nothing was written",
            ))
            .stdout(predicate::str::contains("error    op 0:"))
            .stdout(predicate::str::contains("did you mean: gardening"));
        assert_eq!(snapshot(&wiki.root()), before);
    }

    #[test]
    fn a_batch_can_come_from_standard_input() {
        let wiki = fixture();
        let text = fs::read_to_string(write_batch(&wiki, "batch.yaml", ADD)).unwrap();
        cmd(&wiki)
            .args(["editor", "apply", "-"])
            .write_stdin(text)
            .assert()
            .success();
        assert!(wiki.read("concepts/garden.md").contains("Beds face south."));
        cmd(&wiki)
            .args(["editor", "check", "-"])
            .write_stdin("not: a batch\n")
            .assert()
            .code(1)
            .stderr(predicate::str::contains("invalid batch"));
        cmd(&wiki)
            .args(["editor", "check", "missing.yaml"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("cannot read missing.yaml"));
    }

    #[test]
    fn reading_commands_describe_the_wiki() {
        let wiki = fixture();
        cmd(&wiki)
            .args(["editor", "page", "concepts/garden"])
            .assert()
            .success()
            .stdout(predicate::str::contains("concepts/garden.md  Garden"))
            .stdout(predicate::str::contains("headings: Garden | Beds"))
            .stdout(predicate::str::contains("cites raw/notes/a.md#^message-a"));
        let page: Value = serde_json::from_slice(
            &cmd(&wiki)
                .args(["--json", "editor", "page", "concepts/garden"])
                .assert()
                .success()
                .get_output()
                .stdout,
        )
        .unwrap();
        assert!(
            page["passages"][1]["id"]
                .as_str()
                .unwrap()
                .starts_with("concepts/garden.md#p-")
        );
        cmd(&wiki)
            .args(["editor", "page", "concepts/missing"])
            .assert()
            .code(1);

        cmd(&wiki)
            .args(["editor", "metadata"])
            .assert()
            .success()
            .stdout(predicate::str::contains("tags ("))
            .stdout(predicate::str::contains("gardening (1)"));
        cmd(&wiki)
            .args(["editor", "suggest", "persons"])
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "rewritten: `persons` → `related-people`",
            ));
        cmd(&wiki)
            .args(["editor", "suggest", "tags", "gardnening"])
            .assert()
            .success()
            .stdout(predicate::str::contains("unknown: gardnening"))
            .stdout(predicate::str::contains("did you mean: gardening"));
        cmd(&wiki)
            .args(["editor", "conformance"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Checked 1 published pages."));
    }
}
