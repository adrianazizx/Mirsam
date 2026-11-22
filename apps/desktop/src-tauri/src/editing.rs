//! Page edits, undo and redo (ADR 0014), markup (ADR 0017), and saving
//! (ADR 0015): thin commands over `safha_core::edit`, `safha_core::annotate`
//! and `safha_core::save`. An edit, undo, or redo leaves the document at a
//! new revision, which replaces the old one everywhere (renders, search
//! index). The outcome says whether pages changed, so the UI reloads the page
//! layout, or only paints the pages again.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use safha_core::annotate::{
    AnnotationEdit, AnnotationKind, Ink, InkPoint, Markup, MarkupKind, Note, Quad, Reply, TextBox,
};
use safha_core::edit::{Change, EditHistory, PageEdit};
use safha_core::view::ViewRect;

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
    /// The pages changed (count, order, sizes, content); otherwise only their
    /// annotations did, and the same pages only need painting again.
    pages_changed: bool,
}

impl EditOutcome {
    fn new(page_count: usize, history: &EditHistory, change: Change) -> Self {
        Self {
            page_count,
            can_undo: history.can_undo(),
            can_redo: history.can_redo(),
            modified: history.is_modified(),
            pages_changed: change == Change::Pages,
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
    Ok(EditOutcome::new(page_count, &history, Change::Pages))
}

/// Marked text on one page: the displayed-page rectangles of its runs.
#[derive(Debug, Clone, Deserialize)]
pub struct MarkedPage {
    page: usize,
    rects: Vec<ViewRect>,
}

/// The setting that names who makes new annotations (ADR 0041).
pub const COMMENT_AUTHOR: &str = "commentAuthor";

/// The name new annotations are signed with (`/T`): the user's own choice
/// (the `commentAuthor` setting), never guessed from the system. None when
/// unset.
fn author(state: &AppState) -> Option<String> {
    let name = state.store()?.setting(COMMENT_AUTHOR).ok().flatten()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Applies annotation edits to a document as one undo step.
fn edit_annotations(
    state: &AppState,
    document_id: u32,
    edits: impl FnOnce(&safha_core::Document) -> safha_core::Result<Vec<AnnotationEdit>>,
) -> CommandResult<EditOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let edits = edits(&document)?;
    let (revision, update) = document.with_annotation_edits_by(&edits, author(state).as_deref())?;
    history.push_annotations(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(EditOutcome::new(page_count, &history, Change::Annotations))
}

/// Marks text with highlight, underline, strikethrough, or squiggly: one
/// annotation per page, all in one undo step.
#[tauri::command]
pub async fn add_markup(
    state: State<'_, AppState>,
    document_id: u32,
    kind: MarkupKind,
    pages: Vec<MarkedPage>,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |document| {
        pages
            .into_iter()
            .map(|marked| {
                let view = document.view_transform(marked.page)?;
                let quads = marked.rects.iter().map(|rect| Quad::from_view(*rect, &view)).collect();
                Ok(AnnotationEdit::AddMarkup {
                    page: marked.page,
                    markup: Markup { kind, quads, color: kind.default_color() },
                })
            })
            .collect()
    })
}

/// A point of a pen stroke on the displayed page, with the pen's pressure.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct StrokePoint {
    x: f32,
    y: f32,
    pressure: f32,
}

/// Adds freehand ink: the strokes, in displayed-page points (ADR 0018).
#[tauri::command]
pub async fn add_ink(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    strokes: Vec<Vec<StrokePoint>>,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |document| {
        let view = document.view_transform(page)?;
        let strokes = strokes
            .iter()
            .map(|stroke| stroke.iter().map(|p| InkPoint::from_view(p.x, p.y, p.pressure, &view)).collect())
            .collect();
        Ok(vec![AnnotationEdit::AddInk {
            page,
            ink: Ink { strokes, color: Ink::DEFAULT_COLOR, width: Ink::DEFAULT_WIDTH },
        }])
    })
}

/// Adds a sticky note whose icon's top-left corner is displayed at (`x`, `y`).
#[tauri::command]
pub async fn add_note(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    x: f32,
    y: f32,
    contents: String,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |document| {
        let view = document.view_transform(page)?;
        Ok(vec![AnnotationEdit::AddNote {
            page,
            note: Note { rect: Note::rect_at(x, y, &view), contents, color: Note::DEFAULT_COLOR },
        }])
    })
}

