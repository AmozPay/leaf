//! SQLite schema, versioning, and transactional FTS5 synchronization.
//!
//! Ordinary statements are built with `sea-query` and bound through
//! `sea-query-rusqlite`, so every value is a bound parameter. Raw SQL is kept
//! only where sea-query has no builder: the FTS5 `CREATE VIRTUAL TABLE`, the
//! `PRAGMA`s that mark and version the cache, and `DROP VIEW`/`DROP TRIGGER`
//! (whose identifier is quoted by sea-query).

use crate::model::{
    Anchor, Citation, Error, FacetCount, FacetFilter, FacetMatch, IndexStats, Result,
};
use leaf_common::markdown::ParsedPage;
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, Transaction};
use sea_query::extension::sqlite::SqliteExpr;
use sea_query::{
    Alias, Asterisk, ColumnDef, Expr, ExprTrait, ForeignKey, ForeignKeyAction, Func, Index,
    IntoIden, IntoTableRef, LikeExpr, Order, Query, QuotedBuilder, SelectStatement,
    SqliteQueryBuilder, Table,
};
use sea_query_rusqlite::RusqliteBinder;
use std::collections::BTreeMap;
use std::path::Path;

/// Marks a database as a Leaf cache (`PRAGMA application_id`, "WKLF").
const APPLICATION_ID: i64 = 0x4C45_4146;
/// Bump whenever the schema changes; older caches are then rejected or rebuilt.
pub(crate) const SCHEMA_VERSION: i64 = 4;

/// Table and column names. A `Table` variant names its table.
mod schema {
    use sea_query::Iden;

