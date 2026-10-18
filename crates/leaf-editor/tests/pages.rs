//! Page operations end to end on a temporary wiki: creating pages, titles
//! and summaries, and the `index.md` and `log.md` bookkeeping.

mod common;

use common::{EditorExt, TempWiki, batch, errors, rejected};
use leaf_viewer::{SearchRequest, WikiConfig, WikiIndex};

const A: &str = "raw/notes/a.md#^message-a";

const INDEX: &str = "# Wiki Index\n\n> Last updated: 2026-10-01 | Total pages: 3\n\n\
    ## Concepts\n\n- [[concepts/beds|Beds]] — Raised beds\n- [[concepts/soil|Soil]] — Dark soil\n\n\
    ## Entities\n\n- [[entities/alice|Alice]] — A gardener\n";

fn fixture() -> TempWiki {
    let wiki = TempWiki::new();
    wiki.write(
        "raw/notes/a.md",
        "---\nsource: direct statement\n---\n### user\n\nBeds are raised.\n\n^message-a\n",
    );
    wiki.write("index.md", INDEX);
    wiki.write("log.md", "# Wiki Log\n");
    wiki.write(
        "_meta/vocabulary.yaml",
        "labels:\n  tags: {gardening: {}, soil: {}}\n",
    );
    let page = |title: &str, kind: &str, extra: &str, body: &str| {
        format!(
            "---\ntitle: {title}\ntype: {kind}\ncreated: 2026-10-01\nupdated: 2026-10-02\n\
             tags: [gardening]\nsources: [raw/notes/a.md]\nconfidence: medium\n{extra}---\n{body}"
        )
    };
    wiki.write(
        "concepts/beds.md",
        &page(
            "Beds",
            "concept",
            "",
            &format!(
                "# Beds\n\nIntro with [[concepts/soil|soil]] and [[entities/alice|Alice]].\n\n\
                 ## Layout\n\nBeds are raised. [[{A}]]\n"
            ),
        ),
    );
    wiki.write(
        "concepts/soil.md",
        &page(
            "Soil",
            "concept",
            "",
            &format!(
                "# Soil\n\nSee [[concepts/beds|beds]] and [[entities/alice|Alice]].\n\n\
                 ## Notes\n\nSoil is dark. [[{A}]]\n"
            ),
        ),
    );
    wiki.write(
        "entities/alice.md",
        &page(
            "Alice",
            "entity",
            "related-people: [alice]\n",
            &format!(
                "# Alice\n\nAlice keeps [[concepts/beds|beds]] and [[concepts/soil|soil]].\n\n\
                 ## Work\n\nAlice gardens. [[{A}]]\n"
            ),
        ),
    );
    wiki
}

/// What a fresh viewer reads: the sync report's problems, if any.
fn sync_problems(wiki: &TempWiki) -> (usize, usize) {
    let mut index = WikiIndex::open(WikiConfig::new(wiki.root(), wiki.path("fresh.db"))).unwrap();
    let report = index.sync().unwrap();
    (report.errors.len(), report.warnings.len())
}

const COMPOST: &str = "- op: create_page\n  \
    type: concept\n  slug: compost\n  title: Compost\n  summary: Turning waste into soil\n  \
    metadata: {tags: [soil], confidence: medium}\n  \
    intro:\n    - {text: 'Compost feeds [[concepts/soil|the soil]] of [[concepts/beds|the beds]].'}\n  \
    sections:\n    - heading: Method\n      passages:\n        \
    - {text: Heaps are turned weekly., citations: ['raw/notes/a.md#^message-a']}\n";

