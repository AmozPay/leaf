# Leaf: status, remaining work, and handoff

This file is for whoever continues the work (a person or an agent). Read [AGENTS.md](../AGENTS.md) first for the working agreements; the specifications are [SPEC.md](SPEC.md) (viewer) and [EDITOR-SPEC.md](EDITOR-SPEC.md) (editor).

Status as of **2026-10-17**. The work was merged from `feature/core-library-and-cli` into `main` (local only, not pushed). `cargo test --workspace --all-features` passes (217 tests with `--all-features`), as do fmt, clippy (`-D warnings`) and rustdoc.

## Where things stand

| Crate | State | Tests |
|---|---|---|
| `leaf-common` | Done: the code the viewer and editor share: a public read-only `parse` API (passage and heading byte ranges, citation checks), Markdown extraction, citation syntax and source reading, facet rules, publication rules, the built-in synonym dictionary. | 35 |
| `leaf-viewer` | Done for v0.1: indexing of published pages, BM25 passage search with hard budgets, `--meta` facet filters (AND/OR/NOT, synonyms, people/organization resolution), citation lookup for any raw file type and anchor. Re-exports `leaf-common`'s `parse`, `dictionary` and citation types. | 73 |
| `leaf-editor` | Done: slices 1–7 of 7 (vocabulary, checks, conformance report, batch engine with metadata, vocabulary, passage, page and raw-source operations, `index.md` maintenance, archiving, the raw manifest, `Editor::page`). | 68 |
| `leaf` (CLI) | Done: `leaf viewer <sync|search|cite|people|places|tags|meta|stats>` and, behind the opt-in `editor` cargo feature, `leaf editor <check|apply|metadata|suggest|conformance|page>`. | 29 |
| `leaf-mcp` | Done: an MCP server over stdio (`rmcp`): `guide`, `search`, `cite`, `metadata`, `people`, `stats`; behind the opt-in `editor` cargo feature also `editor_check`, `editor_apply`, `editor_page`, `editor_metadata`, `editor_suggest`, `editor_conformance`. | 3 |

Versions to know: cache `SCHEMA_VERSION = 4` (`crates/leaf-viewer/src/store.rs`), `PARSER_VERSION = 5` (`crates/leaf-viewer/src/wiki.rs`, though the extraction it versions is in `leaf-common`), vocabulary format `VERSION = 1` (`crates/leaf-editor/src/vocabulary.rs`).

## Map of the code

