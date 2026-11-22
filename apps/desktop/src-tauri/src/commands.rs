//! Opening and closing documents, and the local state the app remembers:
//! recent files, where each file was left, settings, and the open tabs.
//!
//! Paths stay in the shell. The UI names a file by an id (open documents), a
//! token (recent files, files waiting for a password), or not at all (the
//! picker); it receives file names for display only (ADR 0001).

use std::collections::HashMap;
use std::path::Path;

use safha_core::catalog::{PageLayout, ReadingDirection};
use safha_core::store::{Session, SessionWindow, ViewState};
use safha_core::{Document, Engine, PdfiumSource};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, MAIN_WINDOW, WindowTabs, display_name};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    devtools: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    id: u32,
    /// File name for display. The UI never receives the full path.
    name: String,
    page_count: usize,
    /// Where the file was left last time, if it was opened before.
    view_state: Option<ViewState>,
    /// The document's own reading preferences (`/ViewerPreferences
    /// /Direction`, `/PageLayout`), the defaults for its page order and layout.
    reading_direction: Option<&'static str>,
    page_layout: Option<&'static str>,
    /// Restored with unsaved changes (ADR 0040).
    modified: bool,
    can_undo: bool,
}

impl DocumentInfo {
    /// A restored document's edit state.
    pub fn with_edits(mut self, modified: bool, can_undo: bool) -> Self {
        self.modified = modified;
        self.can_undo = can_undo;
        self
    }

    pub fn new(id: u32, name: String, document: &Document, view_state: Option<ViewState>) -> Self {
        let hints = document.catalog_hints();
        Self {
            id,
            name,
            page_count: document.page_count(),
            view_state,
            reading_direction: hints.direction.map(|direction| match direction {
                ReadingDirection::LeftToRight => "ltr",
                ReadingDirection::RightToLeft => "rtl",
            }),
            // In the UI's vocabulary (viewer/layout.ts): odd pages on the
            // right means the first page stands alone, as in a book.
            page_layout: hints.page_layout.map(|layout| match layout {
                PageLayout::SinglePage => "single",
                PageLayout::OneColumn => "continuous",
                PageLayout::TwoColumnLeft | PageLayout::TwoPageLeft => "spread",
                PageLayout::TwoColumnRight | PageLayout::TwoPageRight => "book",
            }),
            modified: false,
            can_undo: false,
        }
    }
}