#[test]
fn a_page_is_created_with_its_frontmatter_index_line_and_log_entry() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let outcome = editor.apply(&batch(COMPOST)).unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(
        outcome.changed,
        ["concepts/compost.md", "index.md", "log.md"]
    );
    assert_eq!(outcome.passages.len(), 2);
    assert!(outcome.passages[0].id.starts_with("concepts/compost.md#p-"));

    assert_eq!(
        wiki.read("concepts/compost.md"),
        "---\ntitle: Compost\ntype: concept\ncreated: 2026-10-17\nupdated: 2026-10-17\n\
         tags: [soil]\nsources: [raw/notes/a.md]\nconfidence: medium\n---\n\
         # Compost\n\nCompost feeds [[concepts/soil|the soil]] of [[concepts/beds|the beds]].\n\n\
         ## Method\n\nHeaps are turned weekly. [[raw/notes/a.md#^message-a]]\n"
    );
    let index = wiki.read("index.md");
    assert!(index.contains("> Last updated: 2026-10-17 | Total pages: 4\n"));
    let concepts: Vec<&str> = index
        .lines()
        .filter(|l| l.starts_with("- [[concepts/"))
        .collect();
    assert_eq!(
        concepts,
        [
            "- [[concepts/beds|Beds]] — Raised beds",
            "- [[concepts/compost|Compost]] — Turning waste into soil",
            "- [[concepts/soil|Soil]] — Dark soil",
        ]
    );
    let log = wiki.read("log.md");
    assert!(log.contains("## [2026-10-17] update | Test batch\n"));
    assert!(log.contains("- Created: `concepts/compost.md`.\n"));
    assert!(log.contains("- Changed: `index.md`.\n"));

    // The viewer reads the new page without a problem, and finds it.
    let sync = outcome.sync.unwrap();
    assert!(sync.is_complete() && sync.warnings.is_empty());
    assert_eq!(sync_problems(&wiki), (0, 0));
    let hits = editor
        .index()
        .search(&SearchRequest::new("heaps turned", 3, 1000))
        .unwrap()
        .hits;
    assert_eq!(hits[0].page_path, "concepts/compost.md");
    // The reported IDs address the new passages.
    assert_eq!(
        editor.page("concepts/compost").unwrap().passages[1].id,
        outcome.passages[1].id
    );
}

#[test]
fn a_new_page_is_rejected_unless_it_is_complete() {
    let wiki = fixture();
    let create = |extra: &str| {
        format!(
            "- op: create_page\n  type: concept\n  slug: compost\n  title: Compost\n  summary: S\n  \
             metadata: {{tags: [soil], confidence: medium}}\n  \
             intro: [{{text: 'See [[concepts/soil]] and [[concepts/beds]].'}}]\n{extra}"
        )
    };
    let cases: [(String, &str); 10] = [
        (
            COMPOST.replace("type: concept", "type: recipe"),
            "not a page type",
        ),
        (
            COMPOST.replace("slug: compost", "slug: Compost Heap"),
            "not a slug",
        ),
        (
            COMPOST.replace("slug: compost", "slug: beds"),
            "already exists",
        ),
        (
            COMPOST.replace(", confidence: medium", ""),
            "`confidence` is required",
        ),
        (
            COMPOST.replace("tags: [soil]", "tags: [compostable]"),
            "unknown value `compostable`",
        ),
        (
            COMPOST.replace("[[concepts/beds|the beds]]", "[[concepts/missing|beds]]"),
            "concepts/missing.md",
        ),
        (
            COMPOST.replace(" of [[concepts/beds|the beds]]", ""),
            "1 link(s) to other pages",
        ),
        (
            COMPOST.replace(", citations: ['raw/notes/a.md#^message-a']", ""),
            "uncited passage",
        ),
        (
            COMPOST.replace(
                "text: Heaps are turned weekly.,",
                "text: 'Heaps are turned weekly. [[raw/nowhere.md]]',",
            ),
            "put citations in `citations`",
        ),
        (
            create("  sections:\n    - {heading: Compost, passages: []}\n"),
            "used twice",
        ),
    ];
    for (ops, expected) in &cases {
        let problems = rejected(&wiki, ops);
        assert!(problems.contains(expected), "{expected}: {problems}");
    }
    // The editor sets the dates and type itself.
    let problems = rejected(
        &wiki,
        &COMPOST.replace(
            "confidence: medium",
            "confidence: medium, created: 2020-01-01",
        ),
    );
    assert!(problems.contains("set by the editor"), "{problems}");
    // Creating the same page twice in one batch.
    let twice = format!("{COMPOST}{COMPOST}");
    assert!(rejected(&wiki, &twice).contains("already exists"));
}

