//! The editor's tools (feature `editor`): the only way an agent changes the
//! wiki. A batch is checked as a whole and applied all or nothing.

use super::{Leaf, json};
use leaf_editor::{Batch, BatchOutcome, Editor};
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;
use std::sync::MutexGuard;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BatchParams {
    #[doc = include_str!("../batch-format.md")]
    pub batch: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PageParams {
    /// A published page, e.g. `concepts/garden`.
    pub page: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SuggestParams {
    /// A metadata key, e.g. `related-people` or `relationship.alice`.
    pub key: String,
    /// A value of that key to check; omit to check the key itself.
    pub value: Option<String>,
}

impl Leaf {
    fn editor(&self) -> Result<MutexGuard<'_, Editor>, String> {
        self.editor
            .lock()
            .map_err(|_| "the editor is unavailable".to_string())
    }
}

/// The outcome as JSON; a rejected batch is a tool error, so it is flagged.
fn outcome(outcome: &BatchOutcome) -> Result<String, String> {
    let text = json(outcome)?;
    if outcome.is_acceptable() {
        Ok(text)
    } else {
        Err(text)
    }
}

#[tool_router(router = editor_router, vis = "pub(super)")]
impl Leaf {
    /// Check a batch of edits without writing anything: says whether it
    /// would be applied and, if not, every error with the operation it
    /// belongs to and suggestions.
    #[tool(name = "editor_check")]
    async fn editor_check(
        &self,
        Parameters(params): Parameters<BatchParams>,
    ) -> Result<String, String> {
        let batch = Batch::parse(&params.batch)?;
        let result = self
            .editor()?
            .check(&batch)
            .map_err(|err| self.private(err))?;
        outcome(&result)
    }

    /// Apply a batch of edits: all of it or none of it. Checked as a whole
    /// first; a rejected batch changes nothing. Returns the files changed
    /// and the new ids of changed passages.
    #[tool(name = "editor_apply")]
    async fn editor_apply(
        &self,
        Parameters(params): Parameters<BatchParams>,
    ) -> Result<String, String> {
        let batch = Batch::parse(&params.batch)?;
        let result = self
            .editor()?
            .apply(&batch)
            .map_err(|err| self.private(err))?;
        outcome(&result)
    }

    /// Read a page as the editor sees it: metadata, headings, and passages
    /// with the ids that edit operations take.
    #[tool(name = "editor_page")]
    async fn editor_page(
        &self,
        Parameters(params): Parameters<PageParams>,
    ) -> Result<String, String> {
        let page = self
            .editor()?
            .page(&params.page)
            .map_err(|err| self.private(err))?;
        json(&page)
    }

    /// List every metadata key with its values, page counts and
    /// descriptions: what edits may use.
    #[tool(name = "editor_metadata")]
    async fn editor_metadata(&self) -> Result<String, String> {
        json(
            &self
                .editor()?
                .list_metadata()
                .map_err(|err| self.private(err))?,
        )
    }

    /// Check a metadata key or value: known, rewritten to a canonical
    /// spelling, or unknown with suggestions.
    #[tool(name = "editor_suggest")]
    async fn editor_suggest(
        &self,
        Parameters(params): Parameters<SuggestParams>,
    ) -> Result<String, String> {
        let editor = self.editor()?;
        let lookup = match &params.value {
            Some(value) => editor.check_value(&params.key, value),
            None => editor.check_key(&params.key),
        }
        .map_err(|err| self.private(err))?;
        json(&lookup)
    }

    /// Check the wiki against the vocabulary: problems to fix and the
    /// declarations it would need.
    #[tool(name = "editor_conformance")]
    async fn editor_conformance(&self) -> Result<String, String> {
        json(
            &self
                .editor()?
                .conformance()
                .map_err(|err| self.private(err))?,
        )
    }
}
