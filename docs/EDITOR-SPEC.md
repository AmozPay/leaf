# Leaf Editor — specification

Status: **implemented** — all 7 slices (see *Implementation status* below and [ROADMAP.md](ROADMAP.md)). `leaf-viewer` ([SPEC.md](SPEC.md)) is the read side; this document defines `leaf-editor`, the write side.

## Purpose

Agents never write the wiki directly: `leaf-editor` is the only write path. It exists so that every change respects the wiki's conventions **however small the model making it is**. A batch of edits is either applied completely and consistently, or rejected with errors that say what to change; it is never half applied.

Conventions include the baseline LLM-wiki rules (`SCHEMA.md` in the wiki) and the improvements made with Leaf: published pages listed in `index.md`, message-level citations into immutable `raw/` sources (whole documents, `^message-`, `^block`, `#Heading` anchors), passages as retrieval units, frontmatter metadata as generic facets (`tags`, `related-people`, `related-places`, `relationship` maps, …), and the checksummed raw manifest.

Non-goals:

- **Access control.** Preventing agents from writing files directly (filesystem permissions, the MCP/tool configuration) is outside this crate. The editor makes the only allowed path a safe one.
- **Judging truth.** The editor checks structure, vocabulary, references and citations, not whether a claim is supported by its source. Evidence rules that need judgment stay in `SCHEMA.md` for the agent and the human reviewer.
- **Network or LLM calls.** The editor is offline and deterministic: the same batch on the same wiki gives the same result.

## Guarantees

1. **All-or-nothing batches.** Every operation is applied in memory, the resulting wiki state is validated as a whole, and only then written. Any error rejects the whole batch and leaves every file byte-for-byte unchanged.
2. **Raw is write-once.** A raw file can be added, never modified, replaced or deleted. Its checksum is recorded in the manifest.
3. **No new problems.** After a batch, no changed page has a problem it did not have before; the viewer indexes it as before. Problems a page already had (in a wiki written before the editor) are reported as warnings rather than blocking every edit to that page, so legacy wikis stay editable while they are cleaned up.
4. **Automatic bookkeeping.** `index.md`, `log.md`, `created`/`updated` dates and the raw manifest are maintained by the editor, never by the agent.
5. **Sync.** Each applied batch is followed by a viewer `sync()`; its report is part of the batch result.
6. **Stale edits fail.** Operations address passages by the viewer's content-derived passage IDs, so an edit based on an outdated read is rejected instead of overwriting newer text.
7. **Actionable errors.** Each error names the operation, the rule, and how to fix it, with suggestions where possible ("unknown value `coworker` for `relationship.alice`; did you mean `colleague`? Use `declare_value` to add a new one.").

## Architecture

```
agent ──batch (JSON/YAML)──▶ leaf-editor ──reads──▶ wiki files
                                │  apply in memory → validate → write atomically
                                ├──▶ index.md, log.md, manifest, dates
                                └──▶ leaf-viewer::WikiIndex::sync()
```

- `leaf-editor` and `leaf-viewer` both depend on `leaf-common`, which holds the Markdown/frontmatter parser, citation syntax, passage locators, facet rules and synonym dictionary, so what the editor accepts is exactly what the viewer reads. The editor depends on the viewer only for the index (`WikiIndex`, `WikiConfig`, `SyncReport`).
- Batches are serde data (JSON or YAML), so the same format serves the library, a CLI (`leaf editor apply batch.yaml`, `leaf editor check batch.yaml`) and an MCP tool (`editor_check`, `editor_apply`).

### Viewer changes

- Done: a public read-only `parse` module (`page`, `frontmatter`, `frontmatter_block`, `published_pages`, `published_file`, `PUBLISHED_DIRS`), `Citation`'s `FromStr`/`Display` reference syntax, `WikiIndex::pages()`, and the synonym dictionary (`leaf_common::dictionary`).
- Done (slice 3): `parse::Passage` carries byte ranges (`span`: its text and citations as written, without list marker or nested items; `block`: its list item or paragraph; `in_item`), `parse::Page::headings` with ranges, and the read-only `parse::check_citation(root, &Citation)` (raw file exists, anchor found). None of these are part of passage IDs, so IDs are unchanged.

