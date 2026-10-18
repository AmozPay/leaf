//! Passage operations of the planner: adding, replacing, removing and citing
//! passages by their IDs.

use super::{Planner, Step, Stop};
use crate::batch::PassageId;
use crate::passages::{self, Located};
use leaf_common::{Citation, parse};

impl Planner<'_> {
    /// Whether a citation's target exists: on disk, or among the raw files
    /// this batch adds.
    fn check_citation(&self, citation: &Citation) -> std::result::Result<(), String> {
        let Some(bytes) = self.raw.content.get(&citation.raw_path) else {
            return parse::check_citation(&self.editor.root, citation);
        };
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                parse::check_citation_text(citation, citation.raw_path.ends_with(".md"), text)
            }
            Err(_) if citation.anchor.is_none() => Ok(()),
            Err(_) => Err(format!(
                "{} is not text, so an anchor cannot be located",
                citation.raw_path
            )),
        }
    }

    /// Parse and check citation references: each must name an existing raw
    /// file and, if anchored, one message, block or heading in it. Every bad
    /// one is reported, then the operation is rejected.
    pub(super) fn citations(
        &mut self,
        number: usize,
        references: &[String],
    ) -> Step<Vec<Citation>> {
        let mut citations: Vec<Citation> = Vec::new();
        let mut ok = true;
        for reference in references {
            let checked = reference
                .parse::<Citation>()
                .map_err(|err| err.to_string())
                .and_then(|citation| self.check_citation(&citation).map(|()| citation));
            match checked {
                Ok(citation) => {
                    if !citations
                        .iter()
                        .any(|c| c.to_string() == citation.to_string())
                    {
                        citations.push(citation);
                    }
                }
                Err(message) => {
                    ok = false;
                    self.error(
                        Some(number),
                        format!("citation `{reference}`: {message}"),
                        Vec::new(),
                    );
                }
            }
        }
        if ok {
            Ok(citations)
        } else {
            Err(Stop::Rejected)
        }
    }

    /// The page and passage an ID (`<page>#p-<hash>`) names.
    pub(super) fn passage(&mut self, number: usize, id: &str) -> Step<(String, Located)> {
        let Some((page, locator)) = id.trim().split_once('#') else {
            return Err(self.reject(
                number,
                format!("`{id}` is not a passage ID (`<page>#p-<hash>`)"),
            ));
        };
        let path = self.page(number, page)?;
        let found = passages::find(self.doc(&path), locator);
        match self.in_page(number, &path, found)? {
            Some(located) => Ok((path, located)),
            None => Err(self.reject(
                number,
                format!("passage `{id}` not found: the page may have changed since it was read; read it again"),
            )),
        }
    }

    /// Report the ID a passage has after an operation.
    pub(super) fn record(
        &mut self,
        number: usize,
        path: &str,
        heading: &str,
        text: &str,
        citations: &[Citation],
    ) {
        if let Ok(Some(locator)) = passages::locator_of(self.doc(path), heading, text, citations) {
            self.outcome.passages.push(PassageId {
                op: number,
                id: format!("{path}#{locator}"),
            });
        }
    }

    /// Checked passage text.
    pub(super) fn text(
        &mut self,
        number: usize,
        text: &str,
        citations: &[Citation],
    ) -> Step<String> {
        let checked = passages::check_text(text, citations, self.vocabulary.limits.passage_chars);
        self.check(number, checked)
    }

    pub(super) fn add_passage(
        &mut self,
        number: usize,
        page: &str,
        under: Option<&str>,
        after: Option<&str>,
        text: &str,
        citations: &[String],
    ) -> Step {
        let path = self.page(number, page)?;
        let citations = self.citations(number, citations)?;
        let text = self.text(number, text, &citations)?;
        let (at, heading) = match (under, after) {
            (Some(heading), None) => {
                let found = passages::end_of_heading(self.doc(&path), heading);
                match self.in_page(number, &path, found)? {
                    Ok(place) => place,
                    Err(headings) => {
                        return Err(self.reject_with(
                            number,
                            format!("`{path}` has no single heading `{heading}` (use one of its headings, or `after`)"),
                            headings,
                        ));
                    }
                }
            }
            (None, Some(id)) => {
                let (other, located) = self.passage(number, id)?;
                if other != path {
                    return Err(self.reject(number, format!("passage `{id}` is not on `{path}`")));
                }
                (located.block.end, located.heading)
            }
            _ => {
                return Err(self.reject(
                    number,
                    "give either `under` (a heading, or the page title) or `after` (a passage ID)",
                ));
            }
        };
        let markdown = passages::markdown(&text, &citations);
        passages::insert(
            &mut self.pages.loaded.get_mut(&path).expect("staged").doc,
            at,
            &markdown,
        );
        self.record(number, &path, &heading, &text, &citations);
        Ok(())
    }

    pub(super) fn replace_passage(
        &mut self,
        number: usize,
        passage: &str,
        text: &str,
        citations: &[String],
        drop_citations: &[String],
    ) -> Step {
        let (path, located) = self.passage(number, passage)?;
        let citations = self.citations(number, citations)?;
        let had: Vec<String> = located.citations.iter().map(|c| c.to_string()).collect();
        let mut dropped = Vec::new();
        for reference in drop_citations {
            match reference.parse::<Citation>() {
                Ok(citation) if had.contains(&citation.to_string()) => {
                    dropped.push(citation.to_string())
                }
                _ => {
                    return Err(self.reject_with(
                        number,
                        format!("`{reference}` is not cited by `{passage}`"),
                        had.clone(),
                    ));
                }
            }
        }
        let kept: Vec<String> = citations.iter().map(|c| c.to_string()).collect();
        let lost: Vec<&String> = had
            .iter()
            .filter(|c| !kept.contains(c) && !dropped.contains(c))
            .collect();
        if !lost.is_empty() {
            let lost: Vec<String> = lost.into_iter().cloned().collect();
            return Err(self.reject_with(number, format!(
                    "the new text would drop the citations {}; keep them in `citations` or list them in `drop_citations`",
                    lost.join(", ")
                ), lost));
        }
        let text = self.text(number, text, &citations)?;
        let markdown = passages::markdown(&text, &citations);
        passages::replace(
            &mut self.pages.loaded.get_mut(&path).expect("staged").doc,
            located.span.clone(),
            &markdown,
        );
        self.record(number, &path, &located.heading, &text, &citations);
        Ok(())
    }

    pub(super) fn remove_passage(&mut self, number: usize, passage: &str, reason: &str) -> Step {
        if reason.trim().is_empty() || reason.contains('\n') {
            return Err(self.reject(number, "give a one-line `reason` for removing a passage"));
        }
        let (path, located) = self.passage(number, passage)?;
        let holds = passages::holds_others(self.doc(&path), &located);
        if self.in_page(number, &path, holds)? {
            return Err(self.reject(
                number,
                format!("`{passage}` has nested items; remove them first"),
            ));
        }
        passages::remove(self.doc_mut(&path), &located);
        self.removed_passages
            .push(format!("`{}` ({})", passage.trim(), reason.trim()));
        Ok(())
    }

    pub(super) fn add_citation(&mut self, number: usize, passage: &str, citation: &str) -> Step {
        let (path, located) = self.passage(number, passage)?;
        let added = self.citations(number, &[citation.to_string()])?;
        let added = added.into_iter().next().expect("one citation");
        if located
            .citations
            .iter()
            .any(|c| c.to_string() == added.to_string())
        {
            return Err(self.reject(number, format!("`{passage}` already cites `{added}`")));
        }
        // Append to the passage as written, so its own formatting stays.
        let doc = self.doc_mut(&path);
        let written = doc.body[located.span.clone()].trim_end().to_string();
        passages::replace(doc, located.span.clone(), &format!("{written} [[{added}]]"));
        let mut citations = located.citations.clone();
        citations.push(added);
        self.record(number, &path, &located.heading, &located.text, &citations);
        Ok(())
    }
}
