//! Opening and closing documents, and the local state the app remembers:
//! recent files, where each file was left, settings, and the open tabs.
//!
//! Paths stay in the shell. The UI names a file by an id (open documents), a
//! token (recent files, files waiting for a password), or not at all (the
//! picker); it receives file names for display only (ADR 0001).

use std::collections::HashMap;
use std::path::Path;

use safha_core::store::{Session, ViewState};
use safha_core::{Engine, PdfiumSource};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, display_name};

/// Length of the little-endian header that precedes the RGBA bytes in a
/// bitmap response (`render_tile`): width, height, render time in
/// microseconds, flags (`viewer::FLAG_*`).
pub const BITMAP_HEADER_LEN: usize = 16;

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
    state: State<'_, AppState>,
) -> CommandResult<Option<OpenOutcome>> {
    let picked = app.dialog().file().add_filter("PDF", &["pdf"]).blocking_pick_file();
    let Some(path) = picked.and_then(|file| file.into_path().ok()) else {
        return Ok(None);
    };
    open_path(&state, &path, None).map(Some)
}

/// What the UI shows when it starts. The first call restores the previous
/// session's tabs and opens the document the app was launched with; a later
/// call (the UI reloaded) returns the documents that are still open.
#[tauri::command]
pub async fn startup(state: State<'_, AppState>) -> CommandResult<Startup> {
    if !state.begin_startup() {
        let documents = state
            .open_documents()
            .into_iter()
            .map(|(id, name, page_count)| {
                let view_state = state.path_of(id).and_then(|path| view_state_of(&state, &path));
                OpenOutcome::Opened { document: DocumentInfo { id, name, page_count, view_state } }
            })
            .collect();
        return Ok(Startup { documents, active: None });
    }

    let mut documents = Vec::new();
    let mut active = None;
    let restore = state.store().and_then(|store| store.setting("session.restore").ok().flatten());
    if restore.as_deref() != Some("false")
        && let Some(session) = state.store().and_then(|store| store.session().ok())
    {
        for (position, path) in session.files.iter().enumerate() {
            // A file that can no longer be read is left out; the rest still open.
            let Ok(outcome) = open_path(&state, path, None) else { continue };
            if session.active == Some(position) {
                active = opened_id(&outcome);
            }
            documents.push(outcome);
        }
    }
    if let Some(path) = state.take_startup_document() {
        let outcome = open_path(&state, &path, None)?;
        active = opened_id(&outcome).or(active);
        if !documents
            .iter()
            .any(|known| opened_id(known).is_some() && opened_id(known) == opened_id(&outcome))
        {
            documents.push(outcome);
        }
    }
    Ok(Startup { documents, active })
}

