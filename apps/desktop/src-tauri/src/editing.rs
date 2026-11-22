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
    pub(crate) fn new(page_count: usize, history: &EditHistory, change: Change) -> Self {
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

/// What inserting pages from files led to (ADR 0016).
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum InsertOutcome {
    Inserted {
        edit: EditOutcome,
    },
    /// A picked file needs its password; `unlock_insert` takes it and goes on.
    NeedsPassword {
        token: u32,
        name: String,
        /// The last password given was wrong.
        retry: bool,
    },
}

/// An insert waiting for a password: the files picked, and those opened so far.
pub struct PendingInsert {
    document_id: u32,
    at: usize,
    paths: Vec<PathBuf>,
    opened: Vec<safha_core::Document>,
}

/// Asks for one or more PDFs and inserts all their pages before page `at`
/// (`at` = page count appends), in the order picked: insert, or merge
/// (ADR 0016). A file that needs a password is asked for it, one at a time.
/// `None` when the user cancels the picker.
#[tauri::command]
pub async fn insert_pages_from_files(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    at: usize,
) -> CommandResult<Option<InsertOutcome>> {
    let picked = app.dialog().file().add_filter("PDF", &["pdf"]).blocking_pick_files();
    let Some(paths) = picked else { return Ok(None) };
    let paths: Vec<PathBuf> = paths.into_iter().filter_map(|file| file.into_path().ok()).collect();
    if paths.is_empty() {
        return Ok(None);
    }
    continue_insert(&state, PendingInsert { document_id, at, paths, opened: Vec::new() }, None).map(Some)
}

/// Tries the password of the file an insert waits for, and goes on.
#[tauri::command]
pub async fn unlock_insert(
    state: State<'_, AppState>,
    token: u32,
    password: String,
) -> CommandResult<InsertOutcome> {
    let pending =
        state.take_insert(token).ok_or_else(|| CommandError::new("unknown_token", "no insert waiting"))?;
    continue_insert(&state, pending, Some(&password))
}

/// Opens the insert's remaining files (the next with `password`) and, once
/// all are open, inserts their pages as one undo step. Stops at a file that
/// needs a password, keeping what is open so far.
fn continue_insert(
    state: &AppState,
    mut pending: PendingInsert,
    password: Option<&str>,
) -> CommandResult<InsertOutcome> {
    let engine = safha_core::Engine::global(&safha_core::PdfiumSource::Auto)?;
    let mut password = password;
    while let Some(path) = pending.paths.get(pending.opened.len()) {
        match engine.open_file(path, password) {
            Ok(document) => {
                pending.opened.push(document);
                password = None;
            }
            Err(safha_core::Error::PasswordRequired) => {
                let name = crate::state::display_name(path);
                let retry = password.is_some();
                return Ok(InsertOutcome::NeedsPassword { token: state.hold_insert(pending), name, retry });
            }
            Err(err) => return Err(err.into()),
        }
    }
    let document_id = pending.document_id;
    let others: Vec<&safha_core::Document> = pending.opened.iter().collect();
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (revision, bytes) = document.with_pages_from(&others, pending.at)?;
    history.push_snapshot(bytes);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(InsertOutcome::Inserted { edit: EditOutcome::new(page_count, &history, Change::Pages) })
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

/// How to split a document (ADR 0016), as the UI sends it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SplitBy {
    /// Files of `size` pages; the last gets the rest.
    Pages { size: usize },
    /// One file per top-level bookmark.
    Bookmarks,
    /// Files of at most `limit` bytes.
    Bytes { limit: u64 },
}

/// Where a split at the top-level bookmarks would fall: what the split
/// dialog shows before the user chooses it.
#[derive(Debug, Serialize)]
pub struct BookmarkPart {
    title: String,
    /// Zero-based, end exclusive.
    start: usize,
    end: usize,
}

#[tauri::command]
pub async fn bookmark_parts(
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Vec<BookmarkPart>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    Ok(document
        .bookmark_parts()
        .into_iter()
        .map(|(title, range)| BookmarkPart { title, start: range.start, end: range.end })
        .collect())
}

/// Splits the document (by page count, top-level bookmarks, or file size)
/// into files written into a folder the user chooses: `name (1).pdf`,
/// `name (2).pdf`, … (with the bookmark's title after the number when split
/// at bookmarks), never over an existing file. Returns how many files were
/// written, or `None` when the user cancels.
#[tauri::command]
pub async fn split_document(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    by: SplitBy,
) -> CommandResult<Option<usize>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    match by {
        SplitBy::Pages { size: 0 } | SplitBy::Bytes { limit: 0 } => {
            return Err(CommandError::new("invalid_edit", "parts need at least one page and one byte"));
        }
        SplitBy::Bookmarks if document.bookmark_parts().len() < 2 => {
            return Err(CommandError::new("no_bookmarks", "no top-level bookmarks on different pages"));
        }
        _ => {}
    }
    let current = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let stem = current.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut dialog = app.dialog().file();
    if let Some(dir) = current.parent() {
        dialog = dialog.set_directory(dir);
    }
    let Some(folder) = dialog.blocking_pick_folder().and_then(|folder| folder.into_path().ok()) else {
        return Ok(None);
    };
    // Splitting by size writes each candidate part to measure it: off the UI's way.
    let parts = tauri::async_runtime::spawn_blocking(move || split_parts(&document, &stem, &by))
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))??;
    for (name, bytes, pages) in &parts {
        write_new_pdf(&free_path(&folder, name), bytes, *pages)?;
    }
    Ok(Some(parts.len()))
}

