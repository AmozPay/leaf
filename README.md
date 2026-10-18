# Leaf

Leaf is a set of tools for **LLM wikis**: a library, a command line and an MCP server that let an agent search a curated Markdown wiki with citations, and change it only through validated edits.

## Why

An LLM wiki — a folder of Markdown pages that agents compile from immutable raw sources, as in Karpathy's LLM-wiki pattern — is a good long-term memory for a personal assistant: readable by people, versionable, and cited down to the message a fact came from. Two things go wrong once agents use it every day.

- **Reading.** A local model needs a few relevant claims with their sources, not whole pages, and it must not mistake an uncited summary for evidence. Semantic search returns plausible but fuzzy context, and models invent citations. Leaf retrieves **whole cited passages** lexically, within a **hard size budget**, offline and deterministically. It only serves a citation that the wiki actually contains, and it reads exactly the cited part of the source.
- **Writing.** Every agent that edits the files directly slowly erodes the conventions the wiki depends on: tag sprawl (`coworker` beside `colleague`), broken links, dropped citations, one-sided relationships, forgotten index and log entries. A small model does this faster. Leaf makes its editor the **only write path**: an agent sends a batch of operations; the editor checks it against the wiki's conventions, rewrites synonyms to canonical forms, rejects anything that would introduce a problem (with suggestions), and writes it all at once or not at all.

**This matters most for small local models**, which Leaf is designed around:

- **Small context windows.** Every search result has a hard character budget and returns whole paragraphs, never a truncated claim or a page dump, so the context stays small without breaking a citation apart.
- **Weak at following long instructions.** A small model cannot reliably keep a wiki's whole convention document (`SCHEMA.md`) in mind while writing. Instead of trusting instructions, the editor enforces the conventions on every write. A model can only make the changes it is allowed to make.
- **Prone to inventing spellings and sources.** Keys and values are rewritten to their canonical forms (`co-worker` → `colleague`, `persons` → `related-people`). Unknown ones are rejected, never silently accepted. Search only serves citations the wiki actually contains, `sources` must name raw files that exist, and every citation an agent writes into a passage must resolve to an existing raw file and, if anchored, to an anchor in it.
- **Needs help to recover.** Errors name the operation and the rule, with suggestions and the operation to use instead (`declare_value`, "must name an existing page"). Batches are strict but simple data, capped at 50 operations, so a small model can retry with a corrected batch instead of guessing.

Larger models benefit from the same guarantees; they just need them less.

The conventions themselves are data that Leaf ships (a baseline vocabulary and a synonym dictionary), so a wiki only declares what is specific to it, and rules cannot drift between what agents are told and what is enforced.

## What it is

Citation-preserving retrieval and validated editing for curated Markdown wikis (the LLM-wiki pattern: `raw/` sources, `entities/`, `concepts/`, `comparisons/`, `queries/`, `index.md`, `log.md`). Written in Rust; search runs on a disposable SQLite FTS5 cache.

- **Read**: search a wiki's published pages and get whole, cited passages within a hard size budget; filter by frontmatter metadata; read exactly the cited part of a raw source.
- **Write**: agents change the wiki only through `leaf-editor`, whose batches are validated against the wiki's conventions and applied all at once or not at all, so even a small model cannot break the wiki.

## Workspace

| Crate | What it is | State |
|---|---|---|
| `leaf-common` | What the viewer and editor share: page parser (`parse`), citation syntax and source reading, facet rules, `index.md` publication rules, synonym dictionary. No database, never writes. | Done |
| `leaf-viewer` | Read-only library: indexing, search, metadata filters, citation lookup. Re-exports `leaf-common`'s `parse`, `dictionary` and citation types. Never writes to the wiki; search-only clients depend on it alone. | Done (v0.1) |
| `leaf-editor` | The only write path for agents: vocabulary, checks, conformance report, batches. | Done (slices 1–7) |
| `leaf` | Command-line interface: `leaf viewer <sync, search, cite, people, places, tags, meta, stats>` and, with the `editor` feature, `leaf editor <check, apply, metadata, suggest, conformance, page>`. | Done |
| `leaf-mcp` | MCP server over stdio for Hermes and other clients: search, cite, metadata, people, stats; with the opt-in `editor` feature also the editor's tools. | Done |

Documentation:

