//! Page edits, undo and redo (ADR 0014), and saving (ADR 0015): thin
//! commands over `safha_core::edit` and `safha_core::save`. An edit, undo, or
//! redo leaves the document at a new revision, which replaces the old one
//! everywhere (renders, search index); the UI reloads the page layout from
//! the outcome.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

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

/// The document after a save.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOutcome {
    /// The file's display name (new after Save As).
    name: String,
    modified: bool,
    /// Whether the edits were appended to keep signatures valid, rather than
    /// the file rewritten in full.
    incremental: bool,
}

/// Saves the document over its own file (ADR 0015).
#[tauri::command]
pub async fn save_document(state: State<'_, AppState>, document_id: u32) -> CommandResult<SaveOutcome> {
    let path = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    save_to(&state, document_id, path)
}

/// Asks where to save a copy, saves it there, and from then on the document
/// is that file. `None` when the user cancels.
#[tauri::command]
pub async fn save_document_as(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Option<SaveOutcome>> {
    let current = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut dialog = app.dialog().file().add_filter("PDF", &["pdf"]);
    if let Some(name) = current.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    if let Some(dir) = current.parent() {
        dialog = dialog.set_directory(dir);
    }
    let Some(mut path) = dialog.blocking_save_file().and_then(|file| file.into_path().ok()) else {
        return Ok(None);
    };
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf")) {
        path.as_mut_os_string().push(".pdf");
    }
    save_to(&state, document_id, path).map(Some)
}

fn save_to(state: &AppState, document_id: u32, path: PathBuf) -> CommandResult<SaveOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mode = safha_core::save::save(&document, &path)?;
    history.mark_saved();
    let name = state.set_path(document_id, path.clone()).unwrap_or_default();
    if let Some(store) = state.store()
        && let Err(err) = store.record_open(&path)
    {
        eprintln!("safha: could not record a recent file: {err}");
    }
    Ok(SaveOutcome {
        name,
        modified: history.is_modified(),
        incremental: mode == safha_core::save::SaveMode::Incremental,
    })
}

/// Asks for one or more PDFs and inserts all their pages before page `at`
/// (`at` = page count appends), in the order picked: insert, or merge
/// (ADR 0016). `None` when the user cancels.
#[tauri::command]
pub async fn insert_pages_from_files(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    at: usize,
) -> CommandResult<Option<EditOutcome>> {
    let picked = app.dialog().file().add_filter("PDF", &["pdf"]).blocking_pick_files();
    let Some(paths) = picked else { return Ok(None) };
    let paths: Vec<PathBuf> = paths.into_iter().filter_map(|file| file.into_path().ok()).collect();
    if paths.is_empty() {
        return Ok(None);
    }
    let engine = safha_core::Engine::global(&safha_core::PdfiumSource::Auto)?;
    // A file that needs a password cannot be inserted yet; the error says so.
    let others = paths.iter().map(|path| engine.open_file(path, None)).collect::<Result<Vec<_>, _>>()?;
    let others: Vec<&safha_core::Document> = others.iter().collect();

    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (revision, bytes) = document.with_pages_from(&others, at)?;
    history.push_snapshot(bytes);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(Some(EditOutcome::new(page_count, &history)))
}

/// Writes `pages` as a new PDF where the user chooses. Returns the new
/// file's name, or `None` when the user cancels.
#[tauri::command]
pub async fn extract_pages(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    pages: Vec<usize>,
) -> CommandResult<Option<String>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let bytes = document.extract_pages(&pages)?;
    let current = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let stem = current.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut dialog =
        app.dialog().file().add_filter("PDF", &["pdf"]).set_file_name(format!("{stem} (extract).pdf"));
    if let Some(dir) = current.parent() {
        dialog = dialog.set_directory(dir);
    }
    let Some(mut path) = dialog.blocking_save_file().and_then(|file| file.into_path().ok()) else {
        return Ok(None);
    };
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf")) {
        path.as_mut_os_string().push(".pdf");
    }
    write_new_pdf(&path, &bytes, pages.len())?;
    Ok(Some(crate::state::display_name(&path)))
}

/// Splits the document into files of at most `size` pages, written into a
/// folder the user chooses as `name (1).pdf`, `name (2).pdf`, … next to
/// nothing they would overwrite. Returns how many files were written, or
/// `None` when the user cancels.
#[tauri::command]
pub async fn split_document(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    size: usize,
) -> CommandResult<Option<usize>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let parts = document.split_every(size)?;
    let current = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let stem = current.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut dialog = app.dialog().file();
    if let Some(dir) = current.parent() {
        dialog = dialog.set_directory(dir);
    }
    let Some(folder) = dialog.blocking_pick_folder().and_then(|folder| folder.into_path().ok()) else {
        return Ok(None);
    };
    let total = document.page_count();
    for (index, part) in parts.iter().enumerate() {
        let path = free_path(&folder, &format!("{stem} ({})", index + 1));
        let expected = size.min(total - index * size);
        write_new_pdf(&path, part, expected)?;
    }
    Ok(Some(parts.len()))
}

/// Validates a new PDF (it opens, with `pages` pages) and writes it atomically.
fn write_new_pdf(path: &std::path::Path, bytes: &[u8], pages: usize) -> CommandResult<()> {
    let engine = safha_core::Engine::global(&safha_core::PdfiumSource::Auto)?;
    let reopened = engine
        .open_bytes(bytes.to_vec(), None)
        .map_err(|_| CommandError::new("save_failed", "the new file would not open"))?;
    if reopened.page_count() != pages {
        return Err(CommandError::new("save_failed", "the new file would have the wrong number of pages"));
    }
    safha_core::save::write_atomically(path, bytes)?;
    Ok(())
}

/// `folder/name.pdf`, or `folder/name 2.pdf`, … whichever does not exist yet.
fn free_path(folder: &std::path::Path, name: &str) -> PathBuf {
    let first = folder.join(format!("{name}.pdf"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| folder.join(format!("{name} {n}.pdf"))).find(|path| !path.exists()).unwrap_or(first)
}

/// Asks a window's UI to confirm closing it: it holds unsaved changes.
pub const CLOSE_REQUESTED: &str = "close-requested";
/// Asks a window's UI to confirm quitting: documents hold unsaved changes.
pub const QUIT_REQUESTED: &str = "quit-requested";

/// Closes the calling window although it holds unsaved changes (the user
/// confirmed).
#[tauri::command]
pub fn close_window_discarding(window: WebviewWindow, state: State<'_, AppState>) -> CommandResult<()> {
    state.confirm_close(window.label());
    window.close().map_err(|err| CommandError::new("window_error", err.to_string()))
}

/// Quits although documents hold unsaved changes (the user confirmed).
#[tauri::command]
pub fn quit_discarding(app: AppHandle, state: State<'_, AppState>) {
    state.confirm_quit();
    app.exit(0);
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
