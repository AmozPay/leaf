mod common;

use common::Fixture;
use leaf_viewer::{FacetCount, FacetMatch, SearchRequest, SearchResponse, WikiIndex};

fn tagged(title: &str, tags: &str, body: &str) -> String {
    format!("---\ntitle: {title}\ntags: {tags}\n---\n# {title}\n\n{body}\n")
}

/// a: software + dns, b: software, c: business, d: untagged.
fn fixture() -> (Fixture, WikiIndex) {
    let wiki = Fixture::new();
    wiki.write(
        "index.md",
        "[[concepts/a]] [[concepts/b]] [[concepts/c]] [[concepts/d]]",
    );
    wiki.write(
        "concepts/a.md",
        &tagged("A", "[Software, dns]", "Alpha orchard claim."),
    );
    wiki.write(
        "concepts/b.md",
        &tagged(
            "B",
            "['#software']",
            "Beta orchard claim.\n\nBeta second note.",
        ),
    );
    wiki.write(
        "concepts/c.md",
        &tagged("C", "business", "Gamma orchard claim."),
    );
    wiki.write(
        "concepts/d.md",
        "---\ntitle: D\n---\nDelta orchard claim.\n",
    );
    let index = wiki.synced();
    (wiki, index)
}

/// Distinct page paths of the hits, sorted.
fn pages(response: &SearchResponse) -> Vec<String> {
    let mut pages: Vec<_> = response.hits.iter().map(|h| h.page_path.clone()).collect();
    pages.sort();
    pages.dedup();
    pages
}

fn filtered(index: &WikiIndex, query: &str, tags: &[&str], mode: FacetMatch) -> SearchResponse {
    let request = SearchRequest::new(query, 20, 100_000).with_tags(tags.iter().copied(), mode);
    index.search(&request).unwrap()
}

#[test]
fn all_requires_every_tag_and_any_requires_one() {
    let (_wiki, index) = fixture();
    let all = filtered(&index, "orchard", &["software", "dns"], FacetMatch::All);
    assert_eq!(pages(&all), ["concepts/a.md"]);

    let any = pages(&filtered(
        &index,
        "orchard",
        &["dns", "business"],
        FacetMatch::Any,
    ))
    .to_vec();
    assert_eq!(any, ["concepts/a.md", "concepts/c.md"]);

    let software = pages(&filtered(&index, "orchard", &["software"], FacetMatch::All));
    assert_eq!(software, ["concepts/a.md", "concepts/b.md"]);

    // A tag nobody has: nothing for `all`, ignored members for `any`.
    assert!(
        filtered(&index, "orchard", &["software", "nope"], FacetMatch::All)
            .hits
            .is_empty()
    );
    assert_eq!(
        pages(&filtered(
            &index,
            "orchard",
            &["business", "nope"],
            FacetMatch::Any
        )),
        ["concepts/c.md"]
    );
}

#[test]
fn tags_are_case_insensitive_and_deduplicated() {
    let (_wiki, index) = fixture();
    let shouty = filtered(
        &index,
        "orchard",
        &["#SOFTWARE", " Dns ", "dns"],
        FacetMatch::All,
    );
    assert_eq!(pages(&shouty), ["concepts/a.md"]);
    assert_eq!(shouty.hits[0].tags, ["dns", "software"]);
}

#[test]
fn tags_without_a_query_list_passages_in_order() {
    let (_wiki, index) = fixture();
    let listed = filtered(&index, "", &["software"], FacetMatch::All);
    let texts: Vec<_> = listed.hits.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Alpha orchard claim.",
            "Beta orchard claim.",
            "Beta second note."
        ]
    );
    assert!(listed.hits.iter().all(|h| h.score == 0.0));
    let ranks: Vec<_> = listed.hits.iter().map(|h| h.rank).collect();
    assert_eq!(ranks, [1, 2, 3]);

    // Neither a query nor tags: nothing to search.
    assert!(
        filtered(&index, " ?! ", &[" ", "#"], FacetMatch::Any)
            .hits
            .is_empty()
    );
}

#[test]
fn tag_filters_respect_limits_and_count_tags_in_the_budget() {
    let (_wiki, index) = fixture();
    let request = SearchRequest::new("", 2, 100_000).with_tags(["software"], FacetMatch::All);
    let limited = index.search(&request).unwrap();
    assert_eq!(limited.hits.len(), 2);
    assert!(limited.truncated);

    let hit = &limited.hits[0];
    let without_tags = hit.char_count() - "dns".len() - "software".len();
    let mut untagged = hit.clone();
    untagged.tags.clear();
    assert_eq!(untagged.char_count(), without_tags);
}

#[test]
fn tag_hostile_input_is_a_literal_value() {
    let (_wiki, index) = fixture();
    for hostile in ["'); DROP TABLE page; --", "%", "_", "*", "software OR 1=1"] {
        assert!(
            filtered(&index, "orchard", &[hostile], FacetMatch::Any)
                .hits
                .is_empty()
        );
    }
    assert_eq!(index.stats().unwrap().pages, 4);
}

#[test]
fn tag_index_follows_edits_and_unpublishing() {
    let (wiki, mut index) = fixture();
    assert_eq!(
        index.tags().unwrap(),
        [
            FacetCount {
                value: "software".into(),
                pages: 2
            },
            FacetCount {
                value: "business".into(),
                pages: 1
            },
            FacetCount {
                value: "dns".into(),
                pages: 1
            },
        ]
    );

    wiki.write(
        "concepts/a.md",
        &tagged("A", "[networking]", "Alpha orchard claim."),
    );
    wiki.write("index.md", "[[concepts/a]] [[concepts/c]] [[concepts/d]]");
    let report = index.sync().unwrap();
    assert_eq!((report.updated, report.removed), (1, 1));

    let tags: Vec<_> = index.tags().unwrap().into_iter().map(|t| t.value).collect();
    assert_eq!(tags, ["business", "networking"]);
    assert!(
        filtered(&index, "orchard", &["software"], FacetMatch::Any)
            .hits
            .is_empty()
    );
    assert_eq!(
        pages(&filtered(
            &index,
            "orchard",
            &["networking"],
            FacetMatch::All
        )),
        ["concepts/a.md"]
    );
    assert_eq!(wiki.count("page_facet WHERE facet = 'tags'"), 2);
}

#[test]
fn malformed_tags_warn_but_still_index_the_page() {
    let wiki = Fixture::new();
    wiki.write("index.md", "[[concepts/a]]");
    wiki.write(
        "concepts/a.md",
        "---\ntitle: A\ntags: {not: a list}\n---\nAlpha orchard claim.\n",
    );
    let mut index = wiki.open();
    let report = index.sync().unwrap();
    assert_eq!(report.added, 1);
    assert_eq!(report.warnings.len(), 1, "{report:?}");
    assert!(index.tags().unwrap().is_empty());
}
