//! Passage operations end to end: adding, replacing, removing and citing
//! passages by their IDs, on a temporary wiki.

use std::fs;
mod common;

use common::{EditorExt, TempWiki, batch, errors, rejected};
use leaf_editor::Editor;

const A: &str = "raw/notes/a.md#^message-a";
const B: &str = "raw/notes/a.md#^message-b";

fn fixture() -> TempWiki {
    let wiki = TempWiki::new();
    wiki.write(
        "raw/notes/a.md",
        "---\nsource: direct statement\n---\n### user\n\nBeds are raised.\n\n^message-a\n\n\
         ### user\n\nThe soil is dark.\n\n^message-b\n",
    );
    wiki.write("index.md", "[[concepts/garden]]\n");
    wiki.write("log.md", "# Wiki Log\n");
    wiki.write(
        "concepts/garden.md",
        &format!(
            "---\ntitle: Garden\ncreated: 2026-10-01\nupdated: 2026-10-02\ntype: concept\ntags: []\n\
             sources: [raw/notes/a.md]\nconfidence: medium\n---\n# Garden\n\nAn introduction.\n\n\
             ## Beds\n\nBeds are **raised**. [[{A}]]\n\n- Tomatoes grow here. [[{A}]]\n  - Cherry ones too. [[{A}]]\n\n\
             ## Soil\n\nThe soil is dark. [[{B}]]\n"
        ),
    );
    wiki
}

fn page_text(wiki: &TempWiki) -> String {
    fs::read_to_string(wiki.root().join("concepts/garden.md")).unwrap()
}

/// The ID of the passage whose text starts with `text`.
fn id(editor: &Editor, text: &str) -> String {
    editor
        .page("concepts/garden")
        .unwrap()
        .passages
        .into_iter()
        .find(|p| p.text.starts_with(text))
        .unwrap_or_else(|| panic!("no passage `{text}`"))
        .id
}

#[test]
fn a_page_lists_its_headings_and_passage_ids() {
    let wiki = fixture();
    let page = wiki.editor().page("concepts/garden").unwrap();
    assert_eq!(page.title, "Garden");
    assert_eq!(page.headings, ["Garden", "Beds", "Soil"]);
    let texts: Vec<_> = page.passages.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "An introduction.",
            "Beds are raised.",
            "Tomatoes grow here.",
            "Cherry ones too.",
            "The soil is dark."
        ]
    );
    assert!(page.passages[1].id.starts_with("concepts/garden.md#p-"));
    assert_eq!(page.passages[4].citations, [B]);
    assert!(wiki.editor().page("concepts/missing").is_err());
}

#[test]
fn passages_are_added_under_headings_or_after_passages() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let soil = id(&editor, "The soil");
    let untouched = id(&editor, "Tomatoes");
    let outcome = editor
        .apply(&batch(&format!(
            "- {{op: add_passage, page: concepts/garden, under: beds, text: 'Beds face south, see [[concepts/garden|the garden]].', citations: ['{A}']}}\n\
             - {{op: add_passage, page: concepts/garden, under: Garden, text: A second introduction line.}}\n\
             - {{op: add_passage, page: concepts/garden, after: '{soil}', text: Worms help., citations: ['[[{B}|Note]]']}}\n"
        )))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    let page = page_text(&wiki);
    assert!(page.contains(&format!(
        "  - Cherry ones too. [[{A}]]\n\nBeds face south, see [[concepts/garden|the garden]]. [[{A}]]\n\n## Soil"
    )), "{page}");
    assert!(
        page.contains("An introduction.\n\nA second introduction line.\n\n## Beds"),
        "{page}"
    );
    assert!(
        page.ends_with(&format!(
            "The soil is dark. [[{B}]]\n\nWorms help. [[{B}|Note]]\n"
        )),
        "{page}"
    );
    // New passages are reported by ID, and other IDs are unchanged.
    assert_eq!(outcome.passages.len(), 3);
    let view = editor.page("concepts/garden").unwrap();
    for added in &outcome.passages {
        assert!(view.passages.iter().any(|p| p.id == added.id), "{added:?}");
    }
    assert_eq!(id(&editor, "Tomatoes"), untouched);
    assert_eq!(id(&editor, "The soil"), soil);
}

#[test]
fn bad_additions_are_rejected_untouched() {
    let wiki = fixture();
    let cases = [
        // Uncited outside the introduction is a new problem.
        (
            "- {op: add_passage, page: concepts/garden, under: Soil, text: An uncited claim.}\n",
            "uncited passage(s) under \"Soil\"",
        ),
        (
            "- {op: add_passage, page: concepts/garden, under: Roots, text: X., citations: ['raw/notes/a.md']}\n",
            "no single heading `Roots`",
        ),
        (
            "- {op: add_passage, page: concepts/garden, under: Soil, text: X., citations: ['raw/notes/a.md#^message-zzz']}\n",
            "anchor not found",
        ),
        (
            "- {op: add_passage, page: concepts/garden, under: Soil, text: X., citations: ['raw/notes/missing.md']}\n",
            "citation `raw/notes/missing.md`",
        ),
        (
            "- {op: add_passage, page: concepts/garden, under: Soil, text: Some **bold** claim., citations: ['raw/notes/a.md']}\n",
            "plain paragraph text",
        ),
        (
            "- {op: add_passage, page: concepts/garden, under: Soil, text: '- a list item', citations: ['raw/notes/a.md']}\n",
            "plain paragraph text",
        ),
        (
            "- {op: add_passage, page: concepts/garden, under: Soil, text: 'Cites [[raw/notes/a.md]] inline.'}\n",
            "put citations in `citations`",
        ),
        (
            "- {op: add_passage, page: concepts/garden, text: Nowhere., citations: ['raw/notes/a.md']}\n",
            "either `under`",
        ),
    ];
    for (ops, expected) in cases {
        let message = rejected(&wiki, ops);
        assert!(
            message.contains(expected),
            "{ops}\nexpected `{expected}` in:\n{message}"
        );
    }
    let heading_suggestions = rejected(
        &wiki,
        "- {op: add_passage, page: concepts/garden, under: Roots, text: X., citations: ['raw/notes/a.md']}\n",
    );
    assert!(
        heading_suggestions.contains("[\"Garden\", \"Beds\", \"Soil\"]"),
        "{heading_suggestions}"
    );
}