/// Adds a text box (ADR 0019) whose top-left corner is displayed at (`x`,
/// `y`), `width` points wide; its height is fitted to the text.
#[tauri::command]
pub async fn add_text_box(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    x: f32,
    y: f32,
    width: f32,
    contents: String,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |document| {
        let view = document.view_transform(page)?;
        let height = TextBox::height_for(&contents, width, TextBox::DEFAULT_SIZE)?;
        Ok(vec![AnnotationEdit::AddTextBox {
            page,
            text_box: TextBox {
                rect: TextBox::rect_at(x, y, width, height, &view),
                contents,
                size: TextBox::DEFAULT_SIZE,
                color: TextBox::DEFAULT_COLOR,
            },
        }])
    })
}

/// Replaces a note's or a text box's text (a text box is set again).
#[tauri::command]
pub async fn set_annotation_contents(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    index: usize,
    contents: String,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |_| Ok(vec![AnnotationEdit::SetContents { page, index, contents }]))
}

/// Removes annotations of one page (with their pop-ups and replies), in one
/// undo step.
#[tauri::command]
pub async fn remove_annotations(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    mut indices: Vec<usize>,
) -> CommandResult<EditOutcome> {
    // Highest first: removing one moves the ones after it.
    indices.sort_unstable_by(|a, b| b.cmp(a));
    indices.dedup();
    edit_annotations(&state, document_id, |_| {
        Ok(indices.into_iter().map(|index| AnnotationEdit::Remove { page, index }).collect())
    })
}

/// Answers a comment (ADR 0041).
#[tauri::command]
pub async fn reply_to_comment(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    index: usize,
    contents: String,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |_| Ok(vec![AnnotationEdit::Reply { page, index, contents }]))
}

/// Marks a comment resolved, or open again (ADR 0041).
#[tauri::command]
pub async fn set_comment_resolved(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    index: usize,
    resolved: bool,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |_| Ok(vec![AnnotationEdit::SetResolved { page, index, resolved }]))
}

/// A comment as the comments panel lists it (ADR 0041).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentWire {
    page: usize,
    /// What `reply_to_comment`, `set_comment_resolved` and
    /// `remove_annotations` take.
    index: usize,
    kind: AnnotationKind,
    author: Option<String>,
    /// Unix seconds.
    modified: Option<i64>,
    contents: Option<String>,
    marked_text: Option<String>,
    /// Where it is on the displayed page.
    rect: ViewRect,
    resolved: bool,
    replies: Vec<Reply>,
}

/// Every comment in a document, page by page.
#[tauri::command]
pub async fn document_comments(
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Vec<CommentWire>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    tauri::async_runtime::spawn_blocking(move || comments_of(&document))
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))?
}

fn comments_of(document: &safha_core::Document) -> CommandResult<Vec<CommentWire>> {
    let mut views = std::collections::HashMap::new();
    let mut out = Vec::new();
    for comment in document.comments()? {
        let view = match views.entry(comment.page) {
            std::collections::hash_map::Entry::Occupied(known) => *known.get(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                *slot.insert(document.view_transform(comment.page)?)
            }
        };
        out.push(CommentWire {
            page: comment.page,
            index: comment.index,
            kind: comment.kind,
            author: comment.author,
            modified: comment.modified,
            contents: comment.contents,
            marked_text: comment.marked_text,
            rect: view.rect(comment.bounds),
            resolved: comment.resolved,
            replies: comment.replies,
        });
    }
    Ok(out)
}

/// An annotation as the UI hit-tests and shows it.
#[derive(Debug, Serialize)]
pub struct AnnotationWire {
    /// What `remove_annotations` and `set_annotation_contents` take.
    index: usize,
    kind: AnnotationKind,
    /// Displayed-page rectangles: one per run of marked text, else its box.
    rects: Vec<ViewRect>,
    /// A note's text.
    contents: Option<String>,
    /// Ink strokes, as displayed-page `[x, y]` points.
    strokes: Vec<Vec<[f32; 2]>>,
}

