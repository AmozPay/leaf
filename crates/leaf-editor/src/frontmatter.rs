//! Editing a page's frontmatter while keeping the rest of the page intact.
//!
//! The frontmatter is read into an order-preserving map and written back
//! with `serde-saphyr` in the wiki's style: keys in their original order,
//! lists the edit did not change in the style they were written in, a
//! changed list inline (`tags: [a, b]`) when its line fits in 80 characters
//! and one item per line otherwise (long `sources:` lists), one-level maps
//! as blocks, YAML 1.2 scalars (dates unquoted). Comments inside the
//! frontmatter are not kept. The body after the frontmatter is untouched.

use indexmap::IndexMap;
use leaf_common::parse;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// A page split into its frontmatter and its body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Document {
    pub frontmatter: Frontmatter,
    /// Everything after the frontmatter block, as written.
    pub body: String,
}

impl Document {
    pub fn parse(content: &str) -> Result<Self, String> {
        let content = content.strip_prefix('\u{feff}').unwrap_or(content);
        let (range, yaml) =
            parse::frontmatter_block(content).ok_or("the page has no frontmatter")?;
        Ok(Self {
            frontmatter: Frontmatter::parse(&yaml)?,
            body: content[range.end..].to_string(),
        })
    }

    pub fn render(&self) -> String {
        format!("---\n{}---\n{}", self.frontmatter.render(), self.body)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Scalar(Value),
    List(serde_saphyr::FlowSeq<Vec<Value>>),
    Map(IndexMap<String, Node>),
}

impl Node {
    fn from_value(value: Value) -> Self {
        match value {
            Value::Array(items) => Self::List(serde_saphyr::FlowSeq(items)),
            Value::Object(map) => Self::Map(
                map.into_iter()
                    .map(|(k, v)| (k, Self::from_value(v)))
                    .collect(),
            ),
            other => Self::Scalar(other),
        }
    }

