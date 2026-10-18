//! Query normalization, facet filtering, ranking, and hard output limits.

use crate::model::{FacetFilter, Result, SearchHit, SearchRequest, SearchResponse};
use crate::people;
use crate::store::Store;
use leaf_common::facet::{self, PEOPLE, PLACES, RELATIONSHIP, TAGS};
use std::collections::{BTreeMap, HashMap};

/// Query words beyond this are ignored.
pub(crate) const MAX_TERMS: usize = 32;
/// Ranked candidates examined per search; more matches set `truncated`.
const MAX_SCAN: usize = 1000;

/// Reduce free text to quoted alphanumeric words joined with OR, so no input
/// can act as FTS5 syntax. Returns `None` when there is nothing to search.
pub(crate) fn match_expression(query: &str) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if !word.is_empty() && !terms.contains(&word) {
            terms.push(word);
        }
        if terms.len() == MAX_TERMS {
            break;
        }
    }
    (!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR ")
    })
}

pub(crate) fn search(store: &Store, request: &SearchRequest) -> Result<SearchResponse> {
    let mut response = SearchResponse::default();
    let expression = match_expression(&request.query);
    let filters = normalize_filters(store, &request.filters)?;
    if expression.is_none() && filters.is_empty() {
        return Ok(response);
    }
    let candidates = store.candidates(expression.as_deref(), &filters, MAX_SCAN + 1)?;
    let unscanned = candidates.len() > MAX_SCAN;
    let mut page_facets: HashMap<String, BTreeMap<String, Vec<String>>> = HashMap::new();
    for candidate in candidates.into_iter().take(MAX_SCAN) {
        if response.hits.len() >= request.max_passages {
            response.truncated = true;
            break;
        }
        if !page_facets.contains_key(&candidate.page_path) {
            let facets = store.page_facets(&candidate.page_path)?;
            page_facets.insert(candidate.page_path.clone(), facets);
        }
        let facets = &page_facets[&candidate.page_path];
        let values = |key: &str| facets.get(key).cloned().unwrap_or_default();
        let hit = SearchHit {
            citations: store.citations(candidate.pk)?,
            passage_id: candidate.id,
            page_path: candidate.page_path,
            title: candidate.title,
            heading: candidate.heading,
            text: candidate.text,
            confidence: candidate.confidence,
            contested: candidate.contested,
            tags: values(TAGS),
            people: values(PEOPLE),
            places: values(PLACES),
            rank: response.hits.len() + 1,
            score: if expression.is_some() {
                -candidate.bm25
            } else {
                0.0
            },
        };
        // Whole passages only: one that does not fit is skipped, never cut.
        let size = hit.char_count();
        if response.chars_used + size > request.max_chars {
            response.truncated = true;
            continue;
        }
        response.chars_used += size;
        response.hits.push(hit);
    }
    if unscanned && response.hits.len() < request.max_passages {
        response.truncated = true;
    }
    Ok(response)
}

/// Resolve each filter's kind to a facet key, person names to slugs (in
/// `related-people`, `relationship` and `relationship.<person>`) and
/// organization names to entity slugs (in `*organizations` keys), normalize
/// values, and drop filters left without values.
fn normalize_filters(store: &Store, filters: &[FacetFilter]) -> Result<Vec<FacetFilter>> {
    if filters.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = store.facet_keys()?.into_iter().map(|k| k.value).collect();
    let directory = people::directory(store)?;
    let people: Vec<people::Entity> = directory.iter().map(people::Entity::from).collect();
    let mut organizations: Option<Vec<people::Entity>> = None;
    let relationship_prefix = format!("{RELATIONSHIP}.");
    let mut normalized = Vec::new();
    for filter in filters {
        let mut key = facet::resolve_kind(&filter.facet, &keys);
        if let Some(name) = key.strip_prefix(&relationship_prefix) {
            key = format!("{relationship_prefix}{}", people::resolve(&people, name));
        }
        let mut values = filter.values.clone();
        let candidates = if key == PEOPLE || key == RELATIONSHIP {
            Some(&people)
        } else if facet::names_organizations(&key) {
            if organizations.is_none() {
                organizations = Some(people::other_entities(store, &directory)?);
            }
            organizations.as_ref()
        } else {
            None
        };
        if let Some(candidates) = candidates {
            values = values
                .iter()
                .map(|name| people::resolve(candidates, name))
                .collect();
        }
        let values = facet::filter_values(&key, &values);
        if !values.is_empty() {
            normalized.push(FacetFilter {
                facet: key,
                values,
                mode: filter.mode,
            });
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::match_expression;

    #[test]
    fn expressions_contain_only_quoted_words() {
        assert_eq!(
            match_expression("NEAR(a b) OR \"c\"* -d:e a").as_deref(),
            Some("\"near\" OR \"a\" OR \"b\" OR \"or\" OR \"c\" OR \"d\" OR \"e\"")
        );
        assert_eq!(match_expression(" ?!*() "), None);
        let long = (0..50)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            match_expression(&long).unwrap().matches(" OR ").count(),
            super::MAX_TERMS - 1
        );
    }
}