#[test]
fn sources_are_derived_from_citations_and_merged_with_given_ones() {
    let wiki = fixture();
    wiki.write("raw/notes/b.md", "---\nsource: x\n---\nText.\n");
    let ops = COMPOST.replace(
        "confidence: medium}",
        "confidence: medium, sources: [raw/notes/b.md]}",
    );
    assert!(wiki.editor().apply(&batch(&ops)).unwrap().applied);
    assert!(
        wiki.read("concepts/compost.md")
            .contains("sources: [raw/notes/b.md, raw/notes/a.md]\n")
    );
}

#[test]
fn people_created_in_a_batch_can_be_used_later_in_it() {
    let wiki = fixture();
    let person = |slug: &str, title: &str, metadata: &str| {
        format!(
            "- op: create_page\n  type: entity\n  slug: {slug}\n  title: {title}\n  summary: A gardener\n  \
             metadata: {{tags: [gardening], confidence: low, {metadata}}}\n  \
             intro: [{{text: 'Knows [[concepts/beds]] and [[concepts/soil]].'}}]\n"
        )
    };
    let ops = format!(
        "{}{}",
        person("bob-martin", "Bob Martin", "related-people: [bob-martin]"),
        person(
            "chloe-dubois",
            "Chloé Dubois",
            "relationship.bob-martin: [colleague]"
        ),
    );
    let mut editor = wiki.editor();
    let outcome = editor.apply(&batch(&ops)).unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    // The symmetric label is mirrored onto the other new page.
    assert!(
        wiki.read("entities/bob-martin.md")
            .contains("relationship:\n  chloe-dubois: [colleague]\n")
    );
    assert!(
        wiki.read("entities/chloe-dubois.md")
            .contains("relationship:\n  bob-martin: [colleague]\n")
    );
    assert_eq!(sync_problems(&wiki), (0, 0));
    // A person that is only mentioned in a batch is not accepted.
    let wiki = fixture();
    let problems = rejected(
        &wiki,
        &person(
            "chloe-dubois",
            "Chloé Dubois",
            "relationship.bob-martin: [colleague]",
        ),
    );
    assert!(problems.contains("bob-martin"), "{problems}");
}