**Common** (`crates/leaf-common/src/`; extraction changes here bump the viewer's `PARSER_VERSION`)

| File | Responsibility |
|---|---|
| `lib.rs` | Public facade. |
| `model.rs` | `Citation`, `Anchor`, `CitationExcerpt`, `SyncIssue`, and the errors of reading sources. |
| `publication.rs` | `index.md` allowlist; safe page path resolution; entity page paths and slugs. |
| `markdown.rs` | Frontmatter (serde-saphyr) and body (pulldown-cmark) → passages, citations, facets. Also the `^[raw/…]` provenance-marker handling. |
| `facet.rs` | Facet keys/values normalization; kind resolution (`person` → `related-people`, conventions, dictionary). |
| `dictionary.rs` | Built-in synonym dictionary (embedded `data/synonyms.yaml`). |
| `reference.rs` | Citation reference syntax (`raw/<path>[#anchor]`, wikilink form). |
| `citation.rs` | Reading the cited part of a raw source; media sniffing. |
| `parse.rs` | Public read-only parsing API used by the viewer and the editor. |
| `data/keywords.yaml` | Curated source of the dictionary (keywords → WordNet senses). |
| `examples/build-synonyms.rs` | Regenerates `data/synonyms.yaml` from Open English WordNet 2025 JSON. |

**Viewer** (`crates/leaf-viewer/src/`)

| File | Responsibility |
|---|---|
| `lib.rs` | Public facade and crate docs (doctest); re-exports common's `parse`, `dictionary` and citation types. |
| `config.rs` | Wiki root and cache path validation (cache never inside `raw/`). |
| `model.rs` | Public requests, results and errors (`From` the common errors). |
| `people.rs` | People directory and entity name resolution (slug, title, alias, unique word). |
| `store.rs` | SQLite schema (sea-query), FTS5, versioning, rebuilds. |
| `search.rs` | Query normalization, facet filters, budgets. |
| `wiki.rs` | `WikiIndex`: open/sync/search/read_citation/listings. |
| `examples/smoke.rs` | Read-only smoke run against a real wiki. |

**Editor** (`crates/leaf-editor/src/`)

| File | Responsibility |
|---|---|
| `lib.rs` | Public facade. |
| `vocabulary.rs` | Vocabulary format, baseline (`data/baseline.yaml`), extensions (`_meta/vocabulary.yaml`), merge, consistency checks. |
| `check.rs` | `Checker`: key/value checks against a given vocabulary and the index (rewrites, suggestions). |
| `suggest.rs` | Matching and suggestions (slug form, synonyms, pluralizer, strsim). |
| `editor.rs` | `Editor`, `EditorConfig` (incl. injectable `today`), metadata listing, `Editor::page` (headings, passage IDs). |
| `conformance.rs` | Conformance report; per-page checks shared with batch validation (`Checker::page_problems`). |
| `batch.rs` | Batch format (strict serde), operations, outcome types. |
| `apply.rs` | `Editor::check` / `Editor::apply`: the planner (a step either succeeds or is rejected with its errors recorded: `Step`, `Stop`), metadata operations, mirroring, validation, bookkeeping. |
| `apply/state.rs` | What a batch in progress knows, by concern: `PageSet`, `Archive`, `RawFiles`, `IndexState`, `Declarations`. |
| `apply/vocab_ops.rs` | Planner operations: declare keys, labels, places; synonyms. |
| `apply/passage_ops.rs` | Planner operations: add, replace, remove passages; citations. |
| `apply/page_ops.rs` | Planner operations: create, title, summary, headings; `index.md` lines. |
| `apply/archive_ops.rs` | Planner operations: archive and restore pages; checks that no page still refers to an archived one. |
| `frontmatter.rs` | Order-preserving frontmatter editing and writing in the wiki's style (unchanged lists keep their written layout). |
| `pages.rs` | Headings, titles, slugs and summaries; the body of a new page; the links a page makes to other pages. |
| `init.rs` | Creating a new wiki: folders, `SCHEMA.md` from the baseline, `index.md`, `log.md`, the vocabulary extension. |
| `index_md.rs` | `index.md`: finding, inserting (sorted, per section), updating and removing page lines; the header. |
| `apply/raw_ops.rs` | Planner operations: `add_raw`, anchor checks, the manifest. |
| `passages.rs` | Locating passages and headings by the viewer's ranges; checking, writing, inserting, replacing and removing passage text. |
| `commit.rs` | Lock, journal, backups, atomic writes and removals, rollback, crash recovery. |
| `examples/conformance.rs`, `examples/apply.rs`, `examples/page.rs` | Run the report, check/apply a batch file, or print a page with its passage IDs. |

**Test support**: `crates/leaf-testkit` (`TempWiki`: a temporary wiki with `write`/`read`/`root`/`db`, and `snapshot`, a development dependency only); `crates/leaf-editor/tests/common` (the editor's `batch`, `errors`, `rejected` and `EditorExt::editor`).

**CLI** (`src/`): `cli.rs` (clap), `main.rs` (dispatch, exit codes), `output.rs` (text/JSON of the viewer's results), `editor.rs` (the `editor` feature: batches, metadata, pages, conformance).

## Remaining work, in order

Each slice ends with: tests on temporary wikis, a rehearsal on a **copy** of a real wiki if it writes, docs updated, all checks green, one commit.

### Slice 3: passage operations (done)

`Editor::page` and `add_passage`, `replace_passage`, `remove_passage`, `add_citation` (see EDITOR-SPEC "Passages"). Positions come from `leaf-common`'s `parse::Passage::{span, block, in_item}` and `parse::Heading::range`; citation targets from `parse::check_citation`. Tests: `crates/leaf-editor/tests/passages.rs`. Rehearsed on a copy of a real wiki: only the intended lines changed, and every page's frontmatter round-trips byte for byte.

Not covered yet: moving a passage, editing a passage inside a blockquote or table (not indexed), and adding a passage inside a list (new passages are always paragraphs).

### Slice 4: pages and `index.md` (done)

Done: `create_page`, `set_title`, `set_summary`, `index.md` maintenance (`index_md.rs`), the link rules (`pages.rs`), and a `Checker` that sees the batch's own pages. Tests: `crates/leaf-editor/tests/pages.rs`. Rehearsed on a copy of a real wiki: only the intended lines changed. `add_heading`, `rename_heading`, `archive_page` and `restore_page` are done too: `commit::Write` can remove a file, and archiving scans every remaining page for references (`apply/page_ops.rs`). Rehearsed on a copy of a real wiki: create, archive and restore round-trip to byte-identical page content, and archiving a page other pages link to is refused.

Operations: `create_page`, `set_title`, `set_summary`, `add_heading`, `rename_heading`, `archive_page`, `restore_page`.

- `create_page` writes `<folder>/<slug>.md` for its `type`, frontmatter in baseline key order, `created`/`updated` today, and requires `required_on_create` (`confidence`); at least two outbound page links (done).
- `index.md` maintenance (done): one line per page `- [[<path without .md>|<Title>]] — <summary>` in the section of its type (`## Entities`, …), sorted by path; update the header `> Last updated: <date> | Total pages: <n>`; preserve other text. `set_title`/`set_summary` update the line.
- `archive_page` moves to `_archive/<folder>/<slug>.md` with `archived` and `archived-reason`; refuse while other pages reference it (links, `related-people`, `relationship.*`, `contradictions`, organization keys) unless the same batch removes the references. `restore_page` reverses it.
- The commit layer already supports new files; moves are a write plus a delete — extend `commit::Write` with a removal variant (journaled like the rest).
- People created in a batch must be usable later in the same batch (done: `Checker::staged`).

### Slice 5: raw sources (done)

Done: `add_raw` (`apply/raw_ops.rs`), `leaf_common::parse::{anchors, media_type, check_citation_text}`, staged raw files visible to citations and `sources` checks within the batch. Tests: `crates/leaf-editor/tests/raw.rs`. Rehearsed on a copy of a real wiki: one new file and one new manifest entry, byte-identical elsewhere, and the body checksum matches the wiki's existing convention. The original plan follows.

Operation: `add_raw {path, text | base64, source?}`.

- The path must be under a declared `raw_collections` entry with `addable: true`, lowercase, not existing. Enforce `media` (sniff like `citation.rs`), `require_frontmatter`, `require_anchor` (`message`/`block`/`any`).
- Markdown: add frontmatter `ingested` (today) and `sha256` of the **body** (the baseline LLM-wiki convention), plus `source`/`source_url` as given. Check anchors are unique.
- Record `{path, bytes, sha256}` of the **whole file** in `raw_manifest` (JSON, `files` array sorted by path) when the vocabulary declares one.
- Use `sha2` (already a viewer dependency) and `base64` for binary payloads.
- Raw files are never modified or removed by any operation.

### Slice 6: editing from the command line (done)

Done as planned: `leaf editor check|apply|metadata|suggest|conformance|page` (`src/editor.rs`), behind the opt-in `editor` cargo feature as the owner decided, so the default binary stays read-only (without it `leaf editor …` explains how to rebuild). A batch is read from a file or `-` (standard input). Exit status 4 when a batch is rejected. Tests: `tests/editor_cli.rs` (run with `--all-features`). The original plan follows.

`leaf editor check|apply <batch.yaml>`, `leaf editor metadata` (list_metadata), `leaf editor suggest <key> [value]`, `leaf editor conformance`, `leaf editor page <path>` (get_page: frontmatter, headings, passages with IDs and citations — a read op from the spec, not built yet). Open question: the `leaf` package depends on the viewer only today; editing should come behind a cargo feature (`editor`) so the default binary stays read-only. Confirm with the user before deciding the default.

### Slice 7: MCP adapter (done)

Done: `leaf-mcp` (`src/server.rs`, `src/server/editor.rs`) on `rmcp` 3.5 (the official SDK, maintained: 3.5.1 released 2026-10-05). Read tools are always there; the editor's tools only with `--features editor`, so the default server is read-only. Errors, and rejected batches, are tool errors (`isError`) whose text the model can read; a batch is sent as YAML or JSON text. Tests: `crates/leaf-mcp/tests/mcp.rs` (in-memory client and server) plus a stdio smoke on a copy of a real wiki. The original plan follows.

`leaf-mcp`: tools for search, cite, meta/people/places, and the editor's check/apply/metadata/suggest/page. Prefer the official Rust MCP SDK (`rmcp`) after checking it is maintained. Agents get write access only through `apply`; document that the wiki directory itself must not be writable by the agent's other tools.

### Known limitations

- The rule of two links to other pages makes the first page of an empty wiki impossible on its own: the first batch must create at least three interlinked pages. `init` and the plugin's skill say so. Relaxing the rule for small wikis is a decision for the owner.
- Comments inside a page's frontmatter are dropped when the editor rewrites it. Unchanged lists keep their written layout (inline or one item per line); this is read from the frontmatter's top-level and one-level-map lines in `frontmatter.rs`.
- Mirroring covers symmetric labels only; asymmetric pairs (`manager`, `mentor`) have no inverse label.
- The conformance report infers kinds of undeclared keys heuristically (dictionary word, then values).
- The people directory (viewer) lists names mentioned in `related-people` even without a page; the editor only accepts people with pages.
- `docs/core-design.md` is the original design draft, kept for history.
- Regenerating the dictionary needs `english-wordnet-2025-json.zip` (https://github.com/globalwordnet/english-wordnet/releases/tag/2025-edition; sha256 `7d749f6e2c39e6970e4997839dcf6e42fd281f3c2fae0171d2192bae8cfa4b51`). It is not committed.

## Adopting the editor on an existing wiki

Only with the owner's explicit go-ahead, and after rehearsing on a copy:
1. `cargo run -p leaf-editor --example conformance -- <wiki> /tmp/check.db` — fix or accept the issues, review the proposed declarations.
2. Save the reviewed proposal as `<wiki>/_meta/vocabulary.yaml` (add stricter `required_keys`, `addable: false` for original imports, decide uncited headings).
3. Rehearse batches on a copy with `--example apply -- --check`, then without `--check`.

For the owner's own wiki (`~/wiki`), its `SCHEMA.md` still says `related-people`/`related-places`/`relationship` are not indexed by Leaf; they are now (and filterable with `--meta`).

## Open questions for the owner

None. Editing in the CLI and MCP are opt-in cargo features (`editor`), decided by the owner; the branch is merged. Not done: pushing to a remote (there is none configured).
