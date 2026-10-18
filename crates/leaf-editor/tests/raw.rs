//! Raw sources end to end: adding files to declared collections, their
//! frontmatter and checksums, the manifest, and citing them in the same batch.

use leaf_common::Citation;
use sha2::{Digest, Sha256};
use std::fs;
mod common;

use common::{EditorExt, TempWiki, batch, errors, rejected, snapshot};

const VOCABULARY: &str = "raw_manifest: _meta/raw-manifest.json\n\
    raw_collections:\n  \
    confirmations: {media: [text/markdown], require_anchor: message, require_frontmatter: [source]}\n  \
    exports: {addable: false}\n";

fn fixture() -> TempWiki {
    let wiki = TempWiki::new();
    wiki.write(
        "raw/confirmations/2026-10-01-first.md",
        "---\nsource: direct statement\ningested: 2026-10-01\nsha256: ab\n---\n\
         ^message-first\nBeds are raised.\n",
    );
    wiki.write("raw/exports/old.md", "An original export.\n");
    wiki.write("index.md", "[[concepts/garden]]\n");
    wiki.write("log.md", "# Wiki Log\n");
    wiki.write("_meta/vocabulary.yaml", VOCABULARY);
    wiki.write(
        "_meta/raw-manifest.json",
        "{\n  \"created\": \"2026-10-01\",\n  \"files\": [\n    {\n      \"path\": \"raw/confirmations/2026-10-01-first.md\",\n      \"bytes\": 10,\n      \"sha256\": \"aa\"\n    },\n    {\n      \"path\": \"raw/exports/old.md\",\n      \"bytes\": 20,\n      \"sha256\": \"bb\"\n    }\n  ]\n}\n",
    );
    wiki.write(
        "concepts/garden.md",
        "---\ntitle: Garden\ntype: concept\ncreated: 2026-10-01\nupdated: 2026-10-02\n\
         tags: []\nsources: [raw/confirmations/2026-10-01-first.md]\nconfidence: medium\n---\n\
         # Garden\n\nAn introduction.\n\n## Beds\n\nBeds are raised. [[raw/confirmations/2026-10-01-first.md#^message-first]]\n",
    );
    wiki
}

const CONFIRMATION: &str = "- op: add_raw\n  path: raw/confirmations/2026-10-17-roles.md\n  \
    source: Alice, direct statement in session 20261017\n  \
    text: |\n    ^message-roles\n    Bobby still maintains the Greenhouse setup.\n";

#[test]
fn a_markdown_source_gets_its_frontmatter_checksum_and_manifest_entry() {
    let wiki = fixture();
    let existing = snapshot(&wiki.root().join("raw"));
    let mut editor = wiki.editor();
    let outcome = editor.apply(&batch(CONFIRMATION)).unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(
        outcome.changed,
        [
            "raw/confirmations/2026-10-17-roles.md",
            "_meta/raw-manifest.json",
            "log.md"
        ]
    );
    let body = "^message-roles\nBobby still maintains the Greenhouse setup.\n";
    let written = wiki.read("raw/confirmations/2026-10-17-roles.md");
    assert_eq!(
        written,
        format!(
            "---\nsource: Alice, direct statement in session 20261017\ningested: 2026-10-17\n\
             sha256: {:x}\n---\n{body}",
            Sha256::digest(body.as_bytes())
        )
    );
    // The manifest records the whole file, in path order, and keeps its layout.
    let manifest: serde_json::Value =
        serde_json::from_str(&wiki.read("_meta/raw-manifest.json")).unwrap();
    let files = manifest["files"].as_array().unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        [
            "raw/confirmations/2026-10-01-first.md",
            "raw/confirmations/2026-10-17-roles.md",
            "raw/exports/old.md"
        ]
    );
    assert_eq!(files[1]["bytes"], written.len());
    assert_eq!(
        files[1]["sha256"],
        format!("{:x}", Sha256::digest(written.as_bytes()))
    );
    assert_eq!(manifest["created"], "2026-10-01");
    assert!(
        wiki.read("_meta/raw-manifest.json")
            .starts_with("{\n  \"created\"")
    );
    assert!(
        wiki.read("log.md")
            .contains("- Added raw: `raw/confirmations/2026-10-17-roles.md`.\n")
    );
    // Existing raw files are untouched.
    let after = snapshot(&wiki.root().join("raw"));
    assert!(existing.iter().all(|file| after.contains(file)));
    // The viewer's citation reader finds the new message in the written file.
    let citation: Citation = "raw/confirmations/2026-10-17-roles.md#^message-roles"
        .parse()
        .unwrap();
    leaf_common::parse::check_citation(&wiki.root(), &citation).unwrap();
}

#[test]
fn a_new_source_can_be_cited_in_the_same_batch() {
    let wiki = fixture();
    let ops = format!(
        "{CONFIRMATION}\
         - {{op: add_passage, page: concepts/garden, under: Beds, text: Bobby maintains the setup., \
         citations: ['raw/confirmations/2026-10-17-roles.md#^message-roles']}}\n\
         - {{op: add_metadata, page: concepts/garden, key: sources, values: [raw/confirmations/2026-10-17-roles.md]}}\n"
    );
    let outcome = wiki.editor().apply(&batch(&ops)).unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert!(wiki.read("concepts/garden.md").contains(
        "Bobby maintains the setup. [[raw/confirmations/2026-10-17-roles.md#^message-roles]]"
    ));
    // A citation to an anchor the new file does not have is rejected.
    let wiki = fixture();
    let bad = ops.replace("^message-roles']", "^message-other']");
    assert!(rejected(&wiki, &bad).contains("anchor not found"));
}