#[test]
fn a_title_change_follows_the_top_heading_and_the_index_line() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let outcome = editor
        .apply(&batch(
            "- {op: set_title, page: concepts/beds, title: Raised beds}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(outcome.changed, ["concepts/beds.md", "index.md", "log.md"]);
    let page = wiki.read("concepts/beds.md");
    assert!(page.contains("title: Raised beds\n"));
    assert!(page.contains("updated: 2026-10-17\n"));
    assert!(page.contains("\n# Raised beds\n\nIntro with"));
    assert!(
        wiki.read("index.md")
            .contains("- [[concepts/beds|Raised beds]] — Raised beds\n")
    );
    // Passages under the renamed heading have new IDs; they are reported.
    let now = editor.page("concepts/beds").unwrap();
    let reported: Vec<&str> = outcome.passages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(reported, [now.passages[0].id.as_str()]);
    assert_eq!(sync_problems(&wiki), (0, 0));

    assert!(
        rejected(
            &wiki,
            "- {op: set_title, page: concepts/beds, title: Raised beds}\n"
        )
        .contains("already titled")
    );
    assert!(
        rejected(
            &wiki,
            "- {op: set_title, page: concepts/beds, title: '**Beds**'}\n"
        )
        .contains("plain text")
    );
}

#[test]
fn a_summary_change_only_touches_the_index() {
    let wiki = fixture();
    let before = wiki.read("concepts/soil.md");
    let outcome = wiki
        .editor()
        .apply(&batch(
            "- {op: set_summary, page: concepts/soil, summary: Where roots live}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(outcome.changed, ["index.md", "log.md"]);
    assert_eq!(wiki.read("concepts/soil.md"), before);
    assert!(
        wiki.read("index.md")
            .contains("- [[concepts/soil|Soil]] — Where roots live\n")
    );
    assert!(
        rejected(
            &wiki,
            "- {op: set_summary, page: concepts/missing, summary: x}\n"
        )
        .contains("not a published page")
    );
    assert!(
        rejected(
            &wiki,
            "- {op: set_summary, page: concepts/soil, summary: ''}\n"
        )
        .contains("empty")
    );
}

#[test]
fn an_index_line_that_lists_several_pages_is_not_rewritten() {
    let wiki = fixture();
    wiki.write(
        "index.md",
        "[[concepts/beds]] [[concepts/soil]] [[entities/alice]]\n",
    );
    let problems = rejected(
        &wiki,
        "- {op: set_summary, page: concepts/soil, summary: x}\n",
    );
    assert!(problems.contains("several pages"), "{problems}");
}

#[test]
fn headings_are_added_where_asked_and_can_take_passages() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let outcome = editor
        .apply(&batch(&format!(
            "- {{op: add_heading, page: concepts/beds, heading: Watering}}\n\
             - {{op: add_heading, page: concepts/beds, heading: Corners, level: 3, after: layout}}\n\
             - {{op: add_passage, page: concepts/beds, under: watering, text: Water at dawn., citations: ['{A}']}}\n\
             - {{op: add_passage, page: concepts/beds, under: corners, text: Corners dry first., citations: ['{A}']}}\n"
        )))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(
        wiki.read("concepts/beds.md")
            .split("\n---\n")
            .nth(1)
            .unwrap(),
        format!(
            "# Beds\n\nIntro with [[concepts/soil|soil]] and [[entities/alice|Alice]].\n\n\
             ## Layout\n\nBeds are raised. [[{A}]]\n\n### Corners\n\nCorners dry first. [[{A}]]\n\n\
             ## Watering\n\nWater at dawn. [[{A}]]\n"
        )
    );
    assert_eq!(sync_problems(&wiki), (0, 0));
    assert_eq!(
        editor.page("concepts/beds").unwrap().headings,
        ["Beds", "Layout", "Corners", "Watering"]
    );
}

#[test]
fn a_renamed_heading_reports_the_new_ids_of_its_passages() {
    let wiki = fixture();
    let mut editor = wiki.editor();
    let before = editor.page("concepts/beds").unwrap();
    let outcome = editor
        .apply(&batch(
            "- {op: rename_heading, page: concepts/beds, heading: layout, to: Arrangement}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    let after = editor.page("concepts/beds").unwrap();
    assert_eq!(after.headings, ["Beds", "Arrangement"]);
    // The introduction keeps its ID; the passage under the heading has a new one.
    assert_eq!(after.passages[0].id, before.passages[0].id);
    assert_ne!(after.passages[1].id, before.passages[1].id);
    let reported: Vec<&str> = outcome.passages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(reported, [after.passages[1].id.as_str()]);
    assert!(wiki.read("concepts/beds.md").contains("\n## Arrangement\n"));
}

#[test]
fn heading_operations_reject_what_would_not_read_back() {
    let wiki = fixture();
    for (ops, expected) in [
        (
            "- {op: add_heading, page: concepts/beds, heading: Layout}\n",
            "already has a heading",
        ),
        (
            "- {op: add_heading, page: concepts/beds, heading: Beds}\n",
            "already has a heading",
        ),
        (
            "- {op: add_heading, page: concepts/beds, heading: X, level: 1}\n",
            "2 to 6",
        ),
        (
            "- {op: add_heading, page: concepts/beds, heading: X, after: nowhere}\n",
            "no single heading",
        ),
        (
            "- {op: add_heading, page: concepts/beds, heading: '**X**'}\n",
            "plain text",
        ),
        (
            "- {op: rename_heading, page: concepts/beds, heading: Beds, to: Raised}\n",
            "use set_title",
        ),
        (
            "- {op: rename_heading, page: concepts/beds, heading: Layout, to: Beds}\n",
            "already has a heading",
        ),
        (
            "- {op: rename_heading, page: concepts/beds, heading: Layout, to: Layout}\n",
            "already",
        ),
        (
            "- {op: rename_heading, page: concepts/beds, heading: Nowhere, to: X}\n",
            "no single heading",
        ),
    ] {
        let problems = rejected(&wiki, ops);
        assert!(problems.contains(expected), "{ops}: {problems}");
    }
}

/// The wiki plus an unreferenced note and a person Alice has a relationship with.
fn with_extra_pages() -> TempWiki {
    let wiki = fixture();
    wiki.write(
        "index.md",
        &INDEX
            .replace("Total pages: 3", "Total pages: 5")
            .replace(
                "- [[concepts/soil|Soil]] — Dark soil\n",
                "- [[concepts/old-notes|Old notes]] — Superseded\n- [[concepts/soil|Soil]] — Dark soil\n",
            )
            .replace(
                "- [[entities/alice|Alice]] — A gardener\n",
                "- [[entities/alice|Alice]] — A gardener\n- [[entities/bob-martin|Bob Martin]] — A neighbor\n",
            ),
    );
    wiki.write(
        "concepts/old-notes.md",
        &format!(
            "---\ntitle: Old notes\ntype: concept\ncreated: 2026-10-01\nupdated: 2026-10-02\n\
             tags: [gardening]\nsources: [raw/notes/a.md]\nconfidence: low\n---\n\
             # Old notes\n\nSee [[concepts/beds|beds]] and [[concepts/soil|soil]].\n\n\
             ## Notes\n\nBeds were raised. [[{A}]]\n"
        ),
    );
    wiki.write(
        "entities/bob-martin.md",
        &format!(
            "---\ntitle: Bob Martin\ntype: entity\ncreated: 2026-10-01\nupdated: 2026-10-02\n\
             tags: [gardening]\nsources: [raw/notes/a.md]\nconfidence: low\n\
             related-people: [bob-martin]\nrelationship:\n  alice: [friend]\n---\n\
             # Bob Martin\n\nKnows [[concepts/beds|beds]] and [[concepts/soil|soil]].\n\n\
             ## Work\n\nBob digs. [[{A}]]\n"
        ),
    );
    let alice = wiki.read("entities/alice.md").replace(
        "related-people: [alice]\n",
        "related-people: [alice]\nrelationship:\n  bob-martin: [friend]\n",
    );
    wiki.write("entities/alice.md", &alice);
    wiki
}

#[test]
fn an_unreferenced_page_is_archived_and_restored() {
    let wiki = with_extra_pages();
    let original = wiki.read("concepts/old-notes.md");
    let mut editor = wiki.editor();
    let outcome = editor
        .apply(&batch(
            "- {op: archive_page, page: concepts/old-notes, reason: Superseded by the beds page}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(
        outcome.changed,
        ["_archive/concepts/old-notes.md", "index.md", "log.md"]
    );
    assert_eq!(outcome.removed, ["concepts/old-notes.md"]);
    assert!(!wiki.root().join("concepts/old-notes.md").exists());
    let archived = wiki.read("_archive/concepts/old-notes.md");
    assert!(archived.contains("updated: 2026-10-17\n"));
    assert!(archived.contains(
        "confidence: low\narchived: 2026-10-17\narchived-reason: Superseded by the beds page\n\
         archived-summary: Superseded\n---\n"
    ));
    let index = wiki.read("index.md");
    assert!(!index.contains("old-notes"));
    assert!(index.contains("Total pages: 4\n"));
    assert!(wiki.read("log.md").contains(
        "- Archived: `concepts/old-notes.md` to `_archive/concepts/old-notes.md` (Superseded by the beds page).\n"
    ));
    assert_eq!(sync_problems(&wiki), (0, 0));
    assert!(editor.page("concepts/old-notes").is_err());

    // Restoring brings it back, on index.md again with its old summary.
    let outcome = editor
        .apply(&batch(
            "- {op: restore_page, page: _archive/concepts/old-notes.md}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert_eq!(outcome.removed, ["_archive/concepts/old-notes.md"]);
    assert!(!wiki.root().join("_archive/concepts/old-notes.md").exists());
    assert_eq!(
        wiki.read("concepts/old-notes.md"),
        original.replace("updated: 2026-10-02", "updated: 2026-10-17")
    );
    assert!(
        wiki.read("index.md")
            .contains("- [[concepts/old-notes|Old notes]] — Superseded\n")
    );
    assert!(
        wiki.read("log.md")
            .contains("- Restored: `_archive/concepts/old-notes.md` to `concepts/old-notes.md`.\n")
    );
    assert_eq!(sync_problems(&wiki), (0, 0));
}

#[test]
fn a_referenced_page_is_archived_only_if_the_batch_removes_the_references() {
    let wiki = with_extra_pages();
    // A link from other pages.
    let problems = rejected(
        &wiki,
        "- {op: archive_page, page: concepts/soil, reason: Merged}\n",
    );
    for referrer in [
        "concepts/beds.md",
        "entities/alice.md",
        "concepts/old-notes.md",
    ] {
        assert!(problems.contains(referrer), "{problems}");
    }
    assert!(problems.contains("a link"), "{problems}");
    // Metadata that names a person.
    let problems = rejected(
        &wiki,
        "- {op: archive_page, page: entities/bob-martin, reason: Moved away}\n",
    );
    assert!(problems.contains("entities/alice.md"), "{problems}");
    assert!(problems.contains("`relationship.bob-martin`"), "{problems}");

    // Removing the references in the same batch makes it acceptable.
    let mut editor = wiki.editor();
    let outcome = editor
        .apply(&batch(
            "- {op: remove_metadata, page: entities/alice, key: relationship.bob-martin}\n\
             - {op: archive_page, page: entities/bob-martin, reason: Moved away}\n",
        ))
        .unwrap();
    assert!(outcome.applied, "{}", errors(&outcome));
    assert!(
        wiki.root()
            .join("_archive/entities/bob-martin.md")
            .is_file()
    );
    assert!(!wiki.read("entities/alice.md").contains("bob-martin"));
    assert_eq!(sync_problems(&wiki), (0, 0));
    // Archived people are no longer accepted.
    let problems = rejected(
        &wiki,
        "- {op: add_metadata, page: entities/alice, key: relationship.bob-martin, values: [friend]}\n",
    );
    assert!(problems.contains("bob-martin"), "{problems}");
}

#[test]
fn archive_and_restore_reject_what_does_not_apply() {
    let wiki = with_extra_pages();
    for (ops, expected) in [
        (
            "- {op: archive_page, page: concepts/old-notes, reason: ''}\n",
            "reason",
        ),
        (
            "- {op: archive_page, page: concepts/missing, reason: x}\n",
            "not a published page",
        ),
        (
            "- {op: restore_page, page: concepts/old-notes}\n",
            "not an archived page",
        ),
    ] {
        let problems = rejected(&wiki, ops);
        assert!(problems.contains(expected), "{ops}: {problems}");
    }
    // Archived in one batch, restored in the next: not within the same one.
    let problems = rejected(
        &wiki,
        "- {op: archive_page, page: concepts/old-notes, reason: x}\n\
         - {op: restore_page, page: _archive/concepts/old-notes.md}\n",
    );
    assert!(problems.contains("not an archived page"), "{problems}");
    // A restore needs a free destination, a kept or given summary, and a valid page.
    wiki.write(
        "_archive/concepts/broken.md",
        "---\ntitle: Broken\ntype: concept\ncreated: 2026-10-01\nupdated: 2026-10-02\ntags: [gardening]\n\
         sources: [raw/notes/a.md]\nconfidence: low\narchived: 2026-10-05\narchived-reason: old\n---\n\
         # Broken\n\nLinks [[concepts/missing]] and [[concepts/beds]].\n",
    );
    let problems = rejected(&wiki, "- {op: restore_page, page: concepts/broken}\n");
    assert!(problems.contains("kept no summary"), "{problems}");
    let problems = rejected(
        &wiki,
        "- {op: restore_page, page: concepts/broken, summary: A page}\n",
    );
    assert!(problems.contains("concepts/missing.md"), "{problems}");
    wiki.write("concepts/broken.md", "taken");
    let problems = rejected(
        &wiki,
        "- {op: restore_page, page: concepts/broken, summary: A page}\n",
    );
    assert!(problems.contains("already exists"), "{problems}");
}