- [docs/SPEC.md](docs/SPEC.md): the viewer's specification (indexing, search, citations).
- [docs/EDITOR-SPEC.md](docs/EDITOR-SPEC.md): the editor's specification (vocabulary, operations, validation, bookkeeping).
- [docs/ROADMAP.md](docs/ROADMAP.md): current status, map of the code, remaining work and open questions.
- [AGENTS.md](AGENTS.md): working agreements for anyone continuing the work.
- [docs/core-design.md](docs/core-design.md): the original design draft (historical).

## Command-line interface

```sh
cargo install --path .            # or: cargo run -- <args>

leaf viewer sync                                  # index the pages published in ~/wiki/index.md
leaf viewer search greenhouse watering             # ranked whole passages with their citations
leaf viewer search -n 10 -c 8000 delta floods     # more passages / a larger character budget
leaf viewer cite 'raw/chatgpt/x.md#^message-ID'   # one transcript message
leaf viewer cite raw/articles/site.md             # a whole cited source (frontmatter left out)
leaf viewer cite raw/papers/p.pdf                 # non-text source: media type and size only
leaf viewer cite 'raw/notes/n.md#^block-id'       # an Obsidian block; 'raw/notes/n.md#Heading#Sub' for a section
leaf viewer search --meta=person:bobby,chloe --meta=place:lyon   # pages about Bobby AND Chloé, related to Lyon
leaf viewer search greenhouse --meta=person:bobby,chloe -m any          # pages about Bobby OR Chloé
leaf viewer search --meta-all=person:bobby,chloe --meta-any=place:lyon,paris  # AND for people, OR for places
leaf viewer search greenhouse --meta-not=tag:workshop                    # leave out pages tagged workshop
leaf viewer search --meta=organization:lumen                      # organizations resolve by slug, title, alias or word
leaf viewer search --meta=roles:cto --meta=type:entity              # any frontmatter key works
leaf viewer search --meta=relationship.alice:friend                  # people whose relationship to Alice is friend
leaf viewer search dns -t security,infrastructure                   # -t is a shortcut for --meta=tag:…
leaf viewer search -t workshop                    # no words: every passage of the matching pages
leaf viewer people                                # people, with aliases, roles, relationships, page counts
leaf viewer places                                # places, with page counts
leaf viewer tags                                  # tags in use, with page counts
leaf viewer meta                                  # every metadata kind; `leaf viewer meta roles` lists its values
leaf viewer meta --suggested                      # built-in suggested keys, their synonyms and common labels
leaf viewer stats
leaf --json viewer search delta            # machine-readable output for any command
```

Editing is built in only with the `editor` feature, so the default binary never writes to a wiki:

```sh
cargo install --path . --features editor          # or: cargo run --features editor -- <args>

leaf editor init --domain "Greenhouse notes" --tag gardening   # a new wiki in an empty ~/wiki: folders, SCHEMA.md, index.md, log.md, vocabulary
leaf editor check batch.yaml                  # would this batch be applied? (writes nothing; `-` reads standard input)
leaf editor apply batch.yaml                  # all of it or none of it, then sync the index
leaf editor page concepts/garden              # metadata, headings, passage IDs and citations
leaf editor metadata                          # keys with values, page counts, descriptions
leaf editor suggest persons                   # known, rewritten to a canonical form, or unknown (with suggestions)
leaf editor conformance                       # what in the wiki does not fit the vocabulary, and declarations to review
```

