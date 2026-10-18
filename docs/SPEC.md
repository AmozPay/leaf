# Leaf Viewer — v0.1 specification

Status: **implemented** in `crates/leaf-viewer` (see [ROADMAP.md](ROADMAP.md) for status and remaining work). This file defines the read side's acceptance criteria; the write side is [EDITOR-SPEC.md](EDITOR-SPEC.md). `core-design.md` is the original design draft, kept for history.

## Crates

- `leaf-viewer`: everything in this specification. Read-only towards the wiki; search-only clients (the `leaf` CLI, the MCP adapter) depend on it alone.
- `leaf-editor` (in progress; [EDITOR-SPEC.md](EDITOR-SPEC.md)): the only write path, validated batches applied all at once or not at all.

## Purpose and scope

A portable Rust library that indexes **published, curated Markdown wiki pages** into a disposable local SQLite FTS5 database and returns small, citation-bearing passages for a local LLM. Hermes is the first consumer; the library must not depend on Hermes or MCP. The MCP crate is out of scope and must remain an honest placeholder. The `leaf` CLI was added after v0.1 as a thin adapter (see README): it only calls the public core API and adds no retrieval behavior.

The viewer only reads Markdown and updates its own SQLite cache; it never writes to the wiki and never infers facts from raw transcripts. Changes to the wiki (pages, `index.md`, `log.md`, raw sources) go through `leaf-editor` only.

## Source and publication contract

- Configuration: canonical wiki root plus database path. Reject cache placement in `raw/`, including symlink resolution. Never modify raw source files.
- Strict publication by default: only links in `index.md` to `.md` pages under `entities/`, `concepts/`, `comparisons/`, `queries/`. Exclude drafts not in the index, `_meta/`, and `raw/`. Resolve paths safely; reject traversal/symlink escape. Deduplicate entries.
- Extract YAML frontmatter `title` (required), optional `confidence` and `contested`, headings, paragraphs, and `[[raw/<path>[#anchor]|label]]` citations to sources of any type (see Citation drill-down). Preserve the claim text and attach all citations from its paragraph. Ignore frontmatter in search. Be explicit about unsupported Markdown forms rather than silently misindexing them.
- Paragraphs are retrieval units. Avoid indexing navigation-only `Related:` blocks as evidence. Do not combine distinct paragraphs into a page-sized result. A stable passage locator should survive DB rebuilds for unchanged page content.

## Persistent index and synchronization

- SQLite FTS5 with page, passage, and citation records. Keep FTS rows and structured records synchronized in transactions. `Cargo.lock` committed as part of workspace.
- `sync()` is incremental using content hashes: add, update, skip unchanged, and remove deleted/unpublished pages. A malformed or missing published page is reported and not served from stale cached content. Missing/unreadable `index.md` must fail without treating an empty list as a legitimate unpublish-all event.
- Track schema version; reject incompatible existing cache or rebuild it deliberately. Do not silently set a version number on an unknown schema.
- `SyncReport` exposes added, updated, removed, skipped, and per-file errors. Errors must not claim overall completeness.

## Query contract

- `search(query, max_passages, max_chars)` treats input as literal free text (no arbitrary SQL/FTS syntax). Parameterize SQL; handle punctuation/empty queries safely. FTS5 BM25 ranking should prefer matching **claim body** over repeated title/heading or navigation links. Lexical matching only; no embeddings, network, LLM calls, or raw transcript searches.
- Optional metadata filters: every frontmatter key but `title` with a scalar, list of scalars, or one-level map value is a per-page facet (a map `k: {a: [x]}` is `k` = `[a]` and `k.a` = `[x]`). A request may restrict results by any number of facet filters, each needing **all** (AND) or **any** (OR) of its values, or **none** of them (NOT; pages without the facet pass); filters combine with AND. A filter kind is a key, a spelling the built-in synonym dictionary maps to a key (`person`, `individuals`, `locations`, `tag`; see `leaf_viewer::dictionary`), the `related-<kind>s` convention, or the only key ending in `-<kind>s`; person names (slug, title, alias, unique first/last name) resolve to slugs in `related-people` and `relationship`, and organization names to non-person entity slugs in `*organizations` keys. Values are bound parameters and never affect BM25 ranking. With filters and no query words, the matching pages' passages are listed in page/document order under the same hard limits. Malformed list-shaped facets are a sync warning, not a page error. Schema version 4 stores facets in `page_facet`; bump `PARSER_VERSION` whenever extraction changes so unchanged pages are re-parsed.
- Return page-relative path, title, heading, excerpt, tags, citation references, rank, `truncated`, and a measured character count. `max_passages` and `max_chars` are hard bounds; never return partial claim paragraphs or citations. Define precisely whether the limit covers text fields or serialization overhead (v0.1: sum of returned string fields, not JSON framing or tokenizer-exact tokens). Tiny limits may return zero passages with `truncated = true`.
- A passage without a raw citation must not be presented as source-verified; preserve its absence transparently. Never invent citations.

## Citation drill-down