    /// The node's values as text (a scalar is one value, null none).
    fn texts(&self) -> Vec<String> {
        match self {
            Self::Scalar(Value::Null) => Vec::new(),
            Self::Scalar(value) => vec![text(value)],
            Self::List(list) => list.0.iter().filter(|v| !v.is_null()).map(text).collect(),
            Self::Map(map) => map.keys().cloned().collect(),
        }
    }
}

/// Lines longer than this hold their list one item per line.
const LINE: usize = 80;

/// A node with its list style chosen for writing.
#[derive(Serialize)]
#[serde(untagged)]
enum Styled<'a> {
    Scalar(&'a Value),
    Inline(serde_saphyr::FlowSeq<&'a Vec<Value>>),
    Block(&'a Vec<Value>),
    Map(IndexMap<&'a str, Styled<'a>>),
}

impl<'a> Styled<'a> {
    /// `path` is the key (`base.entry` in a map); `written` the inline
    /// (`true`) or block style of the lists the edit did not change.
    fn of(path: &str, key: &str, node: &'a Node, written: &HashMap<String, bool>) -> Self {
        let indent = if path.contains('.') { 2 } else { 0 };
        match node {
            Node::Scalar(value) => Self::Scalar(value),
            Node::List(list) => {
                let items: usize = list.0.iter().map(|v| text(v).len() + 2).sum();
                let fits = indent + key.len() + 4 + items <= LINE;
                if written.get(path).copied().unwrap_or(fits) {
                    Self::Inline(serde_saphyr::FlowSeq(&list.0))
                } else {
                    Self::Block(&list.0)
                }
            }
            Node::Map(map) => Self::Map(
                map.iter()
                    .map(|(entry, node)| {
                        let path = format!("{path}.{entry}");
                        (entry.as_str(), Self::of(&path, entry, node, written))
                    })
                    .collect(),
            ),
        }
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Frontmatter keys and values, in their written order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Frontmatter(IndexMap<String, Node>, HashMap<String, bool>);

/// How each list was written: inline (`key: [a, b]`, `true`) or one item per
/// line (`false`), by key path (`base.entry` one level down). Only the
/// layout of top-level keys and one-level map entries is read; the values
/// come from the YAML parser.
fn list_styles(yaml: &str) -> HashMap<String, bool> {
    let mut styles = HashMap::new();
    let mut base = None;
    let lines: Vec<&str> = yaml.lines().collect();
    for (n, line) in lines.iter().enumerate() {
        let depth = line.len() - line.trim_start_matches(' ').len();
        let Some((key, rest)) = line.trim().split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches(['"', '\'']);
        let path = match (depth, &base) {
            (0, _) => {
                base = Some(key.to_string());
                key.to_string()
            }
            (2, Some(base)) if !key.starts_with('-') => format!("{base}.{key}"),
            _ => continue,
        };
        let rest = rest.trim();
        if rest.starts_with('[') {
            styles.insert(path, true);
        } else if rest.is_empty()
            && lines
                .get(n + 1)
                .is_some_and(|next| next.trim_start().starts_with("- "))
        {
            styles.insert(path, false);
        }
    }
    styles
}

impl Frontmatter {
    /// A page's frontmatter with no keys yet.
    pub fn empty() -> Self {
        Self(IndexMap::new(), HashMap::new())
    }

    pub fn parse(yaml: &str) -> Result<Self, String> {
        let map: IndexMap<String, Value> = serde_saphyr::from_str(yaml).map_err(|err| {
            let message = err.to_string();
            format!(
                "invalid YAML frontmatter: {}",
                message.lines().next().unwrap_or_default()
            )
        })?;
        Ok(Self(
            map.into_iter()
                .map(|(key, value)| (key, Node::from_value(value)))
                .collect(),
            list_styles(yaml),
        ))
    }

    pub fn render(&self) -> String {
        let mut options = serde_saphyr::SerializerOptions::default();
        options.yaml_12 = true;
        options.no_lang_directive = true;
        // `sources:\n  - raw/…`, as LLM wikis write block lists.
        options.compact_list_indent = false;
        // Long values (paths, titles) stay on one line, not folded `>-`.
        options.prefer_block_scalars = false;
        let styled: IndexMap<&str, Styled> = self
            .0
            .iter()
            .map(|(key, node)| (key.as_str(), Styled::of(key, key, node, &self.1)))
            .collect();
        serde_saphyr::to_string_with_options(&styled, options).unwrap_or_default()
    }

    /// The written spelling of a key, matched ignoring case.
    fn find(map: &IndexMap<String, Node>, key: &str) -> Option<String> {
        let key = key.trim().to_lowercase();
        map.keys().find(|k| k.trim().to_lowercase() == key).cloned()
    }

    pub fn has(&self, key: &str) -> bool {
        match key.split_once('.') {
            Some((base, entry)) => match Self::find(&self.0, base).map(|b| &self.0[&b]) {
                Some(Node::Map(map)) => Self::find(map, entry).is_some(),
                _ => false,
            },
            None => Self::find(&self.0, key).is_some(),
        }
    }

    /// The values of `key`, or of the map entry `base.entry`.
    pub fn values(&self, key: &str) -> Vec<String> {
        match key.split_once('.') {
            Some((base, entry)) => match Self::find(&self.0, base).map(|b| &self.0[&b]) {
                Some(Node::Map(map)) => Self::find(map, entry)
                    .map(|e| map[&e].texts())
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            None => Self::find(&self.0, key)
                .map(|k| self.0[&k].texts())
                .unwrap_or_default(),
        }
    }

    /// Set `key` (or `base.entry`) to `values`: one scalar when `single`,
    /// else a list (an empty list stays as `[]`).
    pub fn set(&mut self, key: &str, values: Vec<String>, single: bool) {
        let node = if single {
            Node::Scalar(values.into_iter().next().map_or(Value::Null, Value::String))
        } else {
            Node::List(serde_saphyr::FlowSeq(
                values.into_iter().map(Value::String).collect(),
            ))
        };
        self.set_node(key, node);
    }

    /// Set a boolean scalar (`contested: true`).
    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.set_node(key, Node::Scalar(Value::Bool(value)));
    }

    fn set_node(&mut self, key: &str, node: Node) {
        match key.split_once('.') {
            Some((base, entry)) => {
                let base_key = Self::find(&self.0, base).unwrap_or_else(|| base.to_string());
                let map = self
                    .0
                    .entry(base_key.clone())
                    .or_insert_with(|| Node::Map(IndexMap::new()));
                if !matches!(map, Node::Map(_)) {
                    *map = Node::Map(IndexMap::new());
                }
                if let Node::Map(map) = map {
                    let entry_key = Self::find(map, entry).unwrap_or_else(|| entry.to_string());
                    self.1.remove(&format!("{base_key}.{entry_key}"));
                    map.insert(entry_key, node);
                }
            }
            None => {
                let written = Self::find(&self.0, key).unwrap_or_else(|| key.to_string());
                self.1.remove(&written);
                self.0.insert(written, node);
            }
        }
    }

    /// Remove `key`, or the map entry `base.entry` (and the map once empty).
    pub fn remove(&mut self, key: &str) {
        match key.split_once('.') {
            Some((base, entry)) => {
                let Some(base_key) = Self::find(&self.0, base) else {
                    return;
                };
                let empty = match self.0.get_mut(&base_key) {
                    Some(Node::Map(map)) => {
                        if let Some(entry_key) = Self::find(map, entry) {
                            map.shift_remove(&entry_key);
                        }
                        map.is_empty()
                    }
                    _ => false,
                };
                if empty {
                    self.0.shift_remove(&base_key);
                }
            }
            None => {
                if let Some(written) = Self::find(&self.0, key) {
                    self.0.shift_remove(&written);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\ntitle: \"Garden: notes\"\ncreated: 2026-10-01\ntags: [Gardening, 2024]\n\
related-places: []\nrelationship:\n  bob-martin: [friend]\nsources:\n  - raw/notes/a.md\n---\n# Garden\n\n\
Body text with --- and **markup**.\n";

    #[test]
    fn untouched_documents_keep_their_meaning_and_body() {
        let doc = Document::parse(PAGE).unwrap();
        assert!(doc.body.starts_with("# Garden\n"));
        let rendered = doc.render();
        assert_eq!(
            rendered,
            "---\ntitle: \"Garden: notes\"\ncreated: 2026-10-01\ntags: [Gardening, 2024]\n\
             related-places: []\nrelationship:\n  bob-martin: [friend]\nsources:\n  - raw/notes/a.md\n---\n# Garden\n\n\
             Body text with --- and **markup**.\n"
        );
        // An unchanged list keeps its style even past 80 characters.
        let wide = "---\ntitle: Wide\ntags: [irrigation, seeds, tools, composting, workshop, gardens]\n---\n";
        assert_eq!(Document::parse(wide).unwrap().render(), wide);
        // A list too long for one line is written one item per line.
        let long: Vec<String> = (1..=4)
            .map(|n| format!("raw/notes/a-very-long-source-name-that-exceeds-eighty-characters-on-its-own-{n}.md"))
            .collect();
        let mut doc = doc;
        doc.frontmatter.set("sources", long, false);
        assert!(doc.render().contains(
            "sources:\n  - raw/notes/a-very-long-source-name-that-exceeds-eighty-characters-on-its-own-1.md\n"
        ), "{}", doc.render());
        // The viewer reads the same metadata from both.
        assert_eq!(
            parse::page(&rendered).unwrap().facets,
            parse::page(PAGE).unwrap().facets
        );
    }

    #[test]
    fn keys_and_map_entries_are_set_and_removed_in_place() {
        let mut doc = Document::parse(PAGE).unwrap();
        let fm = &mut doc.frontmatter;
        assert_eq!(fm.values("Tags"), ["Gardening", "2024"]);
        assert_eq!(fm.values("relationship.bob-martin"), ["friend"]);
        assert!(fm.has("related-places") && fm.values("related-places").is_empty());
        fm.set("tags", vec!["gardening".into()], false);
        fm.set("confidence", vec!["medium".into()], true);
        fm.set("relationship.alice", vec!["colleague".into()], false);
        fm.set_bool("contested", true);
        fm.set("updated", vec!["2026-10-17".into()], true);
        fm.remove("relationship.bob-martin");
        fm.remove("related-places");
        let rendered = doc.render();
        assert!(
            rendered.starts_with(
                "---\ntitle: \"Garden: notes\"\ncreated: 2026-10-01\ntags: [gardening]\n\
             relationship:\n  alice: [colleague]\nsources:\n  - raw/notes/a.md\nconfidence: medium\n\
             contested: true\nupdated: 2026-10-17\n---\n"
            ),
            "{rendered}"
        );
        let mut emptied = Document::parse(&rendered).unwrap();
        emptied.frontmatter.remove("relationship.alice");
        assert!(!emptied.frontmatter.has("relationship"));
    }

    #[test]
    fn pages_without_frontmatter_are_refused() {
        assert!(Document::parse("# Title\n").is_err());
        assert!(Document::parse("---\ntitle: [\n---\n").is_err());
    }
}
