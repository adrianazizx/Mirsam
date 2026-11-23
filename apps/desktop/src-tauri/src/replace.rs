//! Find and replace (ADR 0052): thin commands over `mirsam_core::replace`.
//! The preview reads the document's shared search index and each page's
//! paragraphs; replacing is one edit, so a single undo takes it back.

use mirsam_core::edit::Change;
use mirsam_core::replace::{self, Plan, Report, SkipReason, Target};
use mirsam_core::search::{Query, SearchError, SearchOptions};
use mirsam_core::view::ViewRect;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::editing::EditOutcome;
use crate::error::{CommandError, CommandResult};
use crate::navigation::blocking;
use crate::state::{AppState, lock};

/// At most this many matches are listed.
const MAX_OCCURRENCES: usize = 5_000;

fn compile(query: &str, options: SearchOptions) -> CommandResult<Query> {
    Query::new(query, options).map_err(|err| match err {
        SearchError::Empty => CommandError::new("invalid_query", "empty"),
        SearchError::InvalidRegex(detail) => CommandError::new("invalid_query", detail),
    })
}

/// Every match of a search with what the replacement makes of it, and why
/// the ones that cannot be replaced are not. `invalid_query` for an empty
/// query or a bad regular expression.
#[tauri::command]
pub async fn replace_preview(
    state: State<'_, AppState>,
    document_id: u32,
    query: String,
    options: SearchOptions,
    replacement: String,
) -> CommandResult<Plan> {
    let query = compile(&query, options)?;
    let (document, index) =
        state.index(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    blocking(move || {
        let index = index
            .get_or_build(&document)?
            .ok_or_else(|| CommandError::new("cancelled", "document closed"))?;
        Ok(replace::plan(&document, &index, &query, &replacement, None, MAX_OCCURRENCES)?)
    })
    .await
}

/// Which matches to replace.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ReplaceScope {
    /// The ones a preview listed (all of them, or those left ticked).
    Targets { targets: Vec<Target> },
    /// The search hit shown: its page, and its boxes on the displayed page.
    Hit { page: usize, rects: Vec<ViewRect> },
}

/// What replacing did. `edit` is absent when nothing changed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceOutcome {
    edit: Option<EditOutcome>,
    report: Report,
    /// Why the search hit could not be replaced (a `hit` scope only).
    hit_skipped: Option<SkipReason>,
}

/// Replaces matches of a search through the in-place editor: one undo step.
#[tauri::command]
pub async fn replace_text(
    state: State<'_, AppState>,
    document_id: u32,
    query: String,
    options: SearchOptions,
    replacement: String,
    scope: ReplaceScope,
) -> CommandResult<ReplaceOutcome> {
    let query = compile(&query, options)?;
    // Like every edit: the document's history is held while it changes, so
    // edits apply in order.
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let (document, index) =
        state.index(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut hit_skipped = None;
    let from_hit = matches!(scope, ReplaceScope::Hit { .. });
    let targets = match scope {
        ReplaceScope::Targets { targets } => targets,
        ReplaceScope::Hit { page, rects } => {
            let index = index
                .get_or_build(&document)?
                .ok_or_else(|| CommandError::new("cancelled", "document closed"))?;
            let plan = replace::plan(&document, &index, &query, &replacement, Some(page), MAX_OCCURRENCES)?;
            match replace::occurrence_at(&plan.occurrences, &rects) {
                Some(occurrence) => {
                    hit_skipped = occurrence.skip;
                    occurrence.target.into_iter().collect()
                }
                None => {
                    hit_skipped = Some(SkipReason::Changed);
                    Vec::new()
                }
            }
        }
    };
    let (done, report) = replace::apply(&document, &query, &replacement, &targets)?;
    // The hit's match, skipped when it was set (it read back otherwise, or moved).
    if from_hit && hit_skipped.is_none() {
        hit_skipped = report.skipped.first().map(|skipped| skipped.reason);
    }
    let edit = match done {
        Some((revision, update)) => {
            history.push_content(update);
            let page_count = revision.page_count();
            state.replace_revision(document_id, revision);
            Some(EditOutcome::new(page_count, &history, Change::Content))
        }
        None => None,
    };
    Ok(ReplaceOutcome { edit, report, hit_skipped })
}
