The batch, as YAML or JSON text. It is checked as a whole: one problem rejects it, nothing is written, and the errors say what to fix.

Header: `agent` (who writes; recorded in log.md), `model`, `kind` (update | correction | review | merge), `summary` (one line; the log.md heading), optional `notes` (list of lines), `ops` (at most 50).

Operations (`?` is optional):
- `set_metadata`, `add_metadata`, `remove_metadata` {page, key, values?}. Map entries are `relationship.<person-slug>`; symmetric labels (friend, colleague) are mirrored on the other page.
- `add_passage` {page, under | after, text, citations}: `under` is a heading (or the page title for the introduction), `after` a passage id.
- `replace_passage` {passage, text, citations, drop_citations?}: every old citation must stay unless dropped. `remove_passage` {passage, reason}. `add_citation` {passage, citation}.
- `create_page` {type, slug, title, summary, metadata, intro?, sections?}: type is entity | concept | comparison | query; metadata needs the required keys (tags, confidence) and `sources` is filled from the citations; intro is a list of {text, citations?}; sections a list of {heading, passages: [{text, citations}]}.
- `set_title` {page, title}, `set_summary` {page, summary}, `add_heading` {page, heading, level?, after?}, `rename_heading` {page, heading, to}.
- `archive_page` {page, reason} (refused while other pages still link to it or name it), `restore_page` {page, summary?}. Pages are never deleted.
- `add_raw` {path, text | base64, source?, source_url?}: path `raw/<collection>/<file>`, lowercase, new. Raw files are never changed afterwards. Make it citable: end the paragraphs you will cite with `^id`, or cite by heading.
- `declare_value` {key, value, description?}, `declare_place` {place, title?}, `declare_key` {key, values, description}, `add_synonym` {key, value, synonym}: new tags, places, keys and spellings must be declared before use.

Rules:
- A passage is one line of plain text (no headings, lists, code or emphasis; page links as `[[folder/page|label]]`), at most 2000 characters. Citations go in `citations` (`raw/<path>#^<id>`, `raw/<path>#^message-<id>` or `raw/<path>#<Heading>`), never in the text.
- Every passage cites a raw source, except the introduction under a page title.
- Every page links to at least two other pages, and every link must resolve. In an empty wiki the first batch therefore creates at least three pages that link to each other.
- Dates, `index.md` and `log.md` are the editor's. To change a passage use its id from `editor_page`; an id from an older read is rejected.
- Operations see the ones before them, so a batch can declare a tag, add a source, and create the page that uses both.

Example:

```yaml
agent: assistant
model: model-name
kind: update
summary: Greenhouse watering
ops:
  - op: add_raw
    path: raw/articles/greenhouse-watering.md
    source_url: https://example.org/greenhouse-watering
    text: |
      # Greenhouse watering

      Water seedlings every morning. ^morning
  - op: create_page
    type: concept
    slug: watering
    title: Watering
    summary: How and when to water a greenhouse
    metadata: {tags: [gardening], confidence: medium}
    intro:
      - text: Watering keeps seedlings alive. See [[concepts/light|light]] and [[concepts/soil|soil]].
    sections:
      - heading: Practice
        passages:
          - text: Seedlings are watered every morning.
            citations: ["raw/articles/greenhouse-watering.md#^morning"]
  - op: add_passage
    page: concepts/light
    under: Practice
    text: Light levels change how much water seedlings need.
    citations: ["raw/articles/greenhouse-watering.md#^morning"]
```
