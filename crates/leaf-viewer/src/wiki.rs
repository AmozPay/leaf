//! `WikiIndex`: coordinates configuration, sync, search, and citation reads.

use crate::config::{self, WikiConfig};
use crate::model::{
    Citation, CitationExcerpt, Error, FacetCount, IndexStats, PageRef, Person, Result,
    SearchRequest, SearchResponse, SyncIssue, SyncReport,
};
use crate::people;
use crate::search;
use crate::store::{SCHEMA_VERSION, Store};
use leaf_common::citation;
use leaf_common::facet;
use leaf_common::markdown;
use leaf_common::publication;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

/// Bump when parsing rules change so unchanged pages are re-indexed.
const PARSER_VERSION: u32 = 5;

/// A wiki and its SQLite cache. Reads the wiki; writes only the cache.
pub struct WikiIndex {
    root: PathBuf,
    store: Store,
}

impl WikiIndex {
    /// Validate paths and open (or create) the cache. Does not read pages;
    /// call [`sync`](Self::sync) to bring the cache up to date.
    pub fn open(config: WikiConfig) -> Result<Self> {
        let paths = config::resolve(&config)?;
        let store = Store::open(&paths.database, config.rebuild_incompatible_cache)?;
        Ok(Self {
            root: paths.root,
            store,
        })
    }

    /// The schema version of an incompatible cache that [`open`](Self::open)
    /// rebuilt (empty) because
    /// [`WikiConfig::rebuild_incompatible_cache`] was set; `None` otherwise.
    /// The next [`sync`](Self::sync) re-indexes every page.
    pub fn rebuilt_from(&self) -> Option<i64> {
        self.store.rebuilt_from
    }

    /// The schema version this build writes.
    pub fn schema_version() -> i64 {
        SCHEMA_VERSION
    }

    /// Bring the cache in line with the pages published in `index.md`.
    ///
    /// Unchanged pages (same content hash) are skipped. Pages that are
    /// missing, unsafe, or malformed are reported in
    /// [`SyncReport::errors`] and removed from the cache. Pages no longer
    /// listed are removed. Fails without changing the cache if `index.md`
    /// cannot be read. All changes commit in one transaction.
    pub fn sync(&mut self) -> Result<SyncReport> {
        let index_path = self.root.join("index.md");
        let index = fs::read_to_string(&index_path).map_err(|err| Error::io(index_path, err))?;
        let publication = publication::published_pages(&index);
        let mut report = SyncReport {
            errors: publication.errors,
            ..SyncReport::default()
        };
        let tx = self.store.begin()?;
        for path in &publication.pages {
            let issue = |message: String| SyncIssue {
                path: path.clone(),
                message,
            };
            let content = match publication::resolve_page(&self.root, path).and_then(|file| {
                fs::read_to_string(file).map_err(|err| format!("cannot read page: {err}"))
            }) {
                Ok(content) => content,
                Err(message) => {
                    tx.remove_page(path)?;
                    report.errors.push(issue(message));
                    continue;
                }
            };
            let hash = content_hash(&content);
            let previous = tx.page_hash(path)?;
            if previous.as_deref() == Some(hash.as_str()) {
                report.skipped += 1;
                continue;
            }
            let page = match markdown::parse_page(&content) {
                Ok(page) => page,
                Err(message) => {
                    tx.remove_page(path)?;
                    report.errors.push(issue(message));
                    continue;
                }
            };
            report
                .warnings
                .extend(page.warnings.iter().cloned().map(issue));
            tx.replace_page(path, &hash, &page)?;
            if previous.is_some() {
                report.updated += 1;
            } else {
                report.added += 1;
            }
        }
        for path in tx.indexed_pages()? {
            if !publication.pages.contains(&path) {
                tx.remove_page(&path)?;
                report.removed += 1;
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// Ranked, bounded passages for free-text `request.query`.
    pub fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        search::search(&self.store, request)
    }

    /// Read the source behind a citation returned by [`search`](Self::search):
    /// the whole document (without frontmatter), one message, block or
    /// heading section, cut to at most `max_chars` characters. A source that
    /// is not text is described by its media type and size only.
    ///
    /// Only citations attached to an indexed passage are served, and only
    /// from files inside `raw/`. The text is untrusted data.
    pub fn read_citation(&self, citation: &Citation, max_chars: usize) -> Result<CitationExcerpt> {
        if max_chars == 0 {
            return Err(Error::InvalidRequest("max_chars must be positive".into()));
        }
        citation::check_path(&citation.raw_path)?;
        if !self.store.is_cited(citation)? {
            return Err(Error::CitationNotIndexed);
        }
        let source = citation::resolve(&self.root, &citation.raw_path)?;
        Ok(citation::read(&source, citation, max_chars)?)
    }

    /// Frontmatter tags of indexed pages with their page counts, most used
    /// first; the values [`SearchRequest::with_tags`] can filter on.
    pub fn tags(&self) -> Result<Vec<FacetCount>> {
        self.store.facet_values(facet::TAGS)
    }

    /// People named in `related-people`, joined with their entity pages
    /// (title, aliases, relationships, roles, …), most mentioned first.
    pub fn people(&self) -> Result<Vec<Person>> {
        people::directory(&self.store)
    }

    /// Places named in `related-places`, with their page counts.
    pub fn places(&self) -> Result<Vec<FacetCount>> {
        self.store.facet_values(facet::PLACES)
    }

    /// Every indexed page, by path.
    pub fn pages(&self) -> Result<Vec<PageRef>> {
        Ok(self
            .store
            .pages_under("")?
            .into_iter()
            .map(|(path, title)| PageRef { path, title })
            .collect())
    }

    /// Every frontmatter facet key with the number of pages that have it.
    pub fn facets(&self) -> Result<Vec<FacetCount>> {
        self.store.facet_keys()
    }

    /// Values of one facet with their page counts. `kind` is a key or a
    /// kind as in [`SearchRequest::with_facet`] (`person`, `place`, …).
    pub fn facet_values(&self, kind: &str) -> Result<Vec<FacetCount>> {
        let keys: Vec<String> = self
            .store
            .facet_keys()?
            .into_iter()
            .map(|k| k.value)
            .collect();
        self.store.facet_values(&facet::resolve_kind(kind, &keys))
    }

    /// Counts of indexed pages, passages, and citations.
    pub fn stats(&self) -> Result<IndexStats> {
        self.store.stats()
    }
}

fn content_hash(content: &str) -> String {
    let digest = Sha256::digest(format!("{SCHEMA_VERSION}:{PARSER_VERSION}:{content}"));
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