    #[derive(Iden, Clone, Copy)]
    pub enum Page {
        Table,
        Path,
        Title,
        Hash,
        Confidence,
        Contested,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum PageFacet {
        Table,
        PagePath,
        Facet,
        Ordinal,
        Value,
        ValueNorm,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum Passage {
        Table,
        Pk,
        Id,
        PagePath,
        Ordinal,
        Heading,
        Body,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum Citation {
        Table,
        PassagePk,
        Ordinal,
        RawPath,
        AnchorKind,
        Anchor,
        Label,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum PassageFts {
        Table,
        Rowid,
        Title,
        Heading,
        Body,
        Links,
    }

    #[derive(Iden, Clone, Copy)]
    pub enum SqliteSchema {
        Table,
        Type,
        Name,
        Sql,
    }
}

use schema::{Page, PageFacet, Passage, PassageFts, SqliteSchema};

/// sea-query has no FTS5 builder, so the virtual table is declared here.
const FTS_SCHEMA: &str = "CREATE VIRTUAL TABLE passage_fts USING fts5(
    title, heading, body, links,
    tokenize = 'porter unicode61 remove_diacritics 2'
);";

/// Ordinary tables and indexes, rendered for SQLite.
fn schema_statements() -> Vec<String> {
    let page = Table::create()
        .table(Page::Table)
        .col(ColumnDef::new(Page::Path).text().primary_key())
        .col(ColumnDef::new(Page::Title).text().not_null())
        .col(ColumnDef::new(Page::Hash).text().not_null())
        .col(ColumnDef::new(Page::Confidence).text())
        .col(ColumnDef::new(Page::Contested).integer().not_null())
        .to_string(SqliteQueryBuilder);
    // One row per page, facet and value; a facet with no values is a row
    // with an empty `value_norm`, so it still lists as present.
    let page_facet = Table::create()
        .table(PageFacet::Table)
        .col(ColumnDef::new(PageFacet::PagePath).text().not_null())
        .col(ColumnDef::new(PageFacet::Facet).text().not_null())
        .col(ColumnDef::new(PageFacet::Ordinal).integer().not_null())
        .col(ColumnDef::new(PageFacet::Value).text().not_null())
        .col(ColumnDef::new(PageFacet::ValueNorm).text().not_null())
        .primary_key(
            Index::create()
                .col(PageFacet::PagePath)
                .col(PageFacet::Facet)
                .col(PageFacet::ValueNorm),
        )
        .foreign_key(
            ForeignKey::create()
                .from(PageFacet::Table, PageFacet::PagePath)
                .to(Page::Table, Page::Path)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .to_string(SqliteQueryBuilder);
    let page_facet_value = Index::create()
        .name("page_facet_value")
        .table(PageFacet::Table)
        .col(PageFacet::Facet)
        .col(PageFacet::ValueNorm)
        .to_string(SqliteQueryBuilder);
    let passage = Table::create()
        .table(Passage::Table)
        .col(ColumnDef::new(Passage::Pk).integer().primary_key())
        .col(ColumnDef::new(Passage::Id).text().not_null().unique_key())
        .col(ColumnDef::new(Passage::PagePath).text().not_null())
        .col(ColumnDef::new(Passage::Ordinal).integer().not_null())
        .col(ColumnDef::new(Passage::Heading).text().not_null())
        .col(ColumnDef::new(Passage::Body).text().not_null())
        .foreign_key(
            ForeignKey::create()
                .from(Passage::Table, Passage::PagePath)
                .to(Page::Table, Page::Path)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .to_string(SqliteQueryBuilder);
    let passage_page = Index::create()
        .name("passage_page")
        .table(Passage::Table)
        .col(Passage::PagePath)
        .to_string(SqliteQueryBuilder);
    let citation = Table::create()
        .table(schema::Citation::Table)
        .col(
            ColumnDef::new(schema::Citation::PassagePk)
                .integer()
                .not_null(),
        )
        .col(
            ColumnDef::new(schema::Citation::Ordinal)
                .integer()
                .not_null(),
        )
        .col(ColumnDef::new(schema::Citation::RawPath).text().not_null())
        .col(
            ColumnDef::new(schema::Citation::AnchorKind)
                .text()
                .not_null(),
        )
        .col(ColumnDef::new(schema::Citation::Anchor).text().not_null())
        .col(ColumnDef::new(schema::Citation::Label).text().not_null())
        .primary_key(
            Index::create()
                .col(schema::Citation::PassagePk)
                .col(schema::Citation::Ordinal),
        )
        .foreign_key(
            ForeignKey::create()
                .from(schema::Citation::Table, schema::Citation::PassagePk)
                .to(Passage::Table, Passage::Pk)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .to_string(SqliteQueryBuilder);
    let citation_target = Index::create()
        .name("citation_target")
        .table(schema::Citation::Table)
        .col(schema::Citation::RawPath)
        .col(schema::Citation::AnchorKind)
        .col(schema::Citation::Anchor)
        .to_string(SqliteQueryBuilder);
    vec![
        page,
        page_facet,
        page_facet_value,
        passage,
        passage_page,
        citation,
        citation_target,
    ]
}

/// BM25 with column weights: claim body dominates; title, heading and link
/// targets only help a passage match.
fn ranking() -> Expr {
    let weights = [0.5, 1.0, 10.0, 0.2].map(Expr::val);
    Func::cust("bm25")
        .args(std::iter::once(Expr::col(PassageFts::Table)).chain(weights))
        .into()
}

/// Passages of pages passing every filter: those whose FTS row matches
/// `expression`, ranked by BM25, or all of them in page and document order
/// when there is no expression. Filter keys and values must be normalized
/// and deduplicated, and each filter must have a value.
fn candidates_query(
    expression: Option<&str>,
    filters: &[FacetFilter],
    limit: usize,
) -> SelectStatement {
    let mut statement = Query::select();
    statement
        .column((Passage::Table, Passage::Pk))
        .column((Passage::Table, Passage::Id))
        .column((Passage::Table, Passage::PagePath))
        .column((Page::Table, Page::Title))
        .column((Page::Table, Page::Confidence))
        .column((Page::Table, Page::Contested))
        .column((Passage::Table, Passage::Heading))
        .column((Passage::Table, Passage::Body));
    if let Some(expression) = expression {
        statement
            .expr(ranking())
            .from(PassageFts::Table)
            .inner_join(
                Passage::Table,
                Expr::col((Passage::Table, Passage::Pk))
                    .equals((PassageFts::Table, PassageFts::Rowid)),
            )
            .and_where(Expr::col(PassageFts::Table).matches(expression))
            .order_by_expr(ranking(), Order::Asc);
    } else {
        statement.expr(Expr::val(0.0)).from(Passage::Table);
    }
    statement.inner_join(
        Page::Table,
        Expr::col((Page::Table, Page::Path)).equals((Passage::Table, Passage::PagePath)),
    );
    for filter in filters {
        let mut pages = Query::select();
        pages
            .column(PageFacet::PagePath)
            .from(PageFacet::Table)
            .and_where(Expr::col(PageFacet::Facet).eq(filter.facet.as_str()))
            .and_where(
                Expr::col(PageFacet::ValueNorm).is_in(filter.values.iter().map(String::as_str)),
            );
        if filter.mode == FacetMatch::All {
            let wanted = i64::try_from(filter.values.len()).unwrap_or(i64::MAX);
            pages.group_by_col(PageFacet::PagePath).and_having(
                Expr::from(Func::count_distinct(Expr::col(PageFacet::ValueNorm))).eq(wanted),
            );
        }
        let page = Expr::col((Passage::Table, Passage::PagePath));
        statement.and_where(match filter.mode {
            FacetMatch::Not => page.not_in_subquery(pages.to_owned()),
            FacetMatch::All | FacetMatch::Any => page.in_subquery(pages.to_owned()),
        });
    }
    statement
        .order_by((Passage::Table, Passage::PagePath), Order::Asc)
        .order_by((Passage::Table, Passage::Ordinal), Order::Asc)
        .limit(u64::try_from(limit).unwrap_or(u64::MAX))
        .to_owned()
}

/// Bind and run one built statement.
fn execute(db: &Connection, statement: &impl RusqliteBinder) -> rusqlite::Result<usize> {
    let (sql, values) = statement.build_rusqlite(SqliteQueryBuilder);
    db.prepare_cached(&sql)?
        .execute(values.as_params().as_slice())
}

/// Bind and run one query, mapping every row.
fn query<T>(
    db: &Connection,
    statement: &SelectStatement,
    map: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let (sql, values) = statement.build_rusqlite(SqliteQueryBuilder);
    let mut prepared = db.prepare_cached(&sql)?;
    prepared
        .query_map(values.as_params().as_slice(), map)?
        .collect()
}

/// Bind and run a query expected to return at most one row.
fn query_row<T>(
    db: &Connection,
    statement: &SelectStatement,
    map: impl FnOnce(&Row) -> rusqlite::Result<T>,
) -> rusqlite::Result<Option<T>> {
    let (sql, values) = statement.build_rusqlite(SqliteQueryBuilder);
    db.prepare_cached(&sql)?
        .query_row(values.as_params().as_slice(), map)
        .optional()
}

fn count(db: &Connection, table: impl IntoTableRef) -> rusqlite::Result<usize> {
    let statement = Query::select()
        .expr(Func::count(Expr::col(Asterisk)))
        .from(table)
        .to_owned();
    let found: i64 = query_row(db, &statement, |r| r.get(0))?.unwrap_or_default();
    Ok(usize::try_from(found).unwrap_or_default())
}

pub(crate) struct Store {
    db: Connection,
    /// Schema version of the cache this open replaced, if it rebuilt one.
    pub rebuilt_from: Option<i64>,
}

/// A ranked passage before budgeting.
pub(crate) struct Candidate {
    pub pk: i64,
    pub id: String,
    pub page_path: String,
    pub title: String,
    pub confidence: Option<String>,
    pub contested: bool,
    pub heading: String,
    pub text: String,
    pub bm25: f64,
}

impl Store {
    /// Open an existing Leaf cache or initialise an empty database.
    /// Foreign databases are rejected without being modified.
    pub fn open(path: &Path, rebuild_incompatible: bool) -> Result<Self> {
        let mut db = Connection::open(path)?;
        let mut rebuilt_from = None;
        let (application_id, version, objects) =
            inspect(&db).map_err(|err| foreign_or(err, path))?;
        match (application_id, version) {
            (APPLICATION_ID, SCHEMA_VERSION) => {}
            (APPLICATION_ID, found) if rebuild_incompatible => {
                rebuild(&mut db)?;
                rebuilt_from = Some(found);
            }
            (APPLICATION_ID, found) => {
                return Err(Error::IncompatibleSchema {
                    found,
                    expected: SCHEMA_VERSION,
                });
            }
            (0, 0) if objects == 0 => create(&mut db)?,
            _ => return Err(Error::ForeignDatabase(path.to_path_buf())),
        }
        db.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { db, rebuilt_from })
    }

    pub fn begin(&mut self) -> Result<SyncTransaction<'_>> {
        Ok(SyncTransaction {
            tx: self.db.transaction()?,
        })
    }

    /// Passages matching a sanitized FTS5 expression (best first) or, without
    /// one, listed in order; both limited by the facet filters. The expression
    /// and filter values are bound parameters, never spliced into SQL.
    pub fn candidates(
        &self,
        expression: Option<&str>,
        filters: &[FacetFilter],
        limit: usize,
    ) -> Result<Vec<Candidate>> {
        let statement = candidates_query(expression, filters, limit);
        Ok(query(&self.db, &statement, |r| {
            Ok(Candidate {
                pk: r.get(0)?,
                id: r.get(1)?,
                page_path: r.get(2)?,
                title: r.get(3)?,
                confidence: r.get(4)?,
                contested: r.get(5)?,
                heading: r.get(6)?,
                text: r.get(7)?,
                bm25: r.get(8)?,
            })
        })?)
    }

    pub fn citations(&self, passage_pk: i64) -> Result<Vec<Citation>> {
        use schema::Citation as C;
        let statement = Query::select()
            .columns([C::RawPath, C::AnchorKind, C::Anchor, C::Label])
            .from(C::Table)
            .and_where(Expr::col(C::PassagePk).eq(passage_pk))
            .order_by(C::Ordinal, Order::Asc)
            .to_owned();
        Ok(query(&self.db, &statement, |r| {
            Ok(Citation {
                raw_path: r.get(0)?,
                anchor: anchor_from_columns(&r.get::<_, String>(1)?, r.get(2)?),
                label: r.get(3)?,
            })
        })?)
    }

    /// A page's facets, values in written order.
    pub fn page_facets(&self, page_path: &str) -> Result<BTreeMap<String, Vec<String>>> {
        let statement = Query::select()
            .columns([PageFacet::Facet, PageFacet::Value])
            .from(PageFacet::Table)
            .and_where(Expr::col(PageFacet::PagePath).eq(page_path))
            .order_by(PageFacet::Facet, Order::Asc)
            .order_by(PageFacet::Ordinal, Order::Asc)
            .to_owned();
        let rows: Vec<(String, String)> =
            query(&self.db, &statement, |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut facets: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (facet, value) in rows {
            let values = facets.entry(facet).or_default();
            if !value.is_empty() {
                values.push(value);
            }
        }
        Ok(facets)
    }

    /// Facet keys with the number of pages that have each, most used first.
    pub fn facet_keys(&self) -> Result<Vec<FacetCount>> {
        let pages = Expr::from(Func::count_distinct(Expr::col(PageFacet::PagePath)));
        let statement = Query::select()
            .column(PageFacet::Facet)
            .expr(pages.clone())
            .from(PageFacet::Table)
            .group_by_col(PageFacet::Facet)
            .order_by_expr(pages, Order::Desc)
            .order_by(PageFacet::Facet, Order::Asc)
            .to_owned();
        Ok(query(&self.db, &statement, count_row)?)
    }

    /// Values of one facet with their page counts, most used first. Values
    /// differing only in case count together, shown in one of their spellings.
    pub fn facet_values(&self, facet: &str) -> Result<Vec<FacetCount>> {
        let pages = Expr::from(Func::count_distinct(Expr::col(PageFacet::PagePath)));
        let statement = Query::select()
            .expr(Func::min(Expr::col(PageFacet::Value)))
            .expr(pages.clone())
            .from(PageFacet::Table)
            .and_where(Expr::col(PageFacet::Facet).eq(facet))
            .and_where(Expr::col(PageFacet::ValueNorm).ne(""))
            .group_by_col(PageFacet::ValueNorm)
            .order_by_expr(pages, Order::Desc)
            .order_by(PageFacet::ValueNorm, Order::Asc)
            .to_owned();
        Ok(query(&self.db, &statement, count_row)?)
    }

    /// Indexed pages under `prefix` (e.g. `entities/`) with their titles.
    pub fn pages_under(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let statement = Query::select()
            .columns([Page::Path, Page::Title])
            .from(Page::Table)
            .and_where(
                Expr::col(Page::Path)
                    .like(LikeExpr::new(format!("{}%", escape_like(prefix))).escape('\\')),
            )
            .order_by(Page::Path, Order::Asc)
            .to_owned();
        Ok(query(&self.db, &statement, |r| Ok((r.get(0)?, r.get(1)?)))?)
    }

    /// Whether an indexed passage cites this exact raw message.
    pub fn is_cited(&self, citation: &Citation) -> Result<bool> {
        use schema::Citation as C;
        let (kind, anchor) = anchor_columns(&citation.anchor);
        let statement = Query::select()
            .expr(Expr::exists(
                Query::select()
                    .expr(Expr::val(1))
                    .from(C::Table)
                    .and_where(Expr::col(C::RawPath).eq(citation.raw_path.as_str()))
                    .and_where(Expr::col(C::AnchorKind).eq(kind))
                    .and_where(Expr::col(C::Anchor).eq(anchor))
                    .to_owned(),
            ))
            .to_owned();
        Ok(query_row(&self.db, &statement, |r| r.get(0))?.unwrap_or(false))
    }

    pub fn stats(&self) -> Result<IndexStats> {
        Ok(IndexStats {
            pages: count(&self.db, Page::Table)?,
            passages: count(&self.db, Passage::Table)?,
            citations: count(&self.db, schema::Citation::Table)?,
        })
    }
}

/// All writes of one `sync()`; nothing is visible unless it commits.
pub(crate) struct SyncTransaction<'a> {
    tx: Transaction<'a>,
}

impl SyncTransaction<'_> {
    pub fn page_hash(&self, path: &str) -> Result<Option<String>> {
        let statement = Query::select()
            .column(Page::Hash)
            .from(Page::Table)
            .and_where(Expr::col(Page::Path).eq(path))
            .to_owned();
        Ok(query_row(&self.tx, &statement, |r| r.get(0))?)
    }

    pub fn indexed_pages(&self) -> Result<Vec<String>> {
        let statement = Query::select()
            .column(Page::Path)
            .from(Page::Table)
            .order_by(Page::Path, Order::Asc)
            .to_owned();
        Ok(query(&self.tx, &statement, |r| r.get(0))?)
    }

    /// Delete a page with its passages, citations, and FTS rows.
    pub fn remove_page(&self, path: &str) -> Result<()> {
        let passages = Query::select()
            .column(Passage::Pk)
            .from(Passage::Table)
            .and_where(Expr::col(Passage::PagePath).eq(path))
            .to_owned();
        let fts = Query::delete()
            .from_table(PassageFts::Table)
            .and_where(Expr::col(PassageFts::Rowid).in_subquery(passages))
            .to_owned();
        execute(&self.tx, &fts)?;
        let page = Query::delete()
            .from_table(Page::Table)
            .and_where(Expr::col(Page::Path).eq(path))
            .to_owned();
        execute(&self.tx, &page)?;
        Ok(())
    }

    pub fn replace_page(&self, path: &str, hash: &str, page: &ParsedPage) -> Result<()> {
        use schema::Citation as C;
        self.remove_page(path)?;
        let row = Query::insert()
            .into_table(Page::Table)
            .columns([
                Page::Path,
                Page::Title,
                Page::Hash,
                Page::Confidence,
                Page::Contested,
            ])
            .values_panic([
                path.into(),
                page.title.as_str().into(),
                hash.into(),
                page.confidence.clone().into(),
                page.contested.into(),
            ])
            .to_owned();
        execute(&self.tx, &row)?;
        for (facet, values) in &page.facets {
            // An empty facet is kept as one row with empty values.
            let rows = if values.is_empty() {
                vec![String::new()]
            } else {
                values.clone()
            };
            for (ordinal, value) in rows.iter().enumerate() {
                let row = Query::insert()
                    .into_table(PageFacet::Table)
                    .columns([
                        PageFacet::PagePath,
                        PageFacet::Facet,
                        PageFacet::Ordinal,
                        PageFacet::Value,
                        PageFacet::ValueNorm,
                    ])
                    .values_panic([
                        path.into(),
                        facet.as_str().into(),
                        ordinal_value(ordinal),
                        value.as_str().into(),
                        leaf_common::facet::normalize_value(facet, value).into(),
                    ])
                    .to_owned();
                execute(&self.tx, &row)?;
            }
        }
        for (ordinal, item) in page.passages.iter().enumerate() {
            let passage = Query::insert()
                .into_table(Passage::Table)
                .columns([
                    Passage::Id,
                    Passage::PagePath,
                    Passage::Ordinal,
                    Passage::Heading,
                    Passage::Body,
                ])
                .values_panic([
                    format!("{path}#{}", item.locator).into(),
                    path.into(),
                    ordinal_value(ordinal),
                    item.heading.as_str().into(),
                    item.text.as_str().into(),
                ])
                .to_owned();
            execute(&self.tx, &passage)?;
            let pk = self.tx.last_insert_rowid();
            let fts = Query::insert()
                .into_table(PassageFts::Table)
                .columns([
                    PassageFts::Rowid,
                    PassageFts::Title,
                    PassageFts::Heading,
                    PassageFts::Body,
                    PassageFts::Links,
                ])
                .values_panic([
                    pk.into(),
                    page.title.as_str().into(),
                    item.heading.as_str().into(),
                    item.search_text.as_str().into(),
                    item.links.as_str().into(),
                ])
                .to_owned();
            execute(&self.tx, &fts)?;
            for (index, cited) in item.citations.iter().enumerate() {
                let (kind, anchor) = anchor_columns(&cited.anchor);
                let citation = Query::insert()
                    .into_table(C::Table)
                    .columns([
                        C::PassagePk,
                        C::Ordinal,
                        C::RawPath,
                        C::AnchorKind,
                        C::Anchor,
                        C::Label,
                    ])
                    .values_panic([
                        pk.into(),
                        ordinal_value(index),
                        cited.raw_path.as_str().into(),
                        kind.into(),
                        anchor.into(),
                        cited.label.as_str().into(),
                    ])
                    .to_owned();
                execute(&self.tx, &citation)?;
            }
        }
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        Ok(self.tx.commit()?)
    }
}

/// An anchor as stored: its kind (`document` when absent) and value.
fn anchor_columns(anchor: &Option<Anchor>) -> (&'static str, &str) {
    match anchor {
        None => ("document", ""),
        Some(Anchor::Message(id)) => ("message", id),
        Some(Anchor::Block(id)) => ("block", id),
        Some(Anchor::Heading(path)) => ("heading", path),
    }
}

fn anchor_from_columns(kind: &str, value: String) -> Option<Anchor> {
    match kind {
        "message" => Some(Anchor::Message(value)),
        "block" => Some(Anchor::Block(value)),
        "heading" => Some(Anchor::Heading(value)),
        _ => None,
    }
}

fn count_row(r: &Row) -> rusqlite::Result<FacetCount> {
    Ok(FacetCount {
        value: r.get(0)?,
        pages: usize::try_from(r.get::<_, i64>(1)?).unwrap_or_default(),
    })
}

/// Escape `LIKE` wildcards so a prefix matches literally (with `ESCAPE '\\'`).
fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

fn ordinal_value(ordinal: usize) -> Expr {
    Expr::val(i64::try_from(ordinal).unwrap_or(i64::MAX))
}

fn inspect(db: &Connection) -> rusqlite::Result<(i64, i64, usize)> {
    let application_id = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let objects = count(db, SqliteSchema::Table)?;
    Ok((application_id, version, objects))
}

/// A file SQLite cannot read as a database is foreign, not a database error.
fn foreign_or(err: rusqlite::Error, path: &Path) -> Error {
    match err.sqlite_error_code() {
        Some(ErrorCode::NotADatabase) => Error::ForeignDatabase(path.to_path_buf()),
        _ => err.into(),
    }
}

/// Create the current schema in an empty database.
fn create_schema(tx: &Transaction) -> Result<()> {
    for statement in schema_statements() {
        tx.execute_batch(&statement)?;
    }
    tx.execute_batch(FTS_SCHEMA)?;
    tx.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    Ok(())
}

fn create(db: &mut Connection) -> Result<()> {
    let tx = db.transaction()?;
    create_schema(&tx)?;
    tx.execute_batch(&format!("PRAGMA application_id = {APPLICATION_ID};"))?;
    Ok(tx.commit()?)
}

/// Drop every object of a Leaf cache and recreate the current schema.
fn rebuild(db: &mut Connection) -> Result<()> {
    db.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let tx = db.transaction()?;
    // Virtual tables first: dropping them removes their shadow tables.
    let listing = Query::select()
        .columns([SqliteSchema::Type, SqliteSchema::Name])
        .from(SqliteSchema::Table)
        .and_where(Expr::col(SqliteSchema::Type).is_in(["table", "view", "trigger"]))
        .and_where(Expr::col(SqliteSchema::Name).not_like("sqlite_%"))
        .order_by_expr(
            Expr::col(SqliteSchema::Sql).like("CREATE VIRTUAL%"),
            Order::Desc,
        )
        .to_owned();
    let objects: Vec<(String, String)> = query(&tx, &listing, |r| Ok((r.get(0)?, r.get(1)?)))?;
    for (kind, name) in objects {
        let exists = Query::select()
            .expr(Expr::exists(
                Query::select()
                    .expr(Expr::val(1))
                    .from(SqliteSchema::Table)
                    .and_where(Expr::col(SqliteSchema::Name).eq(name.as_str()))
                    .to_owned(),
            ))
            .to_owned();
        if !query_row(&tx, &exists, |r| r.get::<_, bool>(0))?.unwrap_or(false) {
            continue;
        }
        let drop = if kind == "table" {
            Table::drop()
                .table(Alias::new(name))
                .to_string(SqliteQueryBuilder)
        } else {
            let mut quoted = String::new();
            SqliteQueryBuilder.prepare_iden(&Alias::new(name).into_iden(), &mut quoted);
            format!("DROP {} {quoted}", kind.to_uppercase())
        };
        tx.execute_batch(&drop)?;
    }
    create_schema(&tx)?;
    Ok(tx.commit()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_text_is_bound_never_spliced() {
        let hostile = "\"x\" OR 1=1); DROP TABLE page; --";
        let filters = [FacetFilter {
            facet: hostile.into(),
            values: vec![hostile.into()],
            mode: FacetMatch::All,
        }];
        let (sql, values) =
            candidates_query(Some(hostile), &filters, 7).build_rusqlite(SqliteQueryBuilder);
        assert!(!sql.contains("DROP"), "{sql}");
        assert!(sql.contains(r#""passage_fts" MATCH ?"#), "{sql}");
        assert!(sql.contains("bm25(\"passage_fts\", ?, ?, ?, ?)"), "{sql}");
        assert!(sql.contains(r#""facet" = ?"#), "{sql}");
        assert!(sql.contains(r#""value_norm" IN (?)"#), "{sql}");
        assert!(
            sql.contains(r#"HAVING COUNT(DISTINCT "value_norm") = ?"#),
            "{sql}"
        );
        assert_eq!(values.0.iter().filter(|v| v.0 == hostile.into()).count(), 3);
    }

    #[test]
    fn any_match_has_no_grouping_and_no_query_has_no_fts() {
        let filters = [
            FacetFilter {
                facet: "tags".into(),
                values: vec!["a".into(), "b".into()],
                mode: FacetMatch::Any,
            },
            FacetFilter {
                facet: "related-places".into(),
                values: vec!["lyon".into()],
                mode: FacetMatch::Any,
            },
        ];
        let (sql, _) = candidates_query(None, &filters, 7).build_rusqlite(SqliteQueryBuilder);
        assert!(!sql.contains("HAVING"), "{sql}");
        assert!(!sql.contains("passage_fts"), "{sql}");
        assert!(sql.contains(r#""value_norm" IN (?, ?)"#), "{sql}");
        assert_eq!(
            str::matches(&sql, r#""page_path" IN (SELECT"#).count(),
            2,
            "{sql}"
        );
        let not = [FacetFilter {
            facet: "tags".into(),
            values: vec!["workshop".into()],
            mode: FacetMatch::Not,
        }];
        let (sql, _) = candidates_query(None, &not, 7).build_rusqlite(SqliteQueryBuilder);
        assert!(sql.contains(r#""page_path" NOT IN (SELECT"#), "{sql}");
        assert!(!sql.contains("HAVING"), "{sql}");
        assert_eq!(escape_like("a_b%c\\"), "a\\_b\\%c\\\\");
    }

    #[test]
    fn rebuild_quotes_hostile_object_names() {
        let mut db = Connection::open_in_memory().unwrap();
        create(&mut db).unwrap();
        db.execute_batch(
            "CREATE TABLE \"odd\"\"; DROP TABLE page; --\" (x);
             CREATE VIEW \"v\"\"\" AS SELECT 1;
             CREATE TRIGGER \"t]\" AFTER INSERT ON page BEGIN SELECT 1; END;
             PRAGMA user_version = 0;",
        )
        .unwrap();
        rebuild(&mut db).unwrap();
        let names: Vec<String> = db
            .prepare("SELECT name FROM sqlite_schema WHERE type IN ('table','view','trigger') AND name NOT LIKE 'passage_fts_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            names,
            ["citation", "page", "page_facet", "passage", "passage_fts"]
        );
        let (_, version, _) = inspect(&db).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }
}