## The vocabulary: baseline and `_meta/vocabulary.yaml`

Metadata stays generic: `related-people` and `relationship` are ordinary keys. The vocabulary describes what keys and values *mean*. Leaf owns the conventions: a **baseline vocabulary** ships with the editor (`crates/leaf-editor/data/baseline.yaml`), written from the baseline LLM-wiki skill plus Leaf's additions, and a wiki's optional `_meta/vocabulary.yaml` only **extends** it (edited only through the editor). Entries with the same name replace the baseline's (page types, keys, raw collections, labels, places); lists are combined (required keys, summary headings); `limits`, `uncited_intro` and `raw_manifest` override when given. A wiki with no file uses the baseline as is.

The baseline:

- **Page types**: `entity`, `concept`, `comparison`, `query` (the `summary` type of the baseline skill is dropped: it has no published folder).
- **Required on every page**: `title`, `type`, `created`, `updated`, `tags`, `sources`. **Required on new pages**: `confidence` (`high`, `medium`, `low`); pages written without it are unrated, never assumed `high`.
- **Optional, checked when present**: `contested` (absent means false; the editor writes it only when true), `contradictions`, and the dictionary's keys (`related-people`, `related-places`, `related-organizations`, `related-events`, `related-projects`, `relationship`, `aliases`, `roles`).
- **Raw collections**: `articles` (Markdown with `source_url`), `papers`, `transcripts` (Markdown), `assets`; new Markdown sources get `ingested` and `sha256` (body hash).
- **No tags or places**: each wiki declares its own taxonomy and places, as the baseline skill requires ("add a tag to the taxonomy before using it").
- Uncited introductions are allowed; relationship labels (`friend`, `colleague`, …) are declared, most of them symmetric.

A wiki's extension, for example:

```yaml
version: 1
required_keys: [related-people, related-places]   # stricter than the baseline
summary_headings: [Historical evidence]
raw_manifest: _meta/raw-manifest.json
raw_collections:
  exports: {addable: false}                        # an original import that must not grow
  confirmations: {media: [text/markdown], require_anchor: message, require_frontmatter: [source]}
keys:
  associated-organizations: {values: organization} # a key of its own
labels:
  tags:                                            # the wiki's tag taxonomy
    software: {description: Software work}
    gardening: {}
  relationship:
    colleague: {synonyms: [teammate-at-work]}      # a wiki-specific spelling
places:
  paris: {title: Paris}
  lyon: {title: Lyon, synonyms: [lugdunum]}
```

Value kinds:

| Kind | A valid value is |
|---|---|
| `label` | a value listed under `labels.<key>` (or one of its curated synonyms, rewritten to it) |
| `person` | the slug of a person entity page: an `entities/<slug>.md` page with a `relationship` map or listed in someone's `related-people`, existing or created in the same batch |
| `organization` / `entity` | the slug of a non-person / any entity page |
| `place` | a slug listed under `places` |
| `page` | a published page path |
| `raw` | an existing raw path |
| `date`, `boolean`, `text` | an ISO date, `true`/`false`, any non-empty text |

`raw_collections` declares each `raw/<collection>/` folder with its rules: `addable` (default true), allowed `media` types (sniffed as the viewer does), `require_frontmatter` keys for Markdown, and `require_anchor` (`message`, `block` or `any`: the file must contain at least one such anchor). Adding a file to an undeclared collection is rejected.

`uncited_intro` allows a page's introduction (passages under its title, before the first section heading) to be uncited; `summary_headings` lists section headings under which passages may be uncited ("Historical evidence" summaries). Every other passage needs a citation.

`limits` caps a batch (`ops_per_batch`, default 50) and each passage's text (`passage_chars`, default 2000). Larger batches are rejected with a request to split them: a small model keeps a 50-operation batch coherent, and a human can review it.

`only_on` restricts a key to a page type or to person entities. `single` means one value. `symmetric` labels are mirrored: adding `relationship.bob-martin: [friend]` on Alice's page adds `relationship.alice: [friend]` on Bob's page in the same batch.

