mod common;

use common::{Fixture, cite, page};
use leaf_viewer::{SearchRequest, WikiConfig, WikiIndex};
use std::fs;

fn search(
    index: &WikiIndex,
    text: &str,
    passages: usize,
    chars: usize,
) -> leaf_viewer::SearchResponse {
    index
        .search(&SearchRequest::new(text, passages, chars))
        .unwrap()
}

#[test]
fn untrusted_query_text_is_literal() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write("concepts/topic.md", &page("Topic", "River delta survey."));
    let index = wiki.synced();
    for query in [
        "river OR bogus*",
        "\"); DROP TABLE page; --",
        "NEAR(river delta)",
        "title:river",
        "^river",
        "-river",
        "river AND NOT delta",
        "{title heading}: river",
        "rïver ünïcode ✓",
    ] {
        assert!(
            index.search(&SearchRequest::new(query, 3, 1000)).is_ok(),
            "{query}"
        );
    }
    assert_eq!(search(&index, "river", 3, 1000).hits.len(), 1);
    assert_eq!(index.stats().unwrap().pages, 1);
}

#[test]
fn empty_and_punctuation_only_queries_return_nothing() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write("concepts/topic.md", &page("Topic", "River delta survey."));
    let index = wiki.synced();
    for query in ["", "   ", "?!.,;:*\"'()[]{}^-", "·—"] {
        let response = search(&index, query, 3, 1000);
        assert!(response.hits.is_empty(), "{query}");
        assert!(!response.truncated);
        assert_eq!(response.chars_used, 0);
    }
}

#[test]
fn question_shaped_queries_find_relevant_passages() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/greenhouse]]");
    wiki.write(
        "concepts/greenhouse.md",
        &page(
            "Greenhouse bench design",
            "Alice wanted a three-bench Greenhouse layout for seedling trays.\n\n\
             Irrigation timing was discussed separately.",
        ),
    );
    let index = wiki.synced();
    let response = search(
        &index,
        "What did Alice plan for the Greenhouse benches?",
        2,
        2000,
    );
    assert!(!response.hits.is_empty());
    assert!(response.hits[0].text.contains("three-bench"));
}

#[test]
fn body_matches_rank_above_title_heading_and_navigation_links() {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[concepts/greenhouse-cluster]] [[concepts/other]]",
    );
    wiki.write(
        "concepts/greenhouse-cluster.md",
        &page(
            "Greenhouse cluster",
            "## Greenhouse cluster overview\n\nAn unrelated note about weather.\n\n\
             Related: [[concepts/network]].\n\n\
             The Greenhouse cluster was planned, not deployed.",
        ),
    );
    wiki.write(
        "concepts/other.md",
        &page(
            "Other",
            "See the notes in [[concepts/greenhouse-cluster]] about weather and travel.",
        ),
    );
    let index = wiki.synced();
    let response = search(&index, "Greenhouse cluster", 5, 5000);
    assert!(response.hits[0].text.contains("planned"), "{response:?}");
}

#[test]
fn navigation_only_paragraphs_are_not_evidence() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[entities/alice]]");
    wiki.write(
        "entities/alice.md",
        &page(
            "Alice",
            "Related: [[entities/tally|Tally]], [[concepts/zebra-farming]], and [[concepts/build]].\n\n\
             Other thematic pages: [[concepts/zebra-ranching]] and [[concepts/logistics]].\n\n\
             See also: [[concepts/zebra]].\n\n\
             Alice described a zebra mascot for a school project.",
        ),
    );
    let index = wiki.synced();
    let response = search(&index, "zebra", 10, 10_000);
    assert_eq!(response.hits.len(), 1, "{response:?}");
    assert!(response.hits[0].text.contains("mascot"));
    assert_eq!(index.stats().unwrap().passages, 1);
}