Rehearse `apply` on a copy of a wiki first. Without the feature, `leaf editor …` says how to rebuild. See [Editor](#editor) for the batch format.

- **Paths.** `--wiki` (or `LEAF_WIKI`) defaults to `~/wiki`. `--db` (or `LEAF_DB`) defaults to `~/.cache/leaf/leaf.db`, whose directory is created on first use; the cache is refused inside `raw/`. The viewer commands never write to the wiki.
- **Freshness.** `search` and `cite` sync first (unchanged pages are skipped by hash); `--no-sync` uses the cache as it is. Pages that fail to index are reported on stderr and are absent from results.
- **Output.** Text by default; `--json` prints the core result types on stdout. `cite` prints the message text on stdout and its header and truncation note on stderr, so the text can be piped. An uncited hit is labelled as wiki prose, not source-verified.
- **Metadata filters.** `--meta=KIND:VALUE[,VALUE…]` limits a search to pages by frontmatter. `KIND` is any frontmatter key (`roles`, `type`, `relationship.alice`), a shorthand (`person` → `related-people`, `place` → `related-places`, `tag` → `tags`), by convention `related-<kind>s`, or the only key ending in `-<kind>s` (`organization` finds `associated-organizations`), so new keys are filterable with no code change. `-m/--match all` (default) needs every value of a kind, `-m any` one of them; `--meta-all` and `--meta-any` fix the mode for one kind regardless of `-m`, and `--meta-not` leaves out pages with any of the values (pages without that kind are kept). Different filters always combine with AND. Values match case-insensitively; people may be named by slug, title, alias or a unique first/last name (`bobby`, `Chloé Dubois`, `chloe`), and organizations (values of `*organizations` keys) by an entity page's slug, title, alias or unique word (`Lumen School`, `lumen`). Each hit shows its page's tags, people and places.
- **Synonyms.** A small built-in dictionary maps common spellings to suggested metadata keys and labels, so a wiki does not have to define one: `--meta=individuals:…` is `--meta=person:…` (`related-people`), `locations` is `related-places`, `co-worker` is the relationship label `colleague`. `leaf meta --suggested` lists it. The synonyms are selected from Open English WordNet for one chosen sense of each keyword (`crates/leaf-common/data/keywords.yaml`), and any spelling that could mean two keywords is left out; see [NOTICE](NOTICE) for attribution. Regenerate after editing the keyword list with `cargo run -p leaf-common --features serde --example build-synonyms -- english-wordnet-2025-json.zip`.
- **Rebuilds.** A cache from an older Leaf version is rebuilt automatically (it is disposable) with a notice on stderr; `--no-rebuild` refuses it instead.
- **Limits.** `search -n/--max-passages` (default 5) and `-c/--max-chars` (default 4000) are the core's hard bounds; `cite -c/--max-chars` defaults to 2000.
- **Exit status.** 0 success, 1 error (with a hint where one helps), 2 invalid arguments, 3 `sync` could not index every published page, 4 `editor check` or `editor apply` rejected the batch (nothing was written).

## Viewer library

```rust
use leaf_viewer::{SearchRequest, WikiConfig, WikiIndex};

// The database directory must exist and must not be inside the wiki's raw/.
let mut index = WikiIndex::open(WikiConfig::new("/home/me/wiki", "/var/tmp/leaf.db"))?;
let report = index.sync()?;                 // added / updated / removed / skipped / errors / warnings
assert!(report.is_complete());              // false if any published page could not be indexed

let response = index.search(&SearchRequest::new("greenhouse watering", 5, 2_000))?;
for hit in &response.hits {
    // hit.text is one whole paragraph or list item; hit.citations are its raw sources.
    println!("{} › {}: {} ({} citations)", hit.page_path, hit.heading, hit.text, hit.citations.len());
}
let source = index.read_citation(&response.hits[0].citations[0], 1_000)?; // the cited part, bounded
```

The same example runs as a doctest in `crates/leaf-viewer/src/lib.rs`.

Behavior:

- **Read-only towards the wiki.** Only the SQLite cache is written. The cache is refused inside `raw/` (also via symlinks), and its directory is never created implicitly.
- **Published pages only.** A page is indexed only if `index.md` links it (`[[concepts/x]]`, `[[concepts/x|Label]]`, or `[Label](concepts/x.md)`) and it lives in `entities/`, `concepts/`, `comparisons/` or `queries/`. Drafts, `_meta/` and `raw/` are not indexed; traversal or symlink escapes are reported as errors.
- **Passages.** Each paragraph or list item is a passage, together with the `[[raw/…]]` citations in it (see Citations). Citation markup is taken out of the passage text and is not searchable. Inline markup (emphasis, code spans, inline links) is reduced to its text; page wikilinks stay as `[[target|label]]`. `Related:`-style navigation paragraphs are skipped. Fenced and indented code, tables, blockquotes and HTML are not indexed and are reported as sync warnings. `passage_id` stays the same across rebuilds as long as the paragraph and its heading are unchanged.
- **Metadata (facets).** Every frontmatter key except `title` whose value is a scalar, a list of scalars, or a one-level map is stored per page: `relationship: {alice: [friend]}` becomes `relationship` = `[alice]` and `relationship.alice` = `[friend]`. `SearchRequest::with_facet(kind, values, FacetMatch::All | FacetMatch::Any | FacetMatch::Not)` (and the `with_tags` / `with_people` / `with_places` shortcuts) limits a search to matching pages; filters combine with AND; with an empty query the matching pages' passages are listed in document order (`score` 0). Metadata is a filter, never ranking text; a hit's `tags`, `people` and `places` count towards `max_chars`. `WikiIndex::people()` joins `related-people` slugs with their `entities/<slug>.md` pages (title, aliases, every other key); `places()`, `tags()`, `facets()` and `facet_values(kind)` list values with page counts. Malformed `tags`, `related-people`, `related-places` or `aliases` are ignored with a sync warning; deeper structures elsewhere are skipped.
- **Search.** The query is treated as literal words, combined with OR and ranked by BM25. Body text is weighted far above title, heading and link targets. `max_passages` and `max_chars` are hard limits. `max_chars` counts the characters of every returned string field (`SearchHit::char_count`), not JSON framing or model tokens. A passage that does not fit is skipped whole and `truncated` is set. A hit with no citations is wiki prose, not source-verified evidence (`SearchHit::is_cited`).
- **Citations.** A citation is `[[raw/<path>[#anchor]|label]]`, or the baseline LLM-wiki provenance marker `^[raw/<path>[#anchor]]`, to a file of any type in `raw/` (a path without extension is a `.md` file, as in Obsidian). No anchor cites the whole document; `#^message-ID` one transcript message; `#^block-id` an Obsidian block (a paragraph or list item ending in `^block-id`, or the block before a lone `^block-id` line); `#Heading#Sub` a heading section. `Citation` displays as and parses from that reference form. `read_citation` serves only a citation that appears, exactly, in an indexed passage, confined to `raw/`. Text sources return the cited part (whole documents without Markdown frontmatter), capped at `max_chars`; other files (sniffed by content with `infer`, else by extension with `mime_guess`) return only `media_type` and `size_bytes`, and an anchor into them is refused. Raw text is untrusted data.
- **Cache versioning.** Leaf caches are marked (`application_id`) and versioned. A database Leaf did not create is never modified. An outdated cache is rejected unless `WikiConfig::rebuild_incompatible_cache(true)` is set.

Dependencies: Markdown is parsed with [`pulldown-cmark`](https://crates.io/crates/pulldown-cmark), YAML frontmatter with [`serde-saphyr`](https://crates.io/crates/serde-saphyr), and SQL is built with [`sea-query`](https://crates.io/crates/sea-query) and bound through [`sea-query-rusqlite`](https://crates.io/crates/sea-query-rusqlite) over `rusqlite` (bundled SQLite). Only the FTS5 virtual-table DDL and cache PRAGMAs are raw SQL. Rationale and alternatives: [docs/SPEC.md § Dependencies](docs/SPEC.md#dependencies-established-libraries-over-hand-written-parsers).

Read-only smoke run against a real wiki (database outside the wiki):

```sh
cargo run -p leaf-viewer --example smoke -- ~/wiki /tmp/leaf-smoke.db 'greenhouse watering' \
  'raw/notes/watering.md#^message-m1'   # optional: citations to read, as printed by search
```

## Releases

`scripts/package-release.sh [TARGET]` builds three binaries for a target and packs them as `dist/leaf-<version>-<target>.tar.gz`, printing the archive's sha256: `leaf-mcp` (read-only), `leaf-mcp-editor` (the same plus the editor tools) and `leaf` (the CLI with `leaf editor …`). The two servers are separate builds, so the read-only one cannot be made to write. `.github/workflows/release.yml` does this for four targets when a `v*` tag is pushed. **That workflow has not been run** (there is no remote); packaging was checked locally for the host target only. The [grove plugin](https://github.com/AmozPay/grove) downloads these archives and checks each against a hash it pins.

## MCP server

`leaf-mcp` serves a wiki to an MCP client (such as Hermes or Claude Code) over stdio. It documents itself: the server's instructions, each tool's description and the batch format (with an example that a test keeps valid) are what the client sees, so they are not repeated here. **The agent is never told where the wiki is:** tools take and return wiki-relative paths, `guide` returns the wiki's rules, latest log entries and index, and the server replaces the folder with `<wiki>` in any error.

```sh
cargo install --path crates/leaf-mcp                     # read-only
cargo install --path crates/leaf-mcp --features editor   # also the editor_* tools
leaf-mcp --wiki ~/wiki --db ~/.cache/leaf/leaf.db   # or LEAF_WIKI / LEAF_DB
```

Results are JSON text. A bad request, or a rejected batch, is a tool error (`isError`) whose text says what to fix, so the model can retry. A batch is sent as YAML or JSON text; its format is in the `editor_apply` tool description. The default build is read-only; an agent can write only through `editor_apply`, so the wiki directory itself must not be writable by the agent's other tools.

## Editor

`leaf-editor` checks and applies **batches**: plain YAML or JSON that an agent sends instead of editing files.

```yaml
agent: wiki-curator          # recorded in log.md
model: small-8b              # recorded in log.md
kind: update                 # update | correction | review | merge
summary: Record Bob's role
notes: [From the 2026-10-17 interview.]
ops:
  - {op: declare_value, key: roles, value: maintainer, description: Keeps a system running}
  - {op: add_metadata, page: entities/bob-martin, key: roles, values: [maintainer]}
  - {op: add_metadata, page: entities/alice, key: relationship.bobby, values: [co-worker]}
  - op: add_passage
    page: entities/bob-martin
    under: Work              # a heading (the title means the introduction), or `after: <passage id>`
    text: In October 2026, Alice said Bobby still maintains the Greenhouse setup.
    citations: ["raw/notes/a.md#^message-a"]
```

Passages are addressed by the IDs the viewer gives them (`entities/bob-martin.md#p-1a2b3c4d5e6f`); `Editor::page` lists a page's headings and passages with their IDs and citations. `replace_passage` must keep every old citation unless it lists it in `drop_citations`, `remove_passage` needs a reason (recorded in `log.md`), and `add_citation` appends one citation. An ID from an outdated read is rejected, so an agent cannot overwrite text it has not seen.

What happens:

- **Checked as a whole first.** Operations run in memory in order, against the wiki's vocabulary as the batch changes it. Spellings are rewritten to canonical forms and reported (`co-worker` → `colleague`, `bobby` → `bob-martin`, `keywords` → `tags`); unknown keys or values are rejected with suggestions and a hint (`declare_value`, `declare_place`, "must name an existing page"). Symmetric relationships are mirrored on the other person's page. Passage text must be one line of plain text (page links allowed, no formatting) within `limits.passage_chars`, with its citations given separately; each citation must resolve, and a passage outside the introduction and summary headings must cite something. Every `[[page]]` link must resolve and a page links to at least two others. Editor-managed keys (`created`, `updated`, `type`) cannot be set; `contested` is only written when true.
- **No new problems.** Every changed page is re-read as the index reads it: a problem the batch introduces rejects it, a problem the page already had is a warning (old wikis stay editable).
- **All or nothing.** An acceptable batch is written under a lock with a journal and backups, through temporary files renamed into place, and rolled back if anything fails (also after a crash, on the next open). Then `updated` dates are set, one `log.md` entry records the agent, model, changes and declarations, declarations go to `_meta/vocabulary.yaml`, and the index is synced.
- **Conventions.** Leaf ships a baseline vocabulary (page types, required keys, metadata keys and their value kinds, relationship and confidence labels, raw collections); a wiki's optional `_meta/vocabulary.yaml` adds its own tags, places, keys and rules.

Implemented so far: the metadata operations (`set_metadata`, `add_metadata`, `remove_metadata`), vocabulary declarations (`declare_value`, `declare_place`, `declare_key`, `add_synonym`) passage operations (`add_passage`, `replace_passage`, `remove_passage`, `add_citation`) and page operations (`create_page`, `set_title`, `set_summary`, `add_heading`, `rename_heading`, `archive_page`, `restore_page`, with `index.md` kept up to date) and raw sources (`add_raw`: collection rules, source frontmatter, body checksums, the manifest). MCP access is below (see [docs/ROADMAP.md](docs/ROADMAP.md)).

Try it, **on a copy of a wiki**:

```sh
cargo run -p leaf-editor --example conformance -- <wiki> /tmp/check.db            # issues and declarations to make
cargo run -p leaf-editor --example page -- <wiki> /tmp/check.db concepts/garden     # headings, passage IDs, citations
cargo run -p leaf-editor --example apply -- --check <wiki> /tmp/check.db batch.yaml # dry run
cargo run -p leaf-editor --example apply -- <wiki-copy> /tmp/check.db batch.yaml    # writes
```

## Check the workspace

```sh
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps --all-features
```

See [AGENTS.md](AGENTS.md) before changing anything.