**Existing wikis.** A conformance report lists what in an existing wiki does not fit the baseline: keys, tags, places and raw collections it would need to declare, and pages missing required keys. It proposes those declarations as an extension for a human to review; it never turns a wiki's current contents into rules wholesale.

## Suggestions and new vocabulary

Keys and values are matched in this order:

1. **Same slug**: case, spacing, punctuation, accents and a leading `#` do not matter (`Related People` → `related-people`, `Lyon` → `lyon`). Rewritten automatically.
2. **Synonyms**, rewritten automatically and reported:
   - the **built-in dictionary** (`leaf_common::dictionary`), shipped so a wiki does not have to define synonyms: suggested metadata keys (`persons`, `individuals`, `related-persons` → `related-people`; `locations` → `related-places`; `nicknames` → `aliases`; …) and common labels (`co-worker`, `workmate` → `colleague`; `spouse` → `partner`; …). It is generated offline from a curated keyword list (`crates/leaf-common/data/keywords.yaml`) and Open English WordNet: each keyword is tied to the WordNet senses that mean it, so only that sense's words count (`place` as a location, not a job), and any spelling that could mean two keywords is dropped. Moby-style thesauri were rejected for this because they mix related words with synonyms (`enemy` near `friend`);
   - the vocabulary's own `synonyms` (curated per wiki);
   - entity titles and aliases for person and organization values.
3. Otherwise the value is **unknown** and rejected with suggestions: the same singular/plural (via `pluralizer`) alone when there is one, else look-alikes by string similarity (`strsim`) for typos. A key that the dictionary suggests but the wiki has not declared is suggested for declaration rather than accepted.

The dictionary's common labels (relationship types, confidence levels) are usable without declaring them. No online service is used at any point: the editor would otherwise send the wiki's private vocabulary to a third party, depend on a remote service, and fail offline.

Reusing existing vocabulary is always allowed. Introducing something new takes an explicit operation in the same batch, with a description (`declare_key`, `declare_value`, `declare_place`, `add_synonym`). New vocabulary is therefore deliberate and appears in `log.md`.

## Operations

A batch:

```yaml
agent: hermes-wiki-sync          # required: which agent made the batch
model: qwen3-8b                  # required: which model; both are recorded in log.md
kind: update            # update | correction | review | merge (the log.md entry type)
summary: Relationship and place metadata   # log.md heading
notes:                  # log.md bullets written by the agent (optional)
  - Recorded Bob's Acme role from the 2026-10-17 interview.
ops:
  - op: add_raw
    path: raw/user-confirmations/2026-10-18-bobby-role.md
    source: Alice's direct statement in Hermes session 20261018_…
    text: |
      Bobby still maintains the Greenhouse setup.

      ^message-bobby-role-2026-10-18
  - op: add_passage
    page: entities/bob-martin.md
    under: Bob Martin          # a heading, or `after: <passage id>`
    text: In October 2026, Alice said Bobby still maintains the Greenhouse setup.
    citations: ["raw/user-confirmations/2026-10-18-bobby-role.md#^message-bobby-role-2026-10-18"]
```

**New wiki** (`leaf_editor::init_wiki`, `leaf editor init`): in an empty or missing folder, creates the baseline folders (`entities/`, `concepts/`, `comparisons/`, `queries/`, `raw/<collection>/`, `_meta/`), a `SCHEMA.md` written from the baseline vocabulary (page types, required keys, limits, the starting tag taxonomy), `index.md` with its four sections and header, `log.md` with the first entry, and `_meta/vocabulary.yaml` declaring the starting tags. A folder with any file in it is refused, nothing is overwritten. The result is conformant, and the viewer syncs it without a problem. Because every page needs two links to other pages, the first pages of a new wiki are created together: three interlinked pages in one batch (a single first page is rejected).

**Read** (no batch, no writes): `list_metadata` (keys, values, page counts, descriptions, synonyms), `suggest {key, value}`, `get_page {path}` (frontmatter, headings, passages with IDs and citations), `check {batch}` (full validation, dry run).

