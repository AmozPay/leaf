//! Maintaining `index.md`: one line per published page,
//! `- [[<path without .md>|<Title>]] — <summary>`, in the section of its
//! page type, sorted by path, and the `> Last updated: <date> | Total pages:
//! <n>` header. Every other line is kept as written.
//!
//! Which page a line publishes is decided by the viewer's own link rules
//! (`parse::published_pages`), so a line in `[Label](folder/page.md)` form
//! is found too. A line that publishes several pages is never rewritten.

use leaf_common::parse;

const HEADER: &str = "> Last updated:";

/// The text of `index.md`, line by line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexDoc {
    lines: Vec<String>,
}

/// What a page's line says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The link label, when the line has one.
    pub title: Option<String>,
    /// The text after ` — `, empty when there is none.
    pub summary: String,
}

impl IndexDoc {
    pub fn parse(text: &str) -> Self {
        Self {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    pub fn render(&self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');
        text
    }

    /// The line number publishing `path`; an error if that line publishes
    /// other pages too (it cannot be rewritten without losing them).
    fn find(&self, path: &str) -> Result<Option<usize>, String> {
        for (number, line) in self.lines.iter().enumerate() {
            let pages = parse::published_pages(line).0;
            if pages.contains(path) {
                if pages.len() > 1 {
                    return Err(format!(
                        "index.md lists several pages on the line `{}`; put each page on its own line first",
                        line.trim()
                    ));
                }
                return Ok(Some(number));
            }
        }
        Ok(None)
    }

    /// The title and summary written for `path`, if it has a line.
    pub fn entry(&self, path: &str) -> Result<Option<Entry>, String> {
        Ok(self.find(path)?.map(|number| entry_of(&self.lines[number])))
    }

    /// Write the line for `path`: in place when it has one, else sorted by
    /// path into `section` (created at the end when missing).
    pub fn set(
        &mut self,
        path: &str,
        section: &str,
        title: &str,
        summary: &str,
    ) -> Result<(), String> {
        let line = format_entry(path, title, summary);
        if let Some(number) = self.find(path)? {
            self.lines[number] = line;
            return Ok(());
        }
        let Some(start) = self.section(section) else {
            if self
                .lines
                .last()
                .is_some_and(|last| !last.trim().is_empty())
            {
                self.lines.push(String::new());
            }
            self.lines.push(format!("## {section}"));
            self.lines.push(String::new());
            self.lines.push(line);
            return Ok(());
        };
        let end = self.section_end(start);
        let entries: Vec<(usize, String)> = (start + 1..end)
            .filter_map(|number| {
                parse::published_pages(&self.lines[number])
                    .0
                    .into_iter()
                    .next()
                    .map(|page| (number, page))
            })
            .collect();
        let at = match entries.iter().find(|(_, page)| page.as_str() > path) {
            Some((number, _)) => *number,
            None => match entries.last() {
                Some((number, _)) => number + 1,
                None => {
                    // An empty section: after its heading and a blank line.
                    let mut at = start + 1;
                    if self.lines.get(at).is_some_and(|l| l.trim().is_empty()) {
                        at += 1;
                    } else {
                        self.lines.insert(at, String::new());
                        at += 1;
                    }
                    at
                }
            },
        };
        self.lines.insert(at, line);
        if self.lines.get(at + 1).is_some_and(|l| l.starts_with('#')) {
            self.lines.insert(at + 1, String::new());
        }
        Ok(())
    }

    /// Remove the line publishing `path`; whether there was one.
    pub fn remove(&mut self, path: &str) -> Result<bool, String> {
        match self.find(path)? {
            Some(number) => {
                self.lines.remove(number);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Update the `> Last updated:` header, if the index has one.
    pub fn set_header(&mut self, date: &str, total: usize) {
        if let Some(line) = self.lines.iter_mut().find(|l| l.starts_with(HEADER)) {
            *line = format!("{HEADER} {date} | Total pages: {total}");
        }
    }

    fn section(&self, name: &str) -> Option<usize> {
        self.lines.iter().position(|line| {
            line.strip_prefix("## ")
                .is_some_and(|heading| heading.trim().eq_ignore_ascii_case(name))
        })
    }

    fn section_end(&self, start: usize) -> usize {
        (start + 1..self.lines.len())
            .find(|&number| self.lines[number].starts_with('#'))
            .unwrap_or(self.lines.len())
    }
}

fn format_entry(path: &str, title: &str, summary: &str) -> String {
    let stem = path.strip_suffix(".md").unwrap_or(path);
    match summary.trim() {
        "" => format!("- [[{stem}|{title}]]"),
        summary => format!("- [[{stem}|{title}]] — {summary}"),
    }
}

fn entry_of(line: &str) -> Entry {
    let title = line.split_once("[[").and_then(|(_, rest)| {
        let inner = rest.split_once("]]")?.0;
        inner
            .split_once('|')
            .map(|(_, label)| label.trim().to_string())
    });
    let title = title.or_else(|| {
        let rest = line.split_once('[')?.1;
        let label = rest.split_once("](")?.0;
        Some(label.trim().to_string())
    });
    let summary = line
        .split_once(" — ")
        .map(|(_, summary)| summary.trim().to_string())
        .unwrap_or_default();
    Entry {
        title: title.filter(|t| !t.is_empty()),
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "# Wiki Index\n\n> Last updated: 2026-10-01 | Total pages: 3\n\n\
        ## Concepts\n\n- [[concepts/beds|Beds]] — Raised beds\n- [[concepts/soil|Soil]]\n\n\
        ## Entities\n\n- [[entities/alice|Alice]] — A gardener\n\nFooter note.\n";

    #[test]
    fn lines_are_inserted_sorted_and_the_rest_is_kept() {
        let mut index = IndexDoc::parse(INDEX);
        index
            .set(
                "concepts/compost.md",
                "Concepts",
                "Compost",
                "Rotting things",
            )
            .unwrap();
        index
            .set("concepts/zones.md", "Concepts", "Zones", "")
            .unwrap();
        index
            .set("concepts/aphids.md", "Concepts", "Aphids", "")
            .unwrap();
        index.set_header("2026-10-17", 6);
        let text = index.render();
        let concepts: Vec<&str> = text
            .lines()
            .filter(|l| l.starts_with("- [[concepts/"))
            .collect();
        assert_eq!(
            concepts,
            [
                "- [[concepts/aphids|Aphids]]",
                "- [[concepts/beds|Beds]] — Raised beds",
                "- [[concepts/compost|Compost]] — Rotting things",
                "- [[concepts/soil|Soil]]",
                "- [[concepts/zones|Zones]]",
            ]
        );
        assert!(text.contains("> Last updated: 2026-10-17 | Total pages: 6\n"));
        assert!(text.ends_with("Footer note.\n"));
        assert!(text.contains("\n\n## Entities\n"));
    }

    #[test]
    fn a_missing_or_empty_section_is_created() {
        let mut index = IndexDoc::parse("# Index\n\n## Concepts\n\n## Queries\n");
        index
            .set("concepts/a.md", "Concepts", "A", "First")
            .unwrap();
        index.set("queries/q.md", "Queries", "Q", "").unwrap();
        index
            .set("comparisons/c.md", "Comparisons", "C", "Side by side")
            .unwrap();
        assert_eq!(
            index.render(),
            "# Index\n\n## Concepts\n\n- [[concepts/a|A]] — First\n\n## Queries\n\n- [[queries/q|Q]]\n\n\
             ## Comparisons\n\n- [[comparisons/c|C]] — Side by side\n"
        );
        let mut bare = IndexDoc::parse("");
        bare.set("concepts/a.md", "Concepts", "A", "").unwrap();
        assert_eq!(bare.render(), "## Concepts\n\n- [[concepts/a|A]]\n");
    }

    #[test]
    fn lines_are_found_updated_and_removed_by_the_viewers_link_rules() {
        let mut index = IndexDoc::parse(
            "- [Beds](concepts/beds.md) — Raised\n[[concepts/soil]] [[concepts/clay]]\n",
        );
        assert_eq!(
            index.entry("concepts/beds.md").unwrap(),
            Some(Entry {
                title: Some("Beds".into()),
                summary: "Raised".into()
            })
        );
        assert_eq!(index.entry("concepts/other.md").unwrap(), None);
        index
            .set("concepts/beds.md", "Concepts", "Raised beds", "Raised")
            .unwrap();
        assert_eq!(
            index.render().lines().next().unwrap(),
            "- [[concepts/beds|Raised beds]] — Raised"
        );
        // A line publishing two pages is never rewritten.
        assert!(index.entry("concepts/soil.md").is_err());
        assert!(index.remove("concepts/soil.md").is_err());
        assert!(index.remove("concepts/beds.md").unwrap());
        assert!(!index.remove("concepts/beds.md").unwrap());
        assert_eq!(parse::published_pages(&index.render()).0.len(), 2);
    }
}