#[test]
fn list_items_are_separate_passages_with_their_own_citations() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[entities/alice]]");
    wiki.write(
        "entities/alice.md",
        &page(
            "Alice",
            &format!(
                "## Greenhouse interests, 2023\n\n\
                 - Explored naming for seed catalogue concepts. [[concepts/catalogue]] · {}\n\
                 - Wanted to investigate an automatic vent opener for the cold frame. [[concepts/vent-opener]] · {}\n\
                 - Asked about a drip timer\n  with a pressure sensor. {} · {}\n",
                cite("a.md", "aaa", "Naming"),
                cite("b.md", "bbb", "Printer"),
                cite("c.md", "ccc", "Proxy"),
                cite("c.md", "ddd", "Auth")
            ),
        ),
    );
    let index = wiki.synced();
    assert_eq!(index.stats().unwrap().passages, 3);

    let printer = search(&index, "opener", 5, 10_000);
    assert_eq!(printer.hits.len(), 1);
    let hit = &printer.hits[0];
    assert_eq!(hit.heading, "Greenhouse interests, 2023");
    assert!(!hit.text.contains("catalogue"));
    assert!(!hit.text.contains("drip"));
    assert_eq!(hit.citations.len(), 1);
    assert_eq!(hit.citations[0].anchor.as_ref().unwrap().value(), "bbb");

    let proxy = search(&index, "pressure", 5, 10_000);
    assert_eq!(proxy.hits.len(), 1);
    assert_eq!(
        proxy.hits[0].text,
        "Asked about a drip timer with a pressure sensor."
    );
    let ids: Vec<_> = proxy.hits[0]
        .citations
        .iter()
        .map(|c| c.anchor.as_ref().unwrap().value())
        .collect();
    assert_eq!(ids, ["ccc", "ddd"]);
}

#[test]
fn citation_markup_is_structured_not_searchable_text() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write(
        "concepts/topic.md",
        &page(
            "Topic",
            &format!(
                "River delta survey. {} · {}",
                cite("chatgpt-deadbeef.md", "cafe01", "First"),
                cite("x.md", "cafe02", "Second")
            ),
        ),
    );
    let index = wiki.synced();
    for term in ["chatgpt", "message", "raw", "deadbeef", "cafe01"] {
        assert!(search(&index, term, 3, 1000).hits.is_empty(), "{term}");
    }
    let hit = &search(&index, "river", 3, 1000).hits[0];
    assert_eq!(hit.text, "River delta survey.");
    assert_eq!(hit.citations.len(), 2);
    assert!(hit.is_cited());
}

#[test]
fn uncited_passages_are_marked_as_unverified() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write("concepts/topic.md", &page("Topic", "River delta survey."));
    let index = wiki.synced();
    let hit = &search(&index, "river", 3, 1000).hits[0];
    assert!(hit.citations.is_empty());
    assert!(!hit.is_cited());
}

#[test]
fn passage_and_character_budgets_are_hard_bounds() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    let body: Vec<String> = (0..8)
        .map(|i| {
            format!(
                "River claim number {i}. {}",
                cite("a.md", &format!("m{i}"), "Source")
            )
        })
        .collect();
    wiki.write("concepts/topic.md", &page("Topic", &body.join("\n\n")));
    let index = wiki.synced();

    let capped = search(&index, "river", 3, 100_000);
    assert_eq!(capped.hits.len(), 3);
    assert!(capped.truncated);

    for max_chars in [0, 1, 10, 50, 100, 150, 200, 333, 1000] {
        let response = search(&index, "river", 8, max_chars);
        let measured: usize = response.hits.iter().map(|h| h.char_count()).sum();
        assert_eq!(response.chars_used, measured, "{max_chars}");
        assert!(response.chars_used <= max_chars, "{max_chars}");
        for hit in &response.hits {
            assert_eq!(hit.citations.len(), 1);
            assert!(hit.text.starts_with("River claim number"));
            assert!(hit.text.ends_with('.'));
        }
        if response.hits.len() < 8 {
            assert!(response.truncated, "{max_chars}");
        }
    }

    let tiny = search(&index, "river", 1, 10);
    assert!(tiny.hits.is_empty());
    assert!(tiny.truncated);

    let all = search(&index, "river", 20, 100_000);
    assert_eq!(all.hits.len(), 8);
    assert!(!all.truncated);
}

