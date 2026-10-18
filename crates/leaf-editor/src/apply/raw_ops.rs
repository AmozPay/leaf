//! Raw sources: adding files under `raw/<collection>/` and recording them in
//! the checksum manifest. Existing raw files are never changed or removed.

use super::{Planner, Step, Stop};
use crate::commit::Write;
use crate::error::{Error, Result};
use crate::frontmatter::{Document, Frontmatter};
use crate::vocabulary::AnchorKind;
use base64::Engine as _;
use leaf_common::publication::is_plain_relative;
use leaf_common::{Anchor, Citation, parse};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Bytes sniffed to decide the media type, as the viewer does.
const SNIFF_BYTES: usize = 8 * 1024;

/// The rules of the collection a raw file goes into.
struct Collection {
    folder: String,
    media: Vec<String>,
    require_frontmatter: Vec<String>,
    require_anchor: Option<AnchorKind>,
}

/// What an `add_raw` operation gives: the file's content and Markdown fields.
pub(super) struct RawFile<'a> {
    pub path: &'a str,
    pub text: Option<&'a str>,
    pub base64: Option<&'a str>,
    pub source: Option<&'a str>,
    pub source_url: Option<&'a str>,
}

impl Planner<'_> {
    pub(super) fn add_raw(&mut self, number: usize, file: &RawFile) -> Step {
        let (path, rules) = self.raw_target(number, file.path)?;
        let bytes = self.raw_bytes(number, file)?;
        let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
        let (media, is_text) = parse::media_type(&path, head);
        if !rules.media.is_empty() && !rules.media.contains(&media) {
            return Err(self.reject_with(
                number,
                format!(
                    "`{}` takes {}, not {media}",
                    rules.folder,
                    rules.media.join(", ")
                ),
                rules.media.clone(),
            ));
        }
        let is_markdown = is_text && media == "text/markdown";
        if !is_markdown && (file.source.is_some() || file.source_url.is_some()) {
            return Err(self.reject(
                number,
                "`source` and `source_url` go in a Markdown file's frontmatter",
            ));
        }
        let bytes = if is_markdown {
            let Ok(content) = String::from_utf8(bytes) else {
                return Err(self.reject(number, "the text is not valid UTF-8"));
            };
            self.markdown_source(number, &content, file, &rules.require_frontmatter)?
                .into_bytes()
        } else {
            bytes
        };
        // Anchors in any text must be unique and citable; the collection may
        // also require some.
        match (std::str::from_utf8(&bytes), is_text) {
            (Ok(content), true) => {
                let required = rules.require_anchor.unwrap_or(AnchorKind::Any);
                self.check_anchors(number, &path, content, required)?;
            }
            _ if rules.require_anchor.is_some() => {
                return Err(self.reject(
                    number,
                    format!("`{}` needs text with anchors, not {media}", rules.folder),
                ));
            }
            _ => {}
        }
        self.raw.paths.push(path.clone());
        self.raw.content.insert(path, bytes);
        Ok(())
    }

    /// The path of a new raw file and the rules of its collection.
    fn raw_target(&mut self, number: usize, path: &str) -> Step<(String, Collection)> {
        let path = path.trim();
        let segments: Vec<&str> = path.split('/').collect();
        let file = segments.last().copied().unwrap_or_default();
        if segments.len() < 3
            || segments[0] != "raw"
            || !is_plain_relative(path)
            || file.starts_with('.')
            || !file.contains('.')
        {
            return Err(self.reject(
                number,
                format!("`{path}` must be `raw/<collection>/<file>` with an extension"),
            ));
        }
        if path != path.to_lowercase() {
            return Err(self.reject_with(
                number,
                format!("`{path}` must be lowercase"),
                vec![path.to_lowercase()],
            ));
        }
        let folder = format!("raw/{}/", segments[1]);
        let Some(rules) = self.vocabulary.raw_collections.get(segments[1]) else {
            let declared = self.vocabulary.raw_collections.keys().cloned().collect();
            return Err(self.reject_with(
                number,
                format!("`{folder}` is not a declared raw collection"),
                declared,
            ));
        };
        if !rules.addable {
            return Err(self.reject(number, format!("`{folder}` does not take new files")));
        }
        let rules = Collection {
            folder,
            media: rules.media.clone(),
            require_frontmatter: rules.require_frontmatter.clone(),
            require_anchor: rules.require_anchor,
        };
        if self.editor.root.join(path).exists() || self.raw.content.contains_key(path) {
            return Err(self.reject(
                number,
                format!("`{path}` already exists; raw files are never changed"),
            ));
        }
        Ok((path.to_string(), rules))
    }

    /// The file's bytes: its `text`, or its decoded `base64`.
    fn raw_bytes(&mut self, number: usize, file: &RawFile) -> Step<Vec<u8>> {
        match (file.text, file.base64) {
            (Some(text), None) => Ok(text.as_bytes().to_vec()),
            (None, Some(encoded)) => {
                let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim());
                decoded.map_err(|err| self.reject(number, format!("`base64` is invalid: {err}")))
            }
            _ => Err(self.reject(number, "give the file as either `text` or `base64`")),
        }
    }

    /// A Markdown source with `source`, `source_url`, `ingested` and the
    /// `sha256` of its body in its frontmatter.
    fn markdown_source(
        &mut self,
        number: usize,
        content: &str,
        file: &RawFile,
        required: &[String],
    ) -> Step<String> {
        let mut doc = if parse::frontmatter_block(content).is_some() {
            let parsed = Document::parse(content);
            self.check(number, parsed)?
        } else {
            Document {
                frontmatter: Frontmatter::empty(),
                body: content.to_string(),
            }
        };
        for key in ["ingested", "sha256"] {
            if doc.frontmatter.has(key) {
                return Err(self.reject(
                    number,
                    format!("`{key}` is set by the editor, not by the text"),
                ));
            }
        }
        for (key, value) in [("source", file.source), ("source_url", file.source_url)] {
            let Some(value) = value.map(str::trim) else {
                continue;
            };
            if value.is_empty() || value.contains('\n') {
                return Err(self.reject(number, format!("`{key}` must be one line of text")));
            }
            if doc.frontmatter.has(key) {
                return Err(self.reject(
                    number,
                    format!("`{key}` is given twice: in the text's frontmatter and as a field"),
                ));
            }
            doc.frontmatter.set(key, vec![value.to_string()], true);
        }
        doc.frontmatter
            .set("ingested", vec![self.editor.today().to_string()], true);
        let digest = Sha256::digest(doc.body.as_bytes());
        doc.frontmatter
            .set("sha256", vec![format!("{digest:x}")], true);
        let missing: Vec<String> = required
            .iter()
            .filter(|key| !doc.frontmatter.has(key))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(self.reject_with(
                number,
                format!(
                    "this collection requires the frontmatter {}; give them (`source`, `source_url`, or in the text's frontmatter)",
                    missing.join(", ")
                ),
                missing,
            ));
        }
        Ok(doc.render())
    }

    /// Anchors must be unique and citable, and of the kind the collection
    /// requires. Every problem is reported before the step is rejected.
    fn check_anchors(
        &mut self,
        number: usize,
        path: &str,
        content: &str,
        required: AnchorKind,
    ) -> Step {
        let anchors = parse::anchors(content);
        let mut ok = true;
        let mut seen: Vec<String> = Vec::new();
        for anchor in &anchors {
            let written = anchor.to_string();
            if seen.contains(&written) {
                self.error(
                    Some(number),
                    format!("the anchor `{written}` occurs more than once in `{path}`"),
                    Vec::new(),
                );
                ok = false;
                continue;
            }
            seen.push(written);
            let citable = format!("{path}#{anchor}")
                .parse::<Citation>()
                .map_err(|err| err.to_string())
                .and_then(|citation| {
                    parse::check_citation_text(&citation, path.ends_with(".md"), content)
                });
            if let Err(message) = citable {
                self.error(
                    Some(number),
                    format!("the anchor `{anchor}` in `{path}` cannot be cited: {message}"),
                    Vec::new(),
                );
                ok = false;
            }
        }
        let wanted = match required {
            AnchorKind::Message if !anchors.iter().any(|a| matches!(a, Anchor::Message(_))) => {
                Some("message anchor (`^message-<id>`)")
            }
            AnchorKind::Block if !anchors.iter().any(|a| matches!(a, Anchor::Block(_))) => {
                Some("block anchor (`^<id>`)")
            }
            _ => None,
        };
        if let Some(wanted) = wanted {
            self.error(
                Some(number),
                format!("this collection requires at least one {wanted}"),
                Vec::new(),
            );
            ok = false;
        }
        if ok { Ok(()) } else { Err(Stop::Rejected) }
    }

    /// The raw files to write, and the manifest with their entries added.
    pub(super) fn raw_writes(&mut self) -> Result<Vec<Write>> {
        if self.raw.content.is_empty() {
            return Ok(Vec::new());
        }
        let mut writes: Vec<Write> = self
            .raw
            .content
            .iter()
            .map(|(path, bytes)| Write::file(self.editor.root.join(path), bytes.clone()))
            .collect();
        self.outcome
            .changed
            .extend(self.raw.content.keys().cloned());
        let Some(manifest) = self.vocabulary.raw_manifest.clone() else {
            return Ok(writes);
        };
        let file = self.editor.root.join(&manifest);
        let mut json = match std::fs::read_to_string(&file) {
            Ok(text) => {
                match serde_json::from_str::<Value>(&text) {
                    Ok(json) if json.get("files").is_some_and(Value::is_array) => json,
                    _ => {
                        self.error(
                        None,
                        format!("the raw manifest `{manifest}` is not a JSON object with a `files` list"),
                        Vec::new(),
                    );
                        return Ok(Vec::new());
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({"files": []}),
            Err(err) => return Err(Error::io(&file, err)),
        };
        let files = json["files"].as_array_mut().expect("checked");
        for (path, bytes) in &self.raw.content {
            let digest = Sha256::digest(bytes);
            let entry =
                json!({"path": path, "bytes": bytes.len(), "sha256": format!("{digest:x}")});
            // In path order: before the first entry that sorts after it.
            let at = files
                .iter()
                .position(|f| {
                    f.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|p| p > path.as_str())
                })
                .unwrap_or(files.len());
            files.insert(at, entry);
        }
        let mut text = serde_json::to_string_pretty(&json).expect("a JSON value serializes");
        text.push('\n');
        writes.push(Write::file(file, text.into_bytes()));
        self.outcome.changed.push(manifest);
        Ok(writes)
    }
}
