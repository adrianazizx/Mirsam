//! True redaction and sanitizing (ADR 0042): thin commands over
//! `mirsam_core::redact`. Finding runs on the document's shared search index;
//! applying is one edit (an update, or a rewritten file for a signed
//! document), so undo takes it back like any other.

use mirsam_core::redact::{self, Mark, Pattern, Preset, SanitizeOptions};
use mirsam_core::search::{SearchError, SearchOptions};
use mirsam_core::text::Rect;
use mirsam_core::view::{ViewRect, ViewTransform};
use mirsam_core::{RevisionStep, edit::Change};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::editing::EditOutcome;
use crate::error::{CommandError, CommandResult};
use crate::navigation::blocking;
use crate::state::{AppState, lock};

/// At most this many matches are sent.
const MAX_HITS: usize = 5_000;

/// What to look for.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PatternArg {
    /// Text, or a regular expression when `options.regex` is set.
    Text {
        query: String,
        options: SearchOptions,
    },
    Preset {
        preset: Preset,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactHitWire {
    page: usize,
    text: String,
    before: String,
    after: String,
    rects: Vec<ViewRect>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactHits {
    hits: Vec<RedactHitWire>,
    truncated: bool,
}

/// Finds what matches a pattern, with boxes to mark (displayed-page points).
/// `invalid_query` for an empty query or a bad regular expression.
#[tauri::command]
pub async fn find_redactions(
    state: State<'_, AppState>,
    document_id: u32,
    pattern: PatternArg,
) -> CommandResult<RedactHits> {
    let pattern = match pattern {
        PatternArg::Text { query, options } => Pattern::Text { text: query, options },
        PatternArg::Preset { preset } => Pattern::Preset(preset),
    };
    let (document, index) =
        state.index(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    blocking(move || {
        let index = index
            .get_or_build(&document)?
            .ok_or_else(|| CommandError::new("cancelled", "document closed"))?;
        let (found, truncated) =
            redact::find_matches(&index, &pattern, MAX_HITS).map_err(|err| match err {
                SearchError::Empty => CommandError::new("invalid_query", "empty"),
                SearchError::InvalidRegex(detail) => CommandError::new("invalid_query", detail),
            })?;
        let hits = found
            .into_iter()
            .filter_map(|hit| {
                let view = index.view(hit.page)?;
                Some(RedactHitWire {
                    page: hit.page,
                    text: hit.text,
                    before: hit.before,
                    after: hit.after,
                    rects: hit.rects.iter().map(|rect| view.rect(*rect)).collect(),
                })
            })
            .collect();
        Ok(RedactHits { hits, truncated })
    })
    .await
}

/// A marked area as the UI sends it: boxes on the displayed page, and the
/// text under them when they came from a search or a selection.
#[derive(Debug, Clone, Deserialize)]
pub struct MarkArg {
    page: usize,
    rects: Vec<ViewRect>,
    text: Option<String>,
}

/// The rectangle of page space under a rectangle of the displayed page.
fn page_rect(view: &ViewTransform, rect: ViewRect) -> Rect {
    let corners = [
        view.page_point(rect.x, rect.y),
        view.page_point(rect.x + rect.width, rect.y),
        view.page_point(rect.x, rect.y + rect.height),
        view.page_point(rect.x + rect.width, rect.y + rect.height),
    ];
    let xs = corners.map(|c| c.0);
    let ys = corners.map(|c| c.1);
    Rect {
        left: xs.iter().copied().fold(f32::MAX, f32::min),
        bottom: ys.iter().copied().fold(f32::MAX, f32::min),
        right: xs.iter().copied().fold(f32::MIN, f32::max),
        top: ys.iter().copied().fold(f32::MIN, f32::max),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactOutcome {
    edit: EditOutcome,
    report: redact::Report,
}

/// Applies the marks: removes what is under them and draws the boxes. One
/// undo step. Refused, with nothing changed, if checking the result finds
/// text still readable under a mark (`redaction_failed`).
#[tauri::command]
pub async fn apply_redactions(
    state: State<'_, AppState>,
    document_id: u32,
    marks: Vec<MarkArg>,
) -> CommandResult<RedactOutcome> {
    // Like every edit: the document's history is held while it changes, so
    // edits apply in order.
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut core_marks = Vec::new();
    for mark in &marks {
        let view = document.view_transform(mark.page)?;
        for rect in &mark.rects {
            core_marks.push(Mark { page: mark.page, rect: page_rect(&view, *rect), text: mark.text.clone() });
        }
    }
    let redacted = document.with_redactions(&core_marks)?;
    match redacted.step {
        RevisionStep::Update(update) => history.push(update),
        RevisionStep::Snapshot(bytes) => history.push_snapshot(bytes),
    }
    let page_count = redacted.document.page_count();
    state.replace_revision(document_id, redacted.document);
    Ok(RedactOutcome { edit: EditOutcome::new(page_count, &history, Change::Pages), report: redacted.report })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizeOutcome {
    edit: EditOutcome,
    report: redact::SanitizeReport,
}

/// Sanitizes the document: one undo step, a rewritten file.
#[tauri::command]
pub async fn sanitize_document(
    state: State<'_, AppState>,
    document_id: u32,
    options: SanitizeOptions,
) -> CommandResult<SanitizeOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (revision, bytes, report) = document.with_sanitized(options)?;
    history.push_snapshot(bytes);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(SanitizeOutcome { edit: EditOutcome::new(page_count, &history, Change::Pages), report })
}