/// Tries the password for a file that needs one. A wrong password yields
/// `NeedsPassword` again with `retry: true`. The password is used once and
/// never stored or logged.
#[tauri::command]
pub async fn unlock_document(
    state: State<'_, AppState>,
    token: u32,
    password: String,
) -> CommandResult<OpenOutcome> {
    let path =
        state.pending_path(token).ok_or_else(|| CommandError::new("unknown_token", "no file waiting"))?;
    let outcome = open_path(&state, &path, Some(&password))?;
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
pub async fn open_recent(state: State<'_, AppState>, token: u32) -> CommandResult<OpenOutcome> {
    let path =
        state.recent_path(token).ok_or_else(|| CommandError::new("unknown_token", "no such recent file"))?;
    if !path.is_file() {
        return Err(CommandError::new("file_missing", "the file has been moved or deleted"));
    }
    open_path(&state, &path, None)
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

/// Remembers the open tabs, in order, for the next launch.
#[tauri::command]
pub fn save_session(
    state: State<'_, AppState>,
    document_ids: Vec<u32>,
    active: Option<u32>,
) -> CommandResult<()> {
    let Some(store) = state.store() else { return Ok(()) };
    let mut session = Session::default();
    for id in document_ids {
        if let Some(path) = state.path_of(id) {
            if active == Some(id) {
                session.active = Some(session.files.len());
            }
            session.files.push(path);
        }
    }
    store.save_session(&session).map_err(store_error)
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

#[tauri::command]
pub fn set_setting(state: State<'_, AppState>, key: String, value: String) -> CommandResult<()> {
    if !SETTINGS.contains(&key.as_str()) || value.len() > 1024 {
        return Err(CommandError::new("invalid_setting", format!("unknown setting {key:?}")));
    }
    state.store().map_or(Ok(()), |store| store.set_setting(&key, &value).map_err(store_error))
}

/// The settings the UI may store. A closed list: the webview cannot use the
/// store as general-purpose storage.
const SETTINGS: &[&str] =
    &["locale", "theme", "readingTheme", "leftPanel", "session.restore", "search.options"];

/// A bitmap as raw bytes: a [`BITMAP_HEADER_LEN`]-byte header followed by
/// straight RGBA. Binary all the way to the webview — never base64 or JSON
/// (spec section 9, "Rendering design").
#[cfg(any(test, feature = "devtools"))]
pub fn encode_bitmap(bitmap: &safha_core::Bitmap, render_micros: u128) -> Vec<u8> {
    encode_bitmap_with_flags(bitmap, render_micros, 0)
}

pub fn encode_bitmap_with_flags(bitmap: &safha_core::Bitmap, render_micros: u128, flags: u32) -> Vec<u8> {
    let rgba = bitmap.as_rgba();
    let mut out = Vec::with_capacity(BITMAP_HEADER_LEN + rgba.len());
    out.extend_from_slice(&bitmap.width().to_le_bytes());
    out.extend_from_slice(&bitmap.height().to_le_bytes());
    out.extend_from_slice(&u32::try_from(render_micros).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(rgba);
    out
}

/// Opens `path`, or switches to it if it is already open in a tab.
pub fn open_path(state: &AppState, path: &Path, password: Option<&str>) -> CommandResult<OpenOutcome> {
    if let Some(id) = state.find_open(path) {
        let document = state.document(id).ok_or_else(|| CommandError::unknown_document(id))?;
        let info = DocumentInfo {
            id,
            name: display_name(path),
            page_count: document.page_count(),
            view_state: None,
        };
        return Ok(OpenOutcome::Opened { document: info });
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
    let (id, document) = state.insert_document(document, path.to_path_buf());
    if let Some(store) = state.store()
        && let Err(err) = store.record_open(path)
    {
        eprintln!("safha: could not record a recent file: {err}");
    }
    Ok(OpenOutcome::Opened {
        document: DocumentInfo {
            id,
            name: display_name(path),
            page_count: document.page_count(),
            view_state,
        },
    })
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

    /// The wire format is a contract with `apps/desktop/src/backend/bitmap.ts`.
    #[test]
    fn bitmap_payload_is_a_16_byte_little_endian_header_then_rgba() {
        // A one-page PDF with a 3x2 pt page, rendered at 1 px/pt.
        let body = "<< /Type /Catalog /Pages 2 0 R >>";
        let objects = [
            body.to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );

        let engine = Engine::global(&PdfiumSource::Auto).expect("PDFium is available");
        let bitmap = engine.open_bytes(pdf, None).expect("opens").render_page(0, 1.0).expect("renders");
        let payload = encode_bitmap(&bitmap, 1234);

        assert_eq!(payload.len(), BITMAP_HEADER_LEN + 3 * 2 * 4);
        assert_eq!(payload[0..4], 3u32.to_le_bytes());
        assert_eq!(payload[4..8], 2u32.to_le_bytes());
        assert_eq!(payload[8..12], 1234u32.to_le_bytes());
        assert_eq!(payload[12..16], [0; 4]);
        // A blank page is opaque white.
        assert!(payload[BITMAP_HEADER_LEN..].iter().all(|byte| *byte == 255));
        // The last header word carries flags (bit 0: served from the tile cache).
        let flagged = encode_bitmap_with_flags(&bitmap, 0, crate::viewer::FLAG_FROM_CACHE);
        assert_eq!(flagged[12..16], 1u32.to_le_bytes());
    }

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
        let OpenOutcome::NeedsPassword { token, name, retry } = open_path(&state, &path, None).expect("asks")
        else {
            panic!("an encrypted file must ask for a password");
        };
        assert_eq!((name.as_str(), retry), ("encrypted.pdf", false));
        let pending = state.pending_path(token).expect("waiting");
        let wrong = open_path(&state, &pending, Some("not it")).expect("asks again");
        assert!(matches!(wrong, OpenOutcome::NeedsPassword { retry: true, .. }));
        // fixtures/generate.mjs: ENCRYPTED_PASSWORD.
        let opened = open_path(&state, &pending, Some("safha-test")).expect("opens");
        assert!(matches!(opened, OpenOutcome::Opened { .. }));
    }

    #[test]
    fn opening_a_file_twice_switches_to_its_tab_and_records_it_once() {
        let state = state_with_store();
        let path = fixture("chrome/en-paragraphs.pdf");
        let first = opened_id(&open_path(&state, &path, None).expect("opens")).expect("opened");
        let second = opened_id(&open_path(&state, &path, None).expect("opens")).expect("opened");
        assert_eq!(first, second);
        assert_eq!(state.open_documents().len(), 1);
        let recents = state.store().expect("store").recents(10).expect("recents");
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].name, "en-paragraphs.pdf");
    }

    #[test]
    fn a_document_remembers_where_it_was_left() {
        let state = state_with_store();
        let path = fixture("chrome/contract-two-column.pdf");
        let id = opened_id(&open_path(&state, &path, None).expect("opens")).expect("opened");
        let left = ViewState { page: 0, zoom_mode: "custom".into(), zoom: 1.5, page_offset: 0.25 };
        state
            .store()
            .expect("store")
            .save_view_state(&state.path_of(id).expect("path"), &left)
            .expect("saves");
        assert!(state.remove_document(id));
        let OpenOutcome::Opened { document } = open_path(&state, &path, None).expect("opens") else {
            panic!("opens");
        };
        assert_eq!(document.view_state, Some(left));
    }
}