/// The page's annotations of the kinds this app writes, in displayed-page
/// coordinates.
#[tauri::command]
pub async fn page_annotations(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
) -> CommandResult<Vec<AnnotationWire>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    tauri::async_runtime::spawn_blocking(move || {
        let (annotations, view) = document.page_annotations(page)?;
        Ok(annotations
            .into_iter()
            .map(|info| AnnotationWire {
                index: info.index,
                kind: info.kind,
                rects: info.rects.into_iter().map(|rect| view.rect(rect)).collect(),
                contents: info.contents,
                strokes: info
                    .strokes
                    .iter()
                    .map(|stroke| {
                        stroke
                            .iter()
                            .map(|&(x, y)| {
                                let (x, y) = view.point(x, y);
                                [x, y]
                            })
                            .collect()
                    })
                    .collect(),
            })
            .collect())
    })
    .await
    .map_err(|err| CommandError::new("engine_error", err.to_string()))?
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
    let change = match step {
        Step::Undo => history.undo_change(),
        Step::Redo => history.redo_change(),
    };
    let bytes = match step {
        Step::Undo => history.undo(),
        Step::Redo => history.redo(),
    };
    let (Some(bytes), Some(change)) = (bytes, change) else {
        // Nothing to undo or redo: the document stays as it is.
        return Ok(EditOutcome::new(document.page_count(), &history, Change::Annotations));
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
    Ok(EditOutcome::new(page_count, &history, change))
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
    // A recovered document whose original changed since (ADR 0040): Save
    // would overwrite the newer file. The UI asks where instead.
    if state.save_as_first(document_id) {
        return Err(CommandError::new("save_as_required", "the original changed since"));
    }
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
    let outcome = save_locked(state, document_id, path)?;
    // Saved: its recovery file goes (ADR 0040).
    crate::recovery::write_now(state, document_id)?;
    Ok(outcome)
}

#[cfg(test)]
pub(crate) fn save_to_for_tests(
    state: &AppState,
    document_id: u32,
    path: PathBuf,
) -> CommandResult<SaveOutcome> {
    save_to(state, document_id, path)
}

fn save_locked(state: &AppState, document_id: u32, path: PathBuf) -> CommandResult<SaveOutcome> {
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
    Ok(Some(EditOutcome::new(page_count, &history, Change::Pages)))
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
    // The user chose to lose the changes: nothing to recover next time.
    for key in state.recovery_keys() {
        crate::recovery::forget(&state, &key);
    }
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use safha_core::store::Store;

    use super::*;
    use crate::state::MAIN_WINDOW;

    #[test]
    fn comments_are_signed_with_the_chosen_name_and_answered() {
        let dir = std::env::temp_dir().join(format!("safha-comments-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/out/chrome/en-paragraphs.pdf");
        assert!(fixture.is_file(), "fixture corpus missing: run `node fixtures/generate.mjs`");
        let state = AppState::new(Instant::now(), None);
        state.set_store(Store::open(&dir.join("state.sqlite3")).expect("store"));
        let outcome = crate::commands::open_path(&state, &fixture, None, MAIN_WINDOW).expect("opens");
        let id =
            serde_json::to_value(&outcome).expect("json")["document"]["id"].as_u64().expect("opened") as u32;
        let note = |state: &AppState, text: &str| {
            edit_annotations(state, id, |document| {
                let view = document.view_transform(0)?;
                Ok(vec![AnnotationEdit::AddNote {
                    page: 0,
                    note: Note {
                        rect: Note::rect_at(10.0, 10.0, &view),
                        contents: text.into(),
                        color: Note::DEFAULT_COLOR,
                    },
                }])
            })
            .expect("adds a note")
        };

        // No name chosen: none is written (the system's user name is not guessed).
        note(&state, "first");
        let comments = comments_of(&state.document(id).expect("document")).expect("comments");
        assert_eq!(comments[0].author, None);

        state.store().expect("store").set_setting(COMMENT_AUTHOR, "  مراجِع  ").expect("set");
        note(&state, "second");
        let comments = comments_of(&state.document(id).expect("document")).expect("comments");
        assert_eq!(comments[1].author.as_deref(), Some("مراجِع"));
        // Displayed-page coordinates, where the note was put.
        assert!((comments[1].rect.x - 10.0).abs() < 0.5 && (comments[1].rect.y - 10.0).abs() < 0.5);

        let index = comments[0].index;
        edit_annotations(&state, id, |_| {
            Ok(vec![AnnotationEdit::Reply { page: 0, index, contents: "تم".into() }])
        })
        .expect("replies");
        let outcome = edit_annotations(&state, id, |_| {
            Ok(vec![AnnotationEdit::SetResolved { page: 0, index, resolved: true }])
        })
        .expect("resolves");
        assert!(!outcome.pages_changed && outcome.can_undo);
        let comments = comments_of(&state.document(id).expect("document")).expect("comments");
        assert_eq!(comments.len(), 2);
        assert!(comments[0].resolved);
        assert_eq!(comments[0].replies[0].author.as_deref(), Some("مراجِع"));
        let _ = std::fs::remove_dir_all(dir);
    }

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