#[test]
fn a_binary_source_is_stored_as_given() {
    let wiki = fixture();
    // %PDF-1.4 and a few bytes that are not text.
    let bytes: &[u8] = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n1 0 obj\n";
    let encoded = base64_of(bytes);
    let outcome = wiki
        .editor()
        .apply(&batch(&format!(
            "- {{op: add_raw, path: raw/papers/paper.pdf, base64: '{encoded}'}}\n"
        )))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(
        fs::read(wiki.root().join("raw/papers/paper.pdf")).unwrap(),
        bytes
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&wiki.read("_meta/raw-manifest.json")).unwrap();
    let entry = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "raw/papers/paper.pdf")
        .unwrap();
    assert_eq!(entry["bytes"], bytes.len());
    assert_eq!(entry["sha256"], format!("{:x}", Sha256::digest(bytes)));
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn a_source_that_does_not_fit_its_collection_is_rejected() {
    let wiki = fixture();
    let png = base64_of(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
    let cases: Vec<(String, &str)> = vec![
        (
            CONFIRMATION.replace("raw/confirmations/", "raw/unknown/"),
            "not a declared raw collection",
        ),
        (
            CONFIRMATION.replace("raw/confirmations/", "raw/exports/"),
            "does not take new files",
        ),
        (
            CONFIRMATION.replace("2026-10-17-roles", "2026-10-01-first"),
            "already exists",
        ),
        (
            CONFIRMATION.replace("2026-10-17-roles", "Roles"),
            "must be lowercase",
        ),
        (
            CONFIRMATION.replace("raw/confirmations/2026-10-17-roles.md", "raw/roles.md"),
            "raw/<collection>/<file>",
        ),
        (
            CONFIRMATION.replace(
                "raw/confirmations/2026-10-17-roles.md",
                "raw/confirmations/../x.md",
            ),
            "raw/<collection>/<file>",
        ),
        (
            CONFIRMATION.replace("roles.md", "roles"),
            "with an extension",
        ),
        (
            format!("- {{op: add_raw, path: raw/confirmations/pic.png, base64: '{png}'}}\n"),
            "takes text/markdown, not image/png",
        ),
        (
            CONFIRMATION.replace(
                "  source: Alice, direct statement in session 20261017\n",
                "",
            ),
            "requires the frontmatter source",
        ),
        (
            CONFIRMATION.replace("Bobby still maintains", "^message-roles\n    Again"),
            "more than once",
        ),
        (
            CONFIRMATION.replace("^message-roles", "Roles"),
            "at least one message anchor",
        ),
        (
            CONFIRMATION.replace(
                "^message-roles\n    Bobby",
                "^message-roles\n\n    ### next\n\n    Bobby",
            ),
            "cannot be cited",
        ),
        (
            CONFIRMATION.replace("text: |", "base64: '!!!'\n  text: |"),
            "either `text` or `base64`",
        ),
        (
            "- {op: add_raw, path: raw/confirmations/x.md, source: s}\n".to_string(),
            "either `text` or `base64`",
        ),
        (
            "- {op: add_raw, path: raw/confirmations/x.md, source: s, base64: '!!!'}\n".to_string(),
            "`base64` is invalid",
        ),
        (
            "- {op: add_raw, path: raw/papers/p.txt, text: hello, source: s}\n".to_string(),
            "go in a Markdown file's frontmatter",
        ),
    ];
    for (ops, expected) in &cases {
        let problems = rejected(&wiki, ops);
        assert!(
            problems.contains(expected),
            "{ops}\nexpected `{expected}` in:\n{problems}"
        );
    }
    // The editor sets ingested and sha256 itself; a text cannot bring its own.
    let own = "- op: add_raw\n  path: raw/articles/a.md\n  source_url: https://example.org\n  \
               text: \"---\\ningested: 2020-01-01\\n---\\nBody\\n\"\n";
    assert!(rejected(&wiki, own).contains("set by the editor"));
    let twice = "- op: add_raw\n  path: raw/articles/a.md\n  source_url: https://example.org\n  \
                 text: \"---\\nsource_url: https://other.org\\n---\\nBody\\n\"\n";
    assert!(rejected(&wiki, twice).contains("given twice"));
    // Same path twice in one batch.
    let double = format!("{CONFIRMATION}{CONFIRMATION}");
    assert!(rejected(&wiki, &double).contains("already exists"));
}

#[test]
fn baseline_collections_apply_without_a_declaration() {
    let wiki = fixture();
    let ops = "- op: add_raw\n  path: raw/articles/greenhouse.md\n  source_url: https://example.org/greenhouse\n  \
               text: |\n    # Greenhouse\n\n    Plants like light.\n";
    assert!(wiki.editor().apply(&batch(ops)).unwrap().applied);
    let article = wiki.read("raw/articles/greenhouse.md");
    assert!(article.starts_with(
        "---\nsource_url: https://example.org/greenhouse\ningested: 2026-10-17\nsha256: "
    ));
    // Without the source_url that articles require.
    let wiki = fixture();
    let problems = rejected(
        &wiki,
        &ops.replace("  source_url: https://example.org/greenhouse\n", ""),
    );
    assert!(problems.contains("source_url"), "{problems}");
}

#[test]
fn a_missing_manifest_is_created_and_a_broken_one_is_refused() {
    let wiki = fixture();
    fs::remove_file(wiki.root().join("_meta/raw-manifest.json")).unwrap();
    assert!(wiki.editor().apply(&batch(CONFIRMATION)).unwrap().applied);
    let manifest: serde_json::Value =
        serde_json::from_str(&wiki.read("_meta/raw-manifest.json")).unwrap();
    assert_eq!(manifest["files"].as_array().unwrap().len(), 1);

    let wiki = fixture();
    wiki.write("_meta/raw-manifest.json", "not json");
    assert!(rejected(&wiki, CONFIRMATION).contains("raw manifest"));
}