#[test]
fn oversized_passage_is_skipped_but_smaller_later_ones_fit() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    let long = format!("River river river {}.", "padding ".repeat(60));
    wiki.write(
        "concepts/topic.md",
        &page("Topic", &format!("{long}\n\nShort river note.")),
    );
    let index = wiki.synced();
    let response = search(&index, "river", 2, 120);
    assert_eq!(response.hits.len(), 1);
    assert_eq!(response.hits[0].text, "Short river note.");
    assert!(response.truncated);
}

#[test]
fn char_count_covers_every_returned_string_field() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write(
        "concepts/topic.md",
        &page(
            "Topic",
            &format!("Rïver ✓. {}", cite("a.md", "abc", "Søurce")),
        ),
    );
    let index = wiki.synced();
    let hit = &search(&index, "river", 1, 1000).hits[0];
    let citation = &hit.citations[0];
    let expected = [
        &hit.passage_id,
        &hit.page_path,
        &hit.title,
        &hit.heading,
        &hit.text,
        hit.confidence.as_ref().unwrap(),
        &citation.raw_path,
        &citation.anchor.as_ref().unwrap().value().to_string(),
        &citation.label,
    ]
    .iter()
    .map(|s| s.chars().count())
    .sum::<usize>();
    assert_eq!(hit.char_count(), expected);
}

#[test]
fn ranks_are_ordered_and_contested_flag_is_exposed() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    wiki.write(
        "concepts/topic.md",
        "---\ntitle: Topic\ncontested: true\n---\nRiver river delta.\n\nRiver once.\n",
    );
    let index = wiki.synced();
    let response = search(&index, "river delta", 5, 10_000);
    assert_eq!(response.hits.len(), 2);
    assert_eq!(response.hits[0].rank, 1);
    assert_eq!(response.hits[1].rank, 2);
    assert!(response.hits[0].score >= response.hits[1].score);
    assert!(
        response
            .hits
            .iter()
            .all(|h| h.contested && h.confidence.is_none())
    );
}

#[test]
fn passage_ids_are_stable_across_rebuilds_and_unrelated_edits() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/topic]]");
    let topic = wiki.write(
        "concepts/topic.md",
        &page("Topic", "River delta survey.\n\nSame line.\n\nSame line."),
    );
    let ids = |db: &str| {
        let mut index = WikiIndex::open(WikiConfig::new(&wiki.root, wiki.base.join(db))).unwrap();
        index.sync().unwrap();
        let mut ids: Vec<_> = index
            .search(&SearchRequest::new("river same", 10, 10_000))
            .unwrap()
            .hits
            .into_iter()
            .map(|h| h.passage_id)
            .collect();
        ids.sort();
        ids
    };
    let first = ids("one.db");
    assert_eq!(first.len(), 3);
    assert_eq!(
        first.iter().collect::<std::collections::HashSet<_>>().len(),
        3
    );
    assert_eq!(first, ids("two.db"));
    assert!(first.iter().all(|id| id.starts_with("concepts/topic.md#")));

    let river_id = |ids: &[String], index: &WikiIndex| {
        let hit = index
            .search(&SearchRequest::new("river", 1, 10_000))
            .unwrap()
            .hits
            .remove(0);
        assert!(ids.contains(&hit.passage_id));
        hit.passage_id
    };
    let mut index = WikiIndex::open(WikiConfig::new(&wiki.root, wiki.base.join("one.db"))).unwrap();
    let before = river_id(&first, &index);
    fs::write(
        &topic,
        page("Topic", "A new opening paragraph.\n\nRiver delta survey."),
    )
    .unwrap();
    index.sync().unwrap();
    let after = index
        .search(&SearchRequest::new("river", 1, 10_000))
        .unwrap()
        .hits
        .remove(0)
        .passage_id;
    assert_eq!(before, after);
}
