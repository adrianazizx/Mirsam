//! Page edits, undo and redo (ADR 0014): thin commands over
//! `safha_core::edit`. Each one leaves the document at a new revision, which
//! replaces the old one everywhere (renders, search index); the UI reloads
//! the page layout from the outcome.

use serde::{Deserialize, Serialize};
use tauri::State;

use safha_core::edit::{EditHistory, PageEdit};

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, lock};

/// A page edit as the UI sends it. Page numbers are zero-based.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PageEditArg {
    Rotate { pages: Vec<usize>, quarter_turns: i32 },
    Delete { pages: Vec<usize> },
    Move { pages: Vec<usize>, to: usize },
    InsertBlank { at: usize, width: f32, height: f32 },
    Duplicate { pages: Vec<usize> },
}

impl From<PageEditArg> for PageEdit {
    fn from(arg: PageEditArg) -> Self {
        match arg {
            PageEditArg::Rotate { pages, quarter_turns } => PageEdit::Rotate { pages, quarter_turns },
            PageEditArg::Delete { pages } => PageEdit::Delete { pages },
            PageEditArg::Move { pages, to } => PageEdit::Move { pages, to },
            PageEditArg::InsertBlank { at, width, height } => PageEdit::InsertBlank { at, width, height },
            PageEditArg::Duplicate { pages } => PageEdit::Duplicate { pages },
        }
    }
}

/// The document after an edit, an undo, or a redo.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOutcome {
    page_count: usize,
    can_undo: bool,
    can_redo: bool,
    /// Differs from the file as last opened or saved.
    modified: bool,
}

impl EditOutcome {
    fn new(page_count: usize, history: &EditHistory) -> Self {
        Self {
            page_count,
            can_undo: history.can_undo(),
            can_redo: history.can_redo(),
            modified: history.is_modified(),
        }
    }
}

#[tauri::command]
pub async fn edit_pages(
    state: State<'_, AppState>,
    document_id: u32,
    edit: PageEditArg,
) -> CommandResult<EditOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (revision, update) = document.with_page_edit(&edit.into())?;
    history.push(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(EditOutcome::new(page_count, &history))
}

#[tauri::command]
pub async fn undo_edit(state: State<'_, AppState>, document_id: u32) -> CommandResult<EditOutcome> {
    step(&state, document_id, Step::Undo)
}

#[tauri::command]
pub async fn redo_edit(state: State<'_, AppState>, document_id: u32) -> CommandResult<EditOutcome> {
    step(&state, document_id, Step::Redo)
}

#[derive(Clone, Copy)]
enum Step {
    Undo,
    Redo,
}

fn step(state: &AppState, document_id: u32, step: Step) -> CommandResult<EditOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let bytes = match step {
        Step::Undo => history.undo(),
        Step::Redo => history.redo(),
    };
    let Some(bytes) = bytes else {
        // Nothing to undo or redo: the document stays as it is.
        return Ok(EditOutcome::new(document.page_count(), &history));
    };
    let revision = match document.revision(bytes) {
        Ok(revision) => revision,
        Err(err) => {
            // Put the history back where the document still is.
            match step {
                Step::Undo => history.redo(),
                Step::Redo => history.undo(),
            };
            return Err(err.into());
        }
    };
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(EditOutcome::new(page_count, &history))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_arrive_in_the_shape_the_ui_sends() {
        let rotate: PageEditArg =
            serde_json::from_str(r#"{"kind":"rotate","pages":[0,2],"quarterTurns":-1}"#).expect("parses");
        assert_eq!(PageEdit::from(rotate), PageEdit::Rotate { pages: vec![0, 2], quarter_turns: -1 });
        let blank: PageEditArg =
            serde_json::from_str(r#"{"kind":"insertBlank","at":1,"width":595,"height":842}"#)
                .expect("parses");
        assert_eq!(PageEdit::from(blank), PageEdit::InsertBlank { at: 1, width: 595.0, height: 842.0 });
    }
}
