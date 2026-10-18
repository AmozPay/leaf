# Working on Leaf

Instructions for anyone (person or agent) changing this repository. Start with [docs/ROADMAP.md](docs/ROADMAP.md): what exists, what is left, and in which order. The specifications are [docs/SPEC.md](docs/SPEC.md) (viewer: indexing, search, citations) and [docs/EDITOR-SPEC.md](docs/EDITOR-SPEC.md) (editor: validated writes).

## Never write to a real wiki

- Do not write to `~/wiki` (the owner's wiki) or any wiki you were not asked to change. Read-only checks are fine with the cache outside the wiki (`--db /tmp/…`); confirm afterwards that nothing in the wiki changed (e.g. `find -L ~/wiki -newer <marker>`).
- Rehearse anything that writes on a **copy** in a temporary directory.
- Tests use temporary wikis only, through `leaf-testkit` (`TempWiki`, `snapshot`); the editor's integration tests share `crates/leaf-editor/tests/common`. Do not copy a `Wiki` struct into a new test file.

## Leaf owns the conventions

- The rules come from the LLM-wiki conventions, not from any one wiki's contents: the editor's baseline vocabulary (`crates/leaf-editor/data/baseline.yaml`) and the viewer's synonym dictionary (`crates/leaf-common/data/keywords.yaml` → generated `data/synonyms.yaml`). A wiki's `_meta/vocabulary.yaml` only extends the baseline.
- Never derive rules, defaults or fixtures from a real wiki. Tests, examples and docs use a neutral cast: Alice, Bob Martin ("Bobby"), Chloé Dubois (keeps an accent), Acme, Lumen School, Greenhouse, Paris, Lyon.
- A future LLM-wiki plugin will ship Leaf; keep conventions in data and docs that such a plugin can reuse.

## Use established libraries

Use maintained crates instead of hand-written parsers, builders or heuristics, and check crates.io for maintenance and adoption before adding one. In use: `pulldown-cmark` (Markdown, through `leaf-common`'s `parse` module — the editor must read pages exactly as the viewer does), `serde-saphyr` (YAML), `sea-query` + `sea-query-rusqlite` (SQL), `slug`, `strsim`, `pluralizer`, `jiff` (dates), `tempfile` (atomic writes), `indexmap`, `sha2` + `base64` (raw sources), `infer` + `mime_guess` (media types), `clap`, `rmcp` + `tokio` (the MCP server). Document deliberate exceptions (e.g. raw FTS5 DDL in `store.rs`).

## Rules of the code

- The viewer never writes to the wiki; only `leaf-editor` does, and only through validated, all-or-nothing batches (`apply.rs` → `commit.rs`).
- Code the viewer and editor both need lives in `leaf-common`; never copy it from one crate to the other, and neither depends on the other for it. The editor depends on the viewer only for the index (`WikiIndex`, `WikiConfig`, `SyncReport`).
- Bump `PARSER_VERSION` (`crates/leaf-viewer/src/wiki.rs`) whenever page extraction (`leaf-common`'s `markdown.rs`, `facet.rs`, `reference.rs`) changes, or unchanged pages keep stale index rows. Bump `SCHEMA_VERSION` (`store.rs`) when the cache schema changes.
- No network or LLM calls at runtime. The editor is deterministic; its clock is injectable (`EditorConfig::with_today`).
- Keep `lib.rs` files small facades; one responsibility per module (see the map in ROADMAP).
- When behaviour changes, update the README and the relevant spec in the same commit, and move items in ROADMAP.

## Before every commit

```sh
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps --all-features   # no warnings
```

Commit each finished step separately on the feature branch, with a message body that says why, ending with the co-author trailer used in the history. Do not push, merge or open pull requests without the owner's go-ahead.

## Gotchas

- `serde-saphyr` serialization: use `yaml_12 = true` and `no_lang_directive = true` (dates stay unquoted), `prefer_block_scalars = false` (no folded `>-` values) and `compact_list_indent = false` (indented block lists), as `frontmatter.rs` does.
- In YAML flow mappings, quote values containing commas (`{description: "a, b"}`).
- In Rust string fixtures, a name right after `\n` is not matched by word-boundary searches; check both when renaming fixtures.
- `pulldown-cmark` resolves undefined `[raw/…]` references through a broken-link callback (`markdown::events`); that is how `^[raw/…]` provenance markers become citations.
