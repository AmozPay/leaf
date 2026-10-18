# Leaf core — design draft

> **Historical.** This is the original design draft. It is kept for its diagrams and rationale, but the current specifications are [SPEC.md](SPEC.md) (viewer) and [EDITOR-SPEC.md](EDITOR-SPEC.md) (editor), and the current state is in [ROADMAP.md](ROADMAP.md). Names here (for example a single `leaf-core` library) predate the split into `leaf-common`, `leaf-viewer` and `leaf-editor`.

Status at the time: proposed, not implemented. This document defines the portable Rust core; the CLI and MCP server are adapters, not dependencies of the core.

## Boundary and invariants

- The configured wiki root is the source of truth. The SQLite database is a rebuildable, local cache outside `raw/`.
- Only published Markdown under `entities/`, `concepts/`, `comparisons/`, and `queries/` is indexed by default. `_meta/`, `raw/`, and unlisted candidate evidence are excluded. A stricter published-only mode cross-checks `index.md` entries; this is the proposed default, so a draft in an allowed directory is not silently published.
- Split a page into paragraph-sized passages; preserve its title, heading, relative path, frontmatter quality signals, and `[[raw/...#^message-...]]` citations. Search results are excerpts, not entire pages.
- A query always has an output limit (`max_passages`, `max_chars`). An optional caller-provided token counter can enforce `max_tokens` later; the core must not claim tokenizer-exact bounds without one. Returned text **and** metadata count toward the output budget.
- No LLM, network, or transcript ingestion is needed to index or search. Source links are data, never executable instructions.
- Reading a raw citation is a separate API: accept only a citation discovered in an indexed page, canonicalize and confine the path to `wiki_root/raw/`, find the exact message anchor, and bound the returned excerpt. Do not read the whole transcript by default.

## Component diagram

```mermaid
flowchart LR
    CLI[CLI adapter] --> Core[Leaf core library]
    MCP[MCP adapter] --> Core
    Other[Future client] --> Core
    Core --> Scan[Published page scanner]
    Core --> Parse[Markdown and citation parser]
    Core --> Retrieve[FTS5 retriever and budgeter]
    Core --> Resolve[Explicit citation resolver]
    Scan --> Wiki[(Markdown wiki)]
    Parse --> Wiki
    Resolve --> Wiki
    Retrieve --> DB[(SQLite index)]
    Scan --> DB
    Parse --> DB
```

## Domain model (Rust-facing)

```mermaid
classDiagram
    class WikiConfig {
        +PathBuf wiki_root
        +PathBuf database_path
        +PublicationPolicy publication_policy
    }
    class WikiIndex {
        +open(config) Result~Self~
        +sync() Result~SyncReport~
        +search(request) Result~SearchResponse~
        +read_citation(request) Result~CitationExcerpt~
    }
    class Page {
        +String relative_path
        +String title
        +String content_hash
        +String confidence
        +bool contested
    }
    class Passage {
        +String stable_id
        +String page_path
        +String heading
        +String text
        +Vec~Citation~ citations
    }
    class Citation {
        +String raw_path
        +String message_id
        +String label
    }
    class SearchRequest {
        +String query
        +usize max_passages
        +usize max_chars
    }
    class SearchHit {
        +Passage passage
        +f64 rank
    }
    class SearchResponse {
        +Vec~SearchHit~ hits
        +bool truncated
        +usize chars_used
    }
    class CitationRequest {
        +Citation citation
        +usize max_chars
    }
    WikiIndex --> WikiConfig
    Page "1" *-- "0..*" Passage
    Passage "1" *-- "0..*" Citation
    SearchRequest --> WikiIndex
    SearchResponse "1" *-- "0..*" SearchHit
    SearchHit --> Passage
    CitationRequest --> Citation
```

`stable_id` should derive from page path plus a stable chunk locator, not SQLite row IDs. Exact Rust signatures and error types remain open until implementation. A result with insufficient budget returns no partial citation or misleading half-paragraph: it skips that passage and sets `truncated`.

## Persistent index

```mermaid
erDiagram
    PAGE ||--o{ PASSAGE : contains
    PASSAGE ||--o{ CITATION : supports
    PASSAGE ||--|| PASSAGE_FTS : searchable_as
    PAGE {
        string path PK
        string title
        string content_hash
        string confidence
        boolean contested
    }
    PASSAGE {
        string id PK
        string page_path FK
        string heading
        string body
        int ordinal
    }
    CITATION {
        string passage_id FK
        string raw_path
        string message_id
        string label
    }
    PASSAGE_FTS {
        string title
        string heading
        string body
    }
```

`PASSAGE_FTS` is an FTS5 virtual table, not a conventional SQL table. Implementation must keep its rows synchronized with `PASSAGE` in a transaction (via triggers or explicit writes), use bound SQL parameters, and convert free-text questions to safe FTS expressions rather than interpreting arbitrary query syntax. A schema version permits rebuilding the disposable index after upgrades.

## Incremental indexing

```mermaid
sequenceDiagram
    participant Client as CLI or MCP
    participant Core as WikiIndex
    participant Files as Published Markdown
    participant DB as SQLite FTS5
    Client->>Core: sync()
    Core->>Files: read index.md and allowed page paths
    loop Each published page
        Core->>Files: read page and hash content
        alt New or changed page
            Core->>Core: parse frontmatter, paragraphs, citations
            Core->>DB: replace page, passages, citations and FTS rows atomically
        else Unchanged
            Core->>Core: skip
        end
    end
    Core->>DB: remove pages no longer published
    Core-->>Client: SyncReport (added, updated, removed, skipped, errors)
```

Malformed pages should be reported with paths rather than indexed as unsupported facts. On parse failure, remove that page's indexed rows and mark sync incomplete so stale content is not returned; on deletion or unpublication, remove its rows as well. Never modify Markdown or raw sources.

## Search and citation drill-down

```mermaid
sequenceDiagram
    participant Client as Hermes or other client
    participant Core as WikiIndex
    participant DB as SQLite FTS5
    participant Raw as Raw Markdown
    Client->>Core: search(query, max_passages, max_chars)
    Core->>DB: ranked FTS5 candidates (title, heading, body)
    DB-->>Core: candidate passage IDs and ranks
    Core->>Core: deduplicate, apply limits, include source metadata
    Core-->>Client: SearchResponse (bounded excerpts and citations)
    opt Explicit verification of one returned citation
        Client->>Core: read_citation(citation, max_chars)
        Core->>DB: confirm citation exists on an indexed passage
        Core->>Raw: confined path and exact message anchor lookup
        Raw-->>Core: bounded source message excerpt
        Core-->>Client: CitationExcerpt or explicit error
    end
```

## Open decisions before implementation

1. Should the published-page allowlist require `index.md` links (recommended), or should directory membership alone count as published? The schema says the index is complete; strict mode makes that contract operational.
2. How should passages with several citations be split if their full text plus citation metadata exceeds `max_chars`? Initial recommendation: skip the passage, report truncation, never drop its citations silently.
3. Should a caller-specific tokenizer be accepted as a Rust callback, or should adapters translate model token budgets into conservative character caps? Start with character limits; add tokenizer support once an actual model/client requires it.
4. Ranking: start with FTS5 BM25 and modest title/heading boosts, then evaluate against real questions before adding embeddings or graph expansion.