#[test]
fn passages_are_limited_in_length() {
    let wiki = fixture();
    wiki.write(
        "_meta/vocabulary.yaml",
        "limits: {ops_per_batch: 50, passage_chars: 20}\n",
    );
    let message = rejected(
        &wiki,
        &format!(
            "- {{op: add_passage, page: concepts/garden, under: Soil, text: A claim that is far too long., citations: ['{B}']}}\n"
        ),
    );
    assert!(message.contains("the limit is 20"), "{message}");
}

#[test]
fn replacing_cannot_drop_citations_silently() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let soil = id(&editor, "The soil");
    let message = rejected(
        &wiki,
        &format!(
            "- {{op: replace_passage, passage: '{soil}', text: The soil is very dark., citations: ['{A}']}}\n"
        ),
    );
    assert!(
        message.contains(&format!("would drop the citations {B}")),
        "{message}"
    );
    let outcome = editor
        .apply(&batch(&format!(
            "- {{op: replace_passage, passage: '{soil}', text: The soil is very dark., citations: ['{A}'], drop_citations: ['{B}']}}\n"
        )))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert!(page_text(&wiki).contains(&format!("## Soil\n\nThe soil is very dark. [[{A}]]\n")));
    assert_eq!(outcome.passages[0].id, id(&editor, "The soil"));
    // The old ID is now stale.
    let stale = rejected(
        &wiki,
        &format!(
            "- {{op: replace_passage, passage: '{soil}', text: Again., citations: ['{A}']}}\n"
        ),
    );
    assert!(
        stale.contains("not found: the page may have changed"),
        "{stale}"
    );
}

#[test]
fn replacing_a_list_item_keeps_the_list() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let tomatoes = id(&editor, "Tomatoes");
    let outcome = editor
        .apply(&batch(&format!(
            "- {{op: replace_passage, passage: '{tomatoes}', text: Tomatoes grow well here., citations: ['{A}']}}\n"
        )))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert!(
        page_text(&wiki).contains(&format!(
            "- Tomatoes grow well here. [[{A}]]\n  - Cherry ones too. [[{A}]]\n"
        )),
        "{}",
        page_text(&wiki)
    );
}

#[test]
fn removing_needs_a_reason_and_no_nested_items() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let tomatoes = id(&editor, "Tomatoes");
    let cherry = id(&editor, "Cherry");
    assert!(
        rejected(
            &wiki,
            &format!("- {{op: remove_passage, passage: '{cherry}', reason: ' '}}\n")
        )
        .contains("reason")
    );
    assert!(
        rejected(
            &wiki,
            &format!("- {{op: remove_passage, passage: '{tomatoes}', reason: Wrong.}}\n")
        )
        .contains("nested items")
    );
    let outcome = editor
        .apply(&batch(&format!(
            "- {{op: remove_passage, passage: '{cherry}', reason: Not supported by the source.}}\n\
             - {{op: remove_passage, passage: '{tomatoes}', reason: Duplicate.}}\n"
        )))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    let page = page_text(&wiki);
    assert!(
        page.contains(&format!("Beds are **raised**. [[{A}]]\n\n## Soil")),
        "{page}"
    );
    let log = fs::read_to_string(wiki.root().join("log.md")).unwrap();
    assert!(log.contains(&format!(
        "- Removed passages: `{cherry}` (Not supported by the source.); `{tomatoes}` (Duplicate.).\n"
    )), "{log}");
}

#[test]
fn adding_a_citation_keeps_the_passage_as_written() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let beds = id(&editor, "Beds are");
    assert!(
        rejected(
            &wiki,
            &format!("- {{op: add_citation, passage: '{beds}', citation: '{A}'}}\n")
        )
        .contains("already cites")
    );
    let outcome = editor
        .apply(&batch(&format!(
            "- {{op: add_citation, passage: '{beds}', citation: '{B}'}}\n"
        )))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert!(
        page_text(&wiki).contains(&format!("Beds are **raised**. [[{A}]] [[{B}]]\n")),
        "{}",
        page_text(&wiki)
    );
    let view = editor.page("concepts/garden").unwrap();
    let cited = view
        .passages
        .iter()
        .find(|p| p.id == outcome.passages[0].id)
        .unwrap();
    assert_eq!(cited.citations, [A, B]);
}