/// What opening a file led to.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenOutcome {
    Opened {
        document: DocumentInfo,
    },
    /// The file is encrypted. `unlock_document(token, password)` continues;
    /// `retry` is true when a password was given and was wrong.
    NeedsPassword {
        token: u32,
        name: String,
        retry: bool,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Startup {
    /// Restored tabs (and the launch document), in tab order.
    documents: Vec<OpenOutcome>,
    /// The document to show first.
    active: Option<u32>,
    /// Unsaved changes an earlier session left (ADR 0040), offered once, at
    /// the app's start, in the main window.
    recoveries: Vec<crate::recovery::RecoveryInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFileInfo {
    token: u32,
    name: String,
    /// Unix seconds.
    last_opened: i64,
    /// False when the file has been moved or deleted since.
    exists: bool,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo { version: env!("CARGO_PKG_VERSION"), devtools: cfg!(feature = "devtools") }
}

/// Shows the native file picker and opens the chosen PDF.
/// Returns `None` when the user cancels.
#[tauri::command]
pub async fn open_document_dialog(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> CommandResult<Option<OpenOutcome>> {
    let picked = app.dialog().file().add_filter("PDF", &["pdf"]).blocking_pick_file();
    let Some(path) = picked.and_then(|file| file.into_path().ok()) else {
        return Ok(None);
    };
    open_path(&state, &path, None, window.label()).map(Some)
}

/// What a window's UI shows when it starts.
///
/// The main window's first call restores the previous session: its own tabs,
/// plus a new window for each other window that was open, and the document
/// the app was launched with. Any other call (a new window, or a UI reload)
/// returns the documents that window already has.
#[tauri::command]
pub async fn startup(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> CommandResult<Startup> {
    let label = window.label();
    if label != MAIN_WINDOW || !state.begin_startup() {
        let documents = state
            .open_documents(label)
            .into_iter()
            .filter_map(|(id, name)| {
                let view_state = state.path_of(id).and_then(|path| view_state_of(&state, &path));
                let document = state.document(id)?;
                Some(OpenOutcome::Opened { document: DocumentInfo::new(id, name, &document, view_state) })
            })
            .collect();
        return Ok(Startup { documents, active: state.window_tabs(label).active, recoveries: Vec::new() });
    }

    let mut documents = Vec::new();
    let mut active = None;
    let restore = state.store().and_then(|store| store.setting("session.restore").ok().flatten());
    if restore.as_deref() != Some("false")
        && let Some(session) = state.store().and_then(|store| store.session().ok())
    {
        for (index, saved) in session.windows.iter().enumerate() {
            let target = if index == 0 { MAIN_WINDOW.to_string() } else { state.add_window() };
            let mut tabs = WindowTabs::default();
            for (position, path) in saved.files.iter().enumerate() {
                // A file that can no longer be read is left out; the rest still open.
                let Ok(outcome) = open_path(&state, path, None, &target) else { continue };
                let id = opened_id(&outcome);
                if let Some(id) = id {
                    tabs.ids.push(id);
                    if saved.active == Some(position) {
                        tabs.active = Some(id);
                    }
                }
                if index == 0 {
                    documents.push(outcome);
                }
            }
            if index == 0 {
                active = tabs.active;
            }
            let opened_any = !tabs.ids.is_empty();
            state.set_window_tabs(&target, tabs);
            if index > 0 && opened_any {
                create_window(&app, &target)?;
            }
        }
    }
    if let Some(path) = state.take_startup_document() {
        let outcome = open_path(&state, &path, None, MAIN_WINDOW)?;
        active = opened_id(&outcome).or(active);
        if !documents
            .iter()
            .any(|known| opened_id(known).is_some() && opened_id(known) == opened_id(&outcome))
        {
            documents.push(outcome);
        }
    }
    Ok(Startup { documents, active, recoveries: crate::recovery::list(&state) })
}

/// Opens an empty window.
#[tauri::command]
pub async fn new_window(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    let label = state.add_window();
    create_window(&app, &label)
}

/// Moves a document's tab into a new window of its own; the calling window's
/// UI drops the tab (without closing the document).
#[tauri::command]
pub async fn move_to_new_window(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<()> {
    if state.document(document_id).is_none() {
        return Err(CommandError::unknown_document(document_id));
    }
    let label = state.add_window();
    state.move_to_window(document_id, &label);
    create_window(&app, &label)
}

/// Prints a document with the platform's own print dialog.
///
/// On Linux, GTK's dialog, with PDFium drawing each page (ADR 0013). Elsewhere
/// a window shows the PDF in the platform's own viewer (WebKit on macOS,
/// WebView2 on Windows) and prints it once loaded, so the system print panel
/// prints the PDF itself at full vector quality (ADR 0012; not yet seen on
/// screen).
#[tauri::command]
pub async fn print_document(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<()> {
    let name = state.path_of(document_id).map(|path| display_name(&path)).unwrap_or_default();
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    print_with_platform(&app, &window, &state, document_id, document, name).await
}

#[cfg(target_os = "linux")]
async fn print_with_platform(
    app: &AppHandle,
    window: &WebviewWindow,
    _state: &AppState,
    _document_id: u32,
    document: std::sync::Arc<Document>,
    name: String,
) -> CommandResult<()> {
    crate::print_linux::print(app, window.label(), document, name).await
}

#[cfg(not(target_os = "linux"))]
async fn print_with_platform(
    app: &AppHandle,
    _window: &WebviewWindow,
    state: &AppState,
    document_id: u32,
    _document: std::sync::Arc<Document>,
    name: String,
) -> CommandResult<()> {
    let label = format!("print-{}", state.next_token());
    let base = if cfg!(windows) { "http://safha.localhost" } else { "safha://localhost" };
    let url = format!("{base}/document?doc={document_id}")
        .parse::<tauri::Url>()
        .map_err(|err| CommandError::new("window_error", err.to_string()))?;
    tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::CustomProtocol(url))
        .title(name)
        .inner_size(900.0, 1100.0)
        .on_page_load(|window, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished
                && let Err(err) = window.print()
            {
                eprintln!("safha: printing failed: {err}");
            }
        })
        .build()
        .map(drop)
        .map_err(|err| CommandError::new("window_error", err.to_string()))
}

/// A window like the main one (tauri.conf.json) showing the same UI.
pub fn create_window(app: &AppHandle, label: &str) -> CommandResult<()> {
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::default())
        .title("Mirsam")
        .inner_size(1280.0, 820.0)
        .min_inner_size(880.0, 560.0)
        .build()
        .map(drop)
        .map_err(|err| CommandError::new("window_error", err.to_string()))
}

/// Tries the password for a file that needs one. A wrong password yields
/// `NeedsPassword` again with `retry: true`. The password is used once and
/// never stored or logged.
#[tauri::command]
pub async fn unlock_document(
    window: WebviewWindow,
    state: State<'_, AppState>,
    token: u32,
    password: String,
) -> CommandResult<OpenOutcome> {
    let path =
        state.pending_path(token).ok_or_else(|| CommandError::new("unknown_token", "no file waiting"))?;
    let outcome = open_path(&state, &path, Some(&password), window.label())?;
    if matches!(outcome, OpenOutcome::Opened { .. }) {
        state.forget_pending(token);
    }
    Ok(outcome)
}

/// The user gave up on a password prompt.
#[tauri::command]
pub fn cancel_unlock(state: State<'_, AppState>, token: u32) {
    state.forget_pending(token);
}

/// Closes a document: its queued renders are cancelled, its cached tiles
/// dropped, and a search index build stopped.
#[tauri::command]
pub fn close_document(state: State<'_, AppState>, document_id: u32) -> bool {
    state.remove_document(document_id)
}

#[tauri::command]
pub fn recent_files(state: State<'_, AppState>, limit: usize) -> CommandResult<Vec<RecentFileInfo>> {
    let Some(store) = state.store() else { return Ok(Vec::new()) };
    let recents = store.recents(limit.min(100)).map_err(store_error)?;
    let tokens = state.remember_recents(recents.iter().map(|recent| recent.path.clone()).collect());
    Ok(recents
        .into_iter()
        .zip(tokens)
        .map(|(recent, token)| RecentFileInfo {
            token,
            name: recent.name,
            last_opened: recent.last_opened,
            exists: recent.exists,
        })
        .collect())
}

#[tauri::command]
pub async fn open_recent(
    window: WebviewWindow,
    state: State<'_, AppState>,
    token: u32,
) -> CommandResult<OpenOutcome> {
    let path =
        state.recent_path(token).ok_or_else(|| CommandError::new("unknown_token", "no such recent file"))?;
    if !path.is_file() {
        return Err(CommandError::new("file_missing", "the file has been moved or deleted"));
    }
    open_path(&state, &path, None, window.label())
}

#[tauri::command]
pub fn forget_recent(state: State<'_, AppState>, token: u32) -> CommandResult<()> {
    let (Some(store), Some(path)) = (state.store(), state.recent_path(token)) else { return Ok(()) };
    store.forget_recent(&path).map_err(store_error)
}

#[tauri::command]
pub fn clear_recents(state: State<'_, AppState>) -> CommandResult<()> {
    state.store().map_or(Ok(()), |store| store.clear_recents().map_err(store_error))
}

/// Remembers where a document was left (page, zoom, position on the page).
#[tauri::command]
pub fn save_view_state(state: State<'_, AppState>, document_id: u32, view: ViewState) -> CommandResult<()> {
    let (Some(store), Some(path)) = (state.store(), state.path_of(document_id)) else { return Ok(()) };
    store.save_view_state(&path, &view).map_err(store_error)
}

/// Remembers the calling window's tabs, in order; the saved session holds
/// every window's, for the next launch.
#[tauri::command]
pub fn save_session(
    window: WebviewWindow,
    state: State<'_, AppState>,
    document_ids: Vec<u32>,
    active: Option<u32>,
) -> CommandResult<()> {
    state.set_window_tabs(window.label(), WindowTabs { ids: document_ids, active });
    write_session(&state)
}

/// Writes every window's tabs to the store.
pub fn write_session(state: &AppState) -> CommandResult<()> {
    let Some(store) = state.store() else { return Ok(()) };
    let windows =
        state.session_windows().into_iter().map(|(files, active)| SessionWindow { files, active }).collect();
    store.save_session(&Session { windows }).map_err(store_error)
}

/// Every stored UI setting (`theme`, `locale`, …).
#[tauri::command]
pub fn settings(state: State<'_, AppState>) -> CommandResult<HashMap<String, String>> {
    let Some(store) = state.store() else { return Ok(HashMap::new()) };
    let mut settings = HashMap::new();
    for key in SETTINGS {
        if let Some(value) = store.setting(key).map_err(store_error)? {
            settings.insert((*key).to_string(), value);
        }
    }
    Ok(settings)
}

/// Stores a setting and tells every window, so all of them follow a change
/// of language, theme, or reading theme made in one.
#[tauri::command]
pub fn set_setting(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> CommandResult<()> {
    if !SETTINGS.contains(&key.as_str()) || value.len() > 1024 {
        return Err(CommandError::new("invalid_setting", format!("unknown setting {key:?}")));
    }
    if let Some(store) = state.store() {
        store.set_setting(&key, &value).map_err(store_error)?;
    }
    let _ = app.emit(SETTINGS_EVENT, SettingChanged { key, value });
    Ok(())
}

/// Event sent to every window when a setting changes.
pub const SETTINGS_EVENT: &str = "settings-changed";

#[derive(Debug, Clone, Serialize)]
struct SettingChanged {
    key: String,
    value: String,
}

/// The settings the UI may store. A closed list: the webview cannot use the
/// store as general-purpose storage.
const SETTINGS: &[&str] = &[
    "locale",
    "theme",
    "readingTheme",
    "leftPanel",
    "session.restore",
    "search.options",
    crate::editing::COMMENT_AUTHOR,
];

/// Opens `path` in a tab of `window`, or switches to it if that window has it
/// open already.
pub fn open_path(
    state: &AppState,
    path: &Path,
    password: Option<&str>,
    window: &str,
) -> CommandResult<OpenOutcome> {
    if let Some(id) = state.find_open(path, window) {
        let document = state.document(id).ok_or_else(|| CommandError::unknown_document(id))?;
        return Ok(OpenOutcome::Opened {
            document: DocumentInfo::new(id, display_name(path), &document, None),
        });
    }
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let document = match engine.open_file(path, password) {
        Ok(document) => document,
        Err(safha_core::Error::PasswordRequired) => {
            return Ok(OpenOutcome::NeedsPassword {
                token: state.lock_pending(path.to_path_buf()),
                name: display_name(path),
                retry: password.is_some(),
            });
        }
        Err(err) => return Err(err.into()),
    };
    let view_state = view_state_of(state, path);
    let (id, document) = state.insert_document(document, path.to_path_buf(), window);
    if let Some(store) = state.store()
        && let Err(err) = store.record_open(path)
    {
        eprintln!("safha: could not record a recent file: {err}");
    }
    Ok(OpenOutcome::Opened { document: DocumentInfo::new(id, display_name(path), &document, view_state) })
}

fn view_state_of(state: &AppState, path: &Path) -> Option<ViewState> {
    state.store().and_then(|store| store.view_state(path).ok().flatten())
}

fn opened_id(outcome: &OpenOutcome) -> Option<u32> {
    match outcome {
        OpenOutcome::Opened { document } => Some(document.id),
        OpenOutcome::NeedsPassword { .. } => None,
    }
}

fn store_error(err: safha_core::store::StoreError) -> CommandError {
    CommandError::new("store_error", err.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/out").join(path)
    }

    fn state_with_store() -> AppState {
        let state = AppState::new(std::time::Instant::now(), None);
        state.set_store(safha_core::store::Store::in_memory().expect("store"));
        state
    }

    #[test]
    fn an_encrypted_file_asks_for_its_password_and_a_wrong_one_asks_again() {
        let state = state_with_store();
        let path = fixture("derived/encrypted.pdf");
        assert!(path.is_file(), "fixture corpus missing: run `node fixtures/generate.mjs`");
        let OpenOutcome::NeedsPassword { token, name, retry } =
            open_path(&state, &path, None, MAIN_WINDOW).expect("asks")
        else {
            panic!("an encrypted file must ask for a password");
        };
        assert_eq!((name.as_str(), retry), ("encrypted.pdf", false));
        let pending = state.pending_path(token).expect("waiting");
        let wrong = open_path(&state, &pending, Some("not it"), MAIN_WINDOW).expect("asks again");
        assert!(matches!(wrong, OpenOutcome::NeedsPassword { retry: true, .. }));
        // fixtures/generate.mjs: ENCRYPTED_PASSWORD.
        let opened = open_path(&state, &pending, Some("safha-test"), MAIN_WINDOW).expect("opens");
        assert!(matches!(opened, OpenOutcome::Opened { .. }));
    }

    #[test]
    fn a_book_that_asks_for_right_to_left_spreads_opens_that_way() {
        let state = state_with_store();
        let OpenOutcome::Opened { document } =
            open_path(&state, &fixture("derived/rtl-book-objstm.pdf"), None, MAIN_WINDOW).expect("opens")
        else {
            panic!("opens");
        };
        assert_eq!((document.reading_direction, document.page_layout), (Some("rtl"), Some("book")));
        let OpenOutcome::Opened { document } =
            open_path(&state, &fixture("chrome/en-paragraphs.pdf"), None, MAIN_WINDOW).expect("opens")
        else {
            panic!("opens");
        };
        assert_eq!((document.reading_direction, document.page_layout), (None, None));
    }

    #[test]
    fn opening_a_file_twice_switches_to_its_tab_and_records_it_once() {
        let state = state_with_store();
        let path = fixture("chrome/en-paragraphs.pdf");
        let first = opened_id(&open_path(&state, &path, None, MAIN_WINDOW).expect("opens")).expect("opened");
        let second = opened_id(&open_path(&state, &path, None, MAIN_WINDOW).expect("opens")).expect("opened");
        assert_eq!(first, second);
        assert_eq!(state.open_documents(MAIN_WINDOW).len(), 1);
        let recents = state.store().expect("store").recents(10).expect("recents");
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].name, "en-paragraphs.pdf");
    }

    #[test]
    fn a_document_remembers_where_it_was_left() {
        let state = state_with_store();
        let path = fixture("chrome/contract-two-column.pdf");
        let id = opened_id(&open_path(&state, &path, None, MAIN_WINDOW).expect("opens")).expect("opened");
        let left = ViewState { page: 0, zoom_mode: "custom".into(), zoom: 1.5, page_offset: 0.25 };
        state
            .store()
            .expect("store")
            .save_view_state(&state.path_of(id).expect("path"), &left)
            .expect("saves");
        assert!(state.remove_document(id));
        let OpenOutcome::Opened { document } = open_path(&state, &path, None, MAIN_WINDOW).expect("opens")
        else {
            panic!("opens");
        };
        assert_eq!(document.view_state, Some(left));
    }
}