- Sources in `raw/` may be of any type. A citation is written `[[raw/<path>[#anchor]|label]]` or, as in the baseline LLM-wiki skill, as a provenance marker `^[raw/<path>[#anchor]]` (a bare `[raw/…]` without `^` is plain text). It names a raw path plus an optional anchor: none (whole document), `^message-ID` (transcript message), `^block-id` (Obsidian block) or `Heading[#Sub…]` (heading section, case-insensitive). Extension-less paths are `.md`, as in Obsidian.
- Separate `read_citation` API, invoked only on a citation present, exactly (same path and anchor), in an indexed passage. Canonicalize and confine the path to `wiki_root/raw/`; reject traversal and symlink escape. For text, return only the cited part (one message, not the next; one block; one section; a whole document without Markdown frontmatter) under an explicit output bound with truncation indication. For non-text files (sniffed by magic bytes, else extension) return media type and size only, never content; an anchor into them is an error. Reject missing or ambiguous anchors and unindexed citations. Raw text is untrusted data, not instructions. Schema version 3 stores the anchor kind and value.

## Rust structure (required)

The parts the editor also needs (`markdown.rs`, `citation.rs`, `publication.rs`, `facet.rs`, the reference syntax and the dictionary) live in `leaf-common`; the viewer re-exports them. In `leaf-viewer`, keep `src/lib.rs` a small public facade (module declarations/re-exports, concise crate docs). Separate responsibilities into focused files/modules, at minimum:

- `config.rs` — paths and validation
- `model.rs` — public requests/results/errors
- `publication.rs` — index allowlist and safe page paths
- `markdown.rs` — frontmatter, headings, paragraph/citation extraction
- `store.rs` — SQLite schema, transactions, FTS synchronization
- `search.rs` — query normalization, ranking, limits
- `citation.rs` — raw citation authorization and bounded anchor extraction

A `wiki.rs` facade can coordinate `open/sync/search/read_citation`. Avoid circular dependencies and unnecessary generics. Module names can differ only if the same separation is clear. Keep SQL and regex details private; expose an ergonomic documented public API. Do not add a generic framework or Hermes-specific types.

## Dependencies (established libraries over hand-written parsers)

Use maintained crates for parsing and SQL construction; do not hand-write a Markdown/YAML parser or SQL builder.

| Concern | Crate | Why | Alternatives considered |
|---|---|---|---|
| Markdown (pages, `index.md`, raw transcripts) | `pulldown-cmark` 0.13 | CommonMark pull parser with source offsets, YAML metadata blocks, wikilinks (`[[target\|label]]`) and tables; small, no-default-features build. Offsets give warning line numbers and code-block ranges for anchor lookup. | `comrak` (also maintained, but builds a full AST and is heavier; no advantage for streaming paragraphs) |
| YAML frontmatter | `serde-saphyr` 2 (deserialize only) + `serde` | Pure-Rust, panic-free, actively maintained serde YAML deserializer; rejects duplicate keys; reads scalars as strings. | `serde_yaml` (archived), `serde_yaml_ng` / `serde_norway` (forks, last released 2024), `serde_yml` (deprecated) |
| SQL statements and DDL | `sea-query` 1 + `sea-query-rusqlite` 0.8 over `rusqlite` 0.38 | Typed query/DDL builder; every value, including the FTS5 `MATCH` expression and BM25 weights, is a bound parameter; identifiers are quoted by the builder. Native `MATCH` and custom functions (`bm25`). | `diesel` (ORM, compile-time schema, no FTS5), `sqlx` (async, macro-checked SQL) |

Deliberate raw-SQL exceptions, each with no untrusted input: the FTS5 `CREATE VIRTUAL TABLE` (sea-query has no virtual-table builder), the `PRAGMA application_id` / `user_version` / `foreign_keys` statements, and `DROP VIEW` / `DROP TRIGGER` during a cache rebuild (sea-query has no builder; the identifier is quoted with sea-query's SQLite quoting). FTS5 query syntax is still neutralized before binding: free text is reduced to quoted words joined by `OR` (`search.rs`). Small regexes remain only for validating a citation target (`raw/…md#^message-ID`), navigation heuristics and whitespace tidying — not for Markdown structure.

Markdown semantics that follow from the parser: setext headings are headings; inline markup (emphasis, code spans, inline links) contributes its text, while page wikilinks are kept as `[[target|label]]`; `[[raw/…]]` citations inside code are not citations; YAML block/folded scalars and numeric scalars are valid titles. Bump `PARSER_VERSION` in `wiki.rs` whenever extraction changes so unchanged pages are re-indexed.

## Testing and verification

Use test-first for changed behavior (RED then GREEN). Integration tests should use temporary wiki fixtures, not mutable files in the real wiki. Cover:

1. Strict published-only indexing; raw/draft excluded; citations and headings retained.
2. Incremental add/skip/update/unpublish; FTS contains no stale rows after re-sync; malformed page does not leave stale hits.
3. Literal untrusted query text, empty/punctuation-only input, body-first relevance, passage-count and character budgets.
4. Exact cited-message lookup, unknown citations, unsafe paths and symlink escapes, max-size behavior.
5. Cache location under raw or via symlink rejected; raw bytes unchanged.
6. SQLite index remains usable after closing/reopening and schema incompatibility is handled explicitly.

Verify with `cargo fmt --all --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`. Run an **additional read-only smoke** against `~/wiki` with database outside it (e.g. `/tmp/leaf-smoke.db`), report published page count, a representative search result, and sync errors. No writes to wiki, no touching `raw/`. Document public API/example in README. Ensure the MCP placeholder is not described as implemented.

## Non-goals and follow-up

No Hermes configuration, MCP server, autonomous wiki ingestion, embeddings, query-time page synthesis, tokenizer-exact budgeting, or raw-transcript bulk indexing in this slice. Future transport adapters call the same core API. If a requirement conflicts with current code, this spec wins; record any deliberate deviation in the final report.