**Raw sources**
- `add_raw {path, text | base64, source?, source_url?}`: path `raw/<collection>/<file>` (an extension, lowercase, must not exist) under a declared, addable collection (the baseline declares `articles`, `papers`, `transcripts`, `assets`). `base64` carries a binary file; a file is never read from the agent's disk. The collection's `media` types (sniffed as the viewer does) and `require_frontmatter` keys are enforced. For Markdown: the editor adds the frontmatter `source` and `source_url` as given (or those the text already has; giving one twice is an error), `ingested` (today) and `sha256` (of the body, the text after the frontmatter, as the existing direct confirmations have it); a text that brings its own `ingested` or `sha256` is rejected. Every `^message-<id>` and `^<id>` anchor must be unique and citable, and `require_anchor` needs at least one of its kind. Any other file type is stored as given. The manifest gets `{path, bytes, sha256}` of the whole file, inserted in path order (an existing manifest keeps its layout and the order of its entries). Later operations of the same batch can cite the new file, and list it under `sources`.

**Pages**
- `create_page {type, slug, title, summary, metadata, intro: [{text, citations}], sections: [{heading, passages: [{text, citations}]}]}`: writes `<folder>/<slug>.md` with frontmatter in the vocabulary's key order (`title`, `type`, `created`, `updated`, then the required keys, then the rest as given), a `# Title` heading, the `intro` passages, one `##` heading per section, and adds the `index.md` line. `metadata` takes every key the editor does not set itself, checked like `set_metadata`; `sources` is derived from the citations (and merged with any given). Every key in `required_keys` and `required_on_create` must be given (`tags` may be `[]`). The slug must be new and lowercase-hyphenated; headings must be unique and plain text. People created earlier in the batch can be referenced later in it: a new entity page counts as a person once it has a `relationship` map or names its own slug in `related-people`.
- `set_title {page, title}`: sets the frontmatter title, the top `# ` heading when it carries the old title, and the `index.md` label. The title is part of passage IDs, so the new IDs of the passages under a renamed heading are reported (`BatchOutcome::passages`).
- `set_summary {page, summary}`: only the page's `index.md` line changes; the page file is not touched.
- `archive_page {page, reason}`: moves the page to `_archive/<folder>/<slug>.md` (the baseline skill's archive folder) with `archived: <date>`, `archived-reason` and, when its `index.md` line had a summary, `archived-summary` added to its frontmatter, and removes its `index.md` line (the viewer drops it on sync). Rejected while other pages still refer to it (links, and metadata that names it: `related-people` and other person, organization, entity and page keys, `relationship.<slug>` entries, `contradictions`) unless the same batch removes those references; the references are checked on every page as the batch leaves it, and the error lists the referring pages. Deletion is not offered. The outcome lists the moved file under `removed`.
- `restore_page {page, summary?}`: moves an archived page back (`page` is its `_archive/…` path or its original path; the original path must be free) and puts its `index.md` line back, with the kept `archived-summary` or the `summary` given. The page is checked as a new page: any problem it has now (a link to a page that no longer exists, an undeclared tag) rejects the batch. Archiving and restoring the same page in one batch is rejected.
- `add_heading {page, heading, level?, after?}`: an empty heading (level 2 to 6, default 2) at the end of the section of the heading `after`, or at the end of the page; headings must be unique (the page title counts) and plain text, so `add_passage under` can name them. `rename_heading {page, heading, to}`: the heading is part of passage IDs, so the new IDs of its passages are reported; the page title is renamed with `set_title`.

**Passages** (addressed by the viewer's passage IDs, e.g. `concepts/x.md#p-1a2b3c4d5e6f`)
- `add_passage {page, under | after, text, citations}`: `under` names a heading (matched ignoring case; the page title means the introduction) and appends after its last passage, or right after the heading; `after` inserts after that passage's paragraph or list item. A new passage is always its own paragraph.
- `replace_passage {passage, text, citations, drop_citations?}`: every citation of the old passage must still be present unless listed in `drop_citations`, so a rewrite cannot silently lose sources. A list item keeps its marker and nested items.
- `remove_passage {passage, reason}`: the reason goes to the `log.md` entry. A list item with nested items cannot be removed (remove the nested items first).
- `add_citation {passage, citation}`: appended to the passage as written (its formatting is kept).
- Text is one line of plain text: it must read back exactly as given (no headings, lists, quotes, code, HTML or emphasis; page links as `[[folder/page|label]]`), contain no raw citations (they go in `citations`) and fit in `limits.passage_chars`. Citations are written as `[[raw/…]]` or `[[raw/…|label]]` after the text, and each must resolve (`parse::check_citation`).
- Passage IDs come from `get_page`. An unknown ID (stale read) is an error; an unknown heading lists the page's headings. The outcome reports the new ID of every added or changed passage (`BatchOutcome::passages`); other passages keep theirs.

**Metadata**
- `set_metadata {page, key, values}`, `add_metadata {page, key, values}`, `remove_metadata {page, key, values?}`. Map entries use `relationship.<person>`. Symmetric labels are mirrored on the other page.

**Vocabulary**
- `declare_key {key, values, description, …}`, `declare_value {key, value, description}`, `declare_place {slug, title}`, `add_synonym {key, value, synonym}`.

## Validation rules

**Structural (in code)**
- Frontmatter parses, has every `required_keys` entry, and a scalar `title`; keys known to the vocabulary.
- Page path: `<folder>/<slug>.md` for its `type`, slug lowercase-hyphenated, unique.
- Body: only constructs the viewer indexes (headings, paragraphs, list items). Fenced/indented code, tables, blockquotes and HTML are rejected, as `SCHEMA.md` forbids code excerpts and the viewer would not index them.
- Every passage cites at least one raw source, except an introduction when `uncited_intro` is set, passages under a `summary_headings` heading, and passages that only link pages (navigation, as the viewer defines it).
- The editor writes citations as `[[raw/…]]` (the viewer also reads `^[raw/…]` provenance markers).
- Every citation resolves to an existing raw file and, when anchored, to exactly one message, block or heading in it, using the viewer's citation reader.
- Every `[[page]]` link resolves to a published page (or one created in the batch); every page has at least two outbound page links.
- `raw/` files are never changed; added raw paths do not exist yet.
- Passage IDs in operations exist in the current page (stale IDs fail).
- Page length warning above ~200 lines (a warning in the result, not a rejection).

**Vocabulary-driven**
- Keys and values exist (with suggestions otherwise); `single`, `only_on`, value kinds, references to people, organizations, places, pages and raw files.
- Symmetric relationships stay reciprocal; a `relationship` edge to a person who does not exist is rejected.
- `contested: true` requires `contradictions`.

**Batch-level**
- `agent` and `model` are present; the batch has at most `limits.ops_per_batch` operations.
- The batch is validated against the final state, so operations may depend on each other in order (create a person, then reference them).
- An empty batch, or one whose ops cancel out, is rejected.

## Bookkeeping

- **`index.md`**: one line per published page, `- [[<path without .md>|<Title>]] — <summary>`, in the section of its type (`## Entities`, `## Concepts`, `## Comparisons`, `## Queries`), sorted by path. The header line `> Last updated: <date> | Total pages: <n>` is updated. Other `index.md` text is preserved.
- **`log.md`**: one appended entry per batch, `## [<date>] <kind> | <summary>`, then `- By <agent> (<model>).`, the agent's `notes` as bullets, and a generated bullet listing the files added, changed and removed, and any vocabulary declared.
- **Dates**: `created` on new pages and `updated` on changed pages are set to today; agents cannot set them. (The clock is injectable for tests.)
- **Raw manifest**: entries `{path, bytes, sha256}` appended to the configured manifest, kept sorted by path.

## Atomicity and concurrency

- One batch at a time: an exclusive lock on `_meta/.leaf-editor/lock` (the standard library's `File::lock`).
- Writes: every new file content is written to a temporary file in the target directory (via `tempfile`), synced, then renamed into place. A journal in `_meta/.leaf-editor/` lists the batch's files with backups of their previous contents until the batch completes; if a rename fails, earlier ones are rolled back. On start, a leftover journal (a crash mid-batch) is rolled back before anything else.
- Sync runs after the lock is released; a sync failure is reported (the batch, already validated, stays applied).

## Testing

All tests use temporary wikis, never `~/wiki`. Required coverage:

1. Every rejected batch leaves every file unchanged (byte snapshot), for each rule.
2. Every applied batch leaves a wiki the viewer syncs with no errors or warnings, with the expected `index.md`, `log.md`, dates and manifest.
3. Raw immutability; anchor uniqueness; manifest checksums.
4. Stale passage IDs, dropped citations, symmetric relationships, references created in the same batch.
5. Suggestions: normalization, synonyms, typos; `declare_*` flows.
6. Crash simulation: interrupted batch rolled back on next start.
7. Round trip: `get_page` → edit → `get_page` keeps unrelated passage IDs stable.

A read-only rehearsal against a *copy* of `~/wiki` validates the bootstrap vocabulary before the editor is ever pointed at the real wiki.

## Implementation status

- **Slice 1, done** (read side): vocabulary, baseline and extensions, key/value checks with the dictionary (`Editor::check_key`, `Editor::check_value`: the spec's `suggest`), `Editor::list_metadata`, the conformance report (`Editor::conformance`).
- **Slice 2, done**: the batch engine (`Editor::check`, `Editor::apply`), `set_metadata`, `add_metadata`, `remove_metadata`, `declare_value`, `declare_place`, `declare_key`, `add_synonym`; automatic `updated` dates and `log.md` entries; lock, journal, atomic writes, crash recovery; index sync.
- **Slice 5, done**: `add_raw` (text or base64) with collection rules, source frontmatter, body checksums, anchor checks and the manifest.
- **Slice 4, done**: `create_page`, `set_title`, `set_summary`; `index.md` maintenance (sorted lines per section, the `> Last updated` header when present; a line that publishes several pages is never rewritten); `created` dates; every `[[page]]` link must resolve and a page needs at least two links to other pages (a problem the batch introduces is rejected, an existing one is a warning); pages created in a batch are visible to the rest of the batch. `add_heading` and `rename_heading`; `archive_page` and `restore_page`, which move files, so the commit layer can now remove files (journaled and rolled back like writes).
- **Slice 3, done**: `Editor::page` (the spec's `get_page`), `add_passage`, `replace_passage`, `remove_passage`, `add_citation`; `limits.passage_chars` and citation targets enforced for written passages; removals with their reasons in `log.md`.
- Every validation rule above is enforced, including `contested: true` needing `contradictions` and the page length warning (a warning above 200 lines, never a rejection).
- **Slice 6, done**: the CLI, `leaf editor …`, built with `--features editor`.
- **Slice 7, done**: `leaf-mcp` exposes `editor_check`, `editor_apply`, `editor_page`, `editor_metadata`, `editor_suggest` and `editor_conformance` behind its opt-in `editor` feature. Details: [ROADMAP.md](ROADMAP.md).
- Batch values may be any YAML scalar (`yes`, `2024`), read as text. Frontmatter is rewritten in the wiki's style: original key order, unchanged lists as they were written, changed lists inline when they fit in 80 characters and one indented item per line otherwise, unquoted dates, no folded scalars; comments inside frontmatter are not kept. The editor's lock and journal live in `_meta/.leaf-editor/`.

## Decisions

- **Uncited summaries** are allowed under `summary_headings`.
- **Agent identity**: batches name `agent` and `model`; both go into `log.md`.
- **Batch size**: at most 50 operations, 2,000 characters per passage, configurable in `limits`.
- **Raw collections** have their own rules in `raw_collections`.
- **Removal** archives pages (`archive_page`, reversible with `restore_page`); raw sources are never removed.

## Implementation slices

1. Viewer: public `parse` module. Editor crate skeleton; vocabulary format, baseline, extensions, loading, validation; conformance report; `list_metadata` and `suggest`.
2. Batch engine: in-memory apply, whole-state validation, lock, journal, atomic writes, sync; metadata operations.
3. Passage operations.
4. `create_page`, headings, titles, summaries, `archive_page`, `restore_page`; `index.md`, `log.md` and dates.
5. `add_raw` and the manifest.
6. CLI (`leaf editor check|apply|metadata|suggest|conformance|page`, behind the `editor` cargo feature).
7. MCP tools, with the viewer's (`leaf-mcp`, editor tools behind the `editor` feature).