/// The files of a split: each one's name (without `.pdf`), bytes, and page count.
fn split_parts(
    document: &safha_core::Document,
    stem: &str,
    by: &SplitBy,
) -> CommandResult<Vec<(String, Vec<u8>, usize)>> {
    let numbered = |index: usize| format!("{stem} ({})", index + 1);
    Ok(match *by {
        SplitBy::Pages { size } => {
            let total = document.page_count();
            document
                .split_every(size)?
                .into_iter()
                .enumerate()
                .map(|(index, bytes)| (numbered(index), bytes, size.min(total - index * size)))
                .collect()
        }
        SplitBy::Bookmarks => {
            let parts = document.bookmark_parts();
            let ranges: Vec<_> = parts.iter().map(|(_, range)| range.clone()).collect();
            document
                .split_ranges(&ranges)?
                .into_iter()
                .zip(parts)
                .enumerate()
                .map(|(index, (bytes, (title, range)))| {
                    let title = file_name_part(&title);
                    let name = if title.is_empty() {
                        numbered(index)
                    } else {
                        format!("{} {title}", numbered(index))
                    };
                    (name, bytes, range.len())
                })
                .collect()
        }
        SplitBy::Bytes { limit } => document
            .split_by_size(usize::try_from(limit).unwrap_or(usize::MAX))?
            .into_iter()
            .enumerate()
            .map(|(index, (range, bytes))| (numbered(index), bytes, range.len()))
            .collect(),
    })
}

/// A bookmark title made fit for a file name: no path separators or other
/// characters file systems refuse, no control characters, no bidi overrides
/// (which can make a name display as something it is not), at most 60
/// characters.
fn file_name_part(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            let refused = c.is_control()
                || r#"/\:*?"<>|"#.contains(c)
                || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
            if refused { ' ' } else { c }
        })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = joined.chars().take(60).collect();
    short.trim_end_matches(['.', ' ']).to_string()
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
    fn inserting_asks_for_each_protected_files_password() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/out");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixtures.join("manifest.json"))
                .expect("fixture corpus: run `node fixtures/generate.mjs`"),
        )
        .expect("manifest");
        let encrypted = manifest["fixtures"]
            .as_array()
            .and_then(|all| all.iter().find(|f| f["producer"] == "qpdf" && f["id"] == "encrypted"))
            .expect("the encrypted fixture");
        let password = encrypted["password"].as_str().expect("its password").to_string();
        let protected = fixtures.join(encrypted["path"].as_str().expect("path"));
        let plain = fixtures.join("chrome/en-paragraphs.pdf");

        let state = AppState::new(Instant::now(), None);
        let outcome = crate::commands::open_path(&state, &plain, None, MAIN_WINDOW).expect("opens");
        let id =
            serde_json::to_value(&outcome).expect("json")["document"]["id"].as_u64().expect("opened") as u32;
        let pages = |state: &AppState| state.document(id).expect("document").page_count();
        let before = pages(&state);
        let protected_pages = safha_core::Engine::global(&safha_core::PdfiumSource::Auto)
            .expect("engine")
            .open_file(&protected, Some(&password))
            .expect("opens")
            .page_count();

        let pending = PendingInsert {
            document_id: id,
            at: before,
            paths: vec![protected.clone(), plain.clone()],
            opened: Vec::new(),
        };
        let token = |outcome: InsertOutcome, retry: bool| match outcome {
            InsertOutcome::NeedsPassword { token, name, retry: asked } => {
                assert_eq!((name.as_str(), asked), ("encrypted.pdf", retry));
                token
            }
            InsertOutcome::Inserted { .. } => panic!("inserted without the password"),
        };
        let first = token(continue_insert(&state, pending, None).expect("asks"), false);
        let pending = state.take_insert(first).expect("held");
        let second = token(continue_insert(&state, pending, Some("wrong")).expect("asks again"), true);
        assert_eq!(pages(&state), before, "nothing inserted while waiting");
        let pending = state.take_insert(second).expect("held");
        let InsertOutcome::Inserted { edit } =
            continue_insert(&state, pending, Some(&password)).expect("inserts")
        else {
            panic!("the right password inserts");
        };
        assert!(edit.pages_changed && edit.can_undo);
        assert_eq!(pages(&state), before + protected_pages + before);

        // Giving up forgets the held insert.
        let pending = PendingInsert { document_id: id, at: 0, paths: vec![protected], opened: Vec::new() };
        let held = token(continue_insert(&state, pending, None).expect("asks"), false);
        state.forget_pending(held);
        assert!(state.take_insert(held).is_none());
    }

    #[test]
    fn splits_arrive_in_the_shape_the_ui_sends() {
        let parse = |json: &str| serde_json::from_str::<SplitBy>(json).expect("parses");
        assert_eq!(parse(r#"{"kind":"pages","size":3}"#), SplitBy::Pages { size: 3 });
        assert_eq!(parse(r#"{"kind":"bookmarks"}"#), SplitBy::Bookmarks);
        assert_eq!(parse(r#"{"kind":"bytes","limit":5000000}"#), SplitBy::Bytes { limit: 5_000_000 });
    }

    #[test]
    fn bookmark_titles_become_safe_file_names() {
        assert_eq!(file_name_part("الفصل الأول: البداية"), "الفصل الأول البداية");
        // No separators survive: the title cannot leave the chosen folder
        // (and always follows the document's name and number).
        assert_eq!(file_name_part("../../etc/passwd"), ".. .. etc passwd");
        assert_eq!(file_name_part("a\u{202E}fdp.exe"), "a fdp.exe");
        assert_eq!(file_name_part("Chapter 1..."), "Chapter 1");
        assert_eq!(file_name_part("\t\n"), "");
        assert_eq!(file_name_part(&"x".repeat(100)).chars().count(), 60);
    }

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
