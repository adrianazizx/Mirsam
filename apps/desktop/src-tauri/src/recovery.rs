//! Autosave and recovery (ADR 0040): thin commands over
//! `safha_core::recovery` and the state store.
//!
//! - **Autosave.** After every edit, undo, or redo, a document with unsaved
//!   changes has its current revision written to its recovery file a moment
//!   later (edits in a burst write once). A document back at its saved
//!   revision has none.
//! - **Recovery.** A recovery file left when the app starts belongs to a
//!   session that ended without saving or discarding (a crash, a power cut,
//!   a killed process). The UI lists them; each is restored as a modified
//!   document (undo goes back to the original, when it is unchanged) or
//!   discarded.
//! - Saving, and closing without saving (a tab, a window, quitting), remove
//!   a document's recovery file.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewWindow};

use safha_core::edit::EditHistory;
use safha_core::recovery::{FileStamp, RecoveryFiles};
use safha_core::store::RecoveryEntry;
use safha_core::{Engine, PdfiumSource};

use crate::commands::{DocumentInfo, OpenOutcome};
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, display_name, lock};

/// How long after a change its recovery file is written: edits in quick
/// succession (a drawn stroke, several deletions) write once.
const DELAY: Duration = Duration::from_secs(2);

/// Opens the recovery directory next to the state store.
pub fn open_files(state: &AppState, state_dir: &Path) {
    state.set_recovery(RecoveryFiles::new(state_dir.join("recovery")));
}

/// A document changed (an edit, undo, or redo): its recovery file is brought
/// up to date after `DELAY`, unless it changes again meanwhile.
pub fn changed(app: &AppHandle, id: u32) {
    let generation = app.state::<AppState>().bump_autosave(id);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(DELAY);
        let state = app.state::<AppState>();
        if state.autosave_generation(id) == Some(generation)
            && let Err(err) = write_now(&state, id)
        {
            eprintln!("safha: autosave failed: {}", err.detail);
        }
    });
}

/// Writes the document's recovery file now, or removes it when the document
/// holds no unsaved changes.
pub fn write_now(state: &AppState, id: u32) -> CommandResult<()> {
    let (Some(files), Some(store)) = (state.recovery(), state.store()) else { return Ok(()) };
    let Some(file) = state.autosave_target(id) else { return Ok(()) };
    let (modified, bytes) = {
        let history = lock(&file.history);
        (history.is_modified(), history.current())
    };
    if !modified {
        forget(state, &file.key);
        return Ok(());
    }
    files.write(&file.key, &bytes)?;
    store
        .save_recovery(&RecoveryEntry {
            key: file.key.clone(),
            path: file.path,
            name: file.name,
            saved_at: 0,
            original: file.original,
        })
        .map_err(|err| CommandError::new("store_error", err.to_string()))?;
    // Saved, or undone back to the saved revision, while this was written.
    if !lock(&file.history).is_modified() {
        forget(state, &file.key);
    }
    Ok(())
}

/// Removes a recovery file and its entry.
pub fn forget(state: &AppState, key: &str) {
    if let Some(files) = state.recovery() {
        files.remove(key);
    }
    if let Some(store) = state.store()
        && let Err(err) = store.forget_recovery(key)
    {
        eprintln!("safha: could not forget a recovery file: {err}");
    }
}

/// A recovery file left by an earlier session, as the UI lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryInfo {
    /// What `restore_recovery` and `discard_recovery` take: the UI never sees paths.
    token: u32,
    name: String,
    /// When it was last written (Unix seconds).
    saved_at: i64,
    /// The original file changed or went away since: the recovered version
    /// can only be saved as a new file.
    original_changed: bool,
}

/// Recovery files that belong to no open document: what an earlier session
/// left unsaved. Entries whose file is gone are forgotten.
#[tauri::command]
pub fn recoveries(state: State<'_, AppState>) -> Vec<RecoveryInfo> {
    list(&state)
}

pub fn list(state: &AppState) -> Vec<RecoveryInfo> {
    let (Some(files), Some(store)) = (state.recovery(), state.store()) else { return Vec::new() };
    let entries = match store.recoveries() {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("safha: could not list recovery files: {err}");
            return Vec::new();
        }
    };
    let open = state.recovery_keys();
    let mut listed = Vec::new();
    for entry in entries {
        if open.contains(&entry.key) {
            continue;
        }
        if !files.exists(&entry.key) {
            forget(state, &entry.key);
            continue;
        }
        let original_changed = changed_since(&entry);
        listed.push(RecoveryInfo {
            token: state.remember_recovery(entry.key.clone()),
            name: entry.name,
            saved_at: entry.saved_at,
            original_changed,
        });
    }
    listed
}

fn changed_since(entry: &RecoveryEntry) -> bool {
    entry.original.is_none() || FileStamp::of(&entry.path) != entry.original
}

/// What restoring led to: the document (or its password prompt), and the
/// tab it replaced, when the original was open here.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    outcome: OpenOutcome,
    replaced: Option<u32>,
}

/// Opens a recovery file as a document with unsaved changes. An encrypted
/// one asks for its password first (`unlock_document` continues).
#[tauri::command]
pub async fn restore_recovery(
    window: WebviewWindow,
    state: State<'_, AppState>,
    token: u32,
) -> CommandResult<RestoreOutcome> {
    let key =
        state.recovery_key(token).ok_or_else(|| CommandError::new("unknown_token", "no such recovery"))?;
    restore(&state, &key, None, window.label())
}

/// Tries the password of an encrypted recovery file (the `token` its
/// restore gave). A wrong one asks again; the password is never stored.
#[tauri::command]
pub async fn unlock_recovery(
    window: WebviewWindow,
    state: State<'_, AppState>,
    token: u32,
    password: String,
) -> CommandResult<RestoreOutcome> {
    let key = state
        .pending_recovery(token)
        .ok_or_else(|| CommandError::new("unknown_token", "no recovery waiting"))?;
    let restored = restore(&state, &key, Some(&password), window.label())?;
    if matches!(restored.outcome, OpenOutcome::Opened { .. }) {
        state.forget_pending(token);
    }
    Ok(restored)
}

/// Restores `key` with `password` (none, or the one the user typed). The
/// document keeps the recovery file's key, so the file stays until the
/// document is saved or discarded.
pub fn restore(
    state: &AppState,
    key: &str,
    password: Option<&str>,
    window: &str,
) -> CommandResult<RestoreOutcome> {
    let (Some(files), Some(store)) = (state.recovery(), state.store()) else {
        return Err(CommandError::new("unknown_token", "no recovery files"));
    };
    let entry = store
        .recoveries()
        .map_err(|err| CommandError::new("store_error", err.to_string()))?
        .into_iter()
        .find(|entry| entry.key == key)
        .ok_or_else(|| CommandError::new("unknown_token", "no such recovery"))?;
    let recovered: Arc<[u8]> = Arc::from(files.read(key)?);
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let document = match engine.open_bytes(recovered.to_vec(), password) {
        Ok(document) => document,
        Err(safha_core::Error::PasswordRequired) => {
            return Ok(RestoreOutcome {
                outcome: OpenOutcome::NeedsPassword {
                    token: state.lock_pending_recovery(key.to_string()),
                    name: entry.name.clone(),
                    retry: password.is_some(),
                },
                replaced: None,
            });
        }
        Err(err) => return Err(err.into()),
    };
    // Undo goes back to the original when it is still the file that was edited.
    let changed = changed_since(&entry);
    let history = match (changed, std::fs::read(&entry.path)) {
        (false, Ok(original)) => EditHistory::recovered(Arc::from(original), Arc::clone(&recovered)),
        _ => EditHistory::unsaved(Arc::clone(&recovered)),
    };
    // The original open in this window (the session reopened it) gives way.
    let replaced = state.find_open(&entry.path, window);
    if let Some(id) = replaced {
        state.remove_document(id);
    }
    let (can_undo, modified) = (history.can_undo(), history.is_modified());
    let (id, document) = state.insert_restored(
        document,
        entry.path.clone(),
        window,
        history,
        key.to_string(),
        entry.original,
        changed,
    );
    Ok(RestoreOutcome {
        outcome: OpenOutcome::Opened {
            document: DocumentInfo::new(id, display_name(&entry.path), &document, None)
                .with_edits(modified, can_undo),
        },
        replaced,
    })
}

/// Deletes a recovery file the user does not want.
#[tauri::command]
pub fn discard_recovery(state: State<'_, AppState>, token: u32) {
    if let Some(key) = state.recovery_key(token) {
        forget(&state, &key);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use safha_core::edit::PageEdit;
    use safha_core::store::Store;

    use super::*;
    use crate::state::MAIN_WINDOW;

    fn fixture(path: &str) -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/out").join(path);
        assert!(path.is_file(), "fixture corpus missing: run `node fixtures/generate.mjs`");
        path
    }

    /// A fresh folder holding a copy of `source` as `name`.
    fn scratch(source: &str, name: &str) -> (PathBuf, PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "safha-recovery-shell-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join(name);
        std::fs::copy(fixture(source), &file).expect("copy");
        (dir, file)
    }

    /// The app's state over a state folder, as at launch.
    fn launch(dir: &Path) -> AppState {
        let state = AppState::new(Instant::now(), None);
        state.set_store(Store::open(&dir.join("state.sqlite3")).expect("store"));
        open_files(&state, dir);
        state
    }

    fn opened(outcome: &OpenOutcome) -> (u32, serde_json::Value) {
        let json = serde_json::to_value(outcome).expect("serializes");
        let id = json["document"]["id"].as_u64().expect("opened") as u32;
        (id, json["document"].clone())
    }

    /// Turns page 1 a quarter, as the edit command does.
    fn rotate(state: &AppState, id: u32) {
        let history = state.history(id).expect("history");
        let mut history = lock(&history);
        let document = state.document(id).expect("document");
        let (revision, update) =
            document.with_page_edit(&PageEdit::Rotate { pages: vec![0], quarter_turns: 1 }).expect("edits");
        history.push(update);
        drop(history);
        state.replace_revision(id, revision);
    }

    fn width(state: &AppState, id: u32) -> f32 {
        state.document(id).expect("document").page_size(0).expect("size").width
    }

    #[test]
    fn unsaved_edits_survive_a_crash_and_undo_back_to_the_original() {
        let (dir, file) = scratch("chrome/en-paragraphs.pdf", "عقد.pdf");
        let state = launch(&dir);
        let (id, _) = opened(&crate::commands::open_path(&state, &file, None, MAIN_WINDOW).expect("opens"));
        let upright = width(&state, id);
        rotate(&state, id);
        write_now(&state, id).expect("autosaves");
        assert!(list(&state).is_empty(), "an open document's own file is not offered");
        // A crash: the app goes without closing anything.
        drop(state);

        let state = launch(&dir);
        let listed = list(&state);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "عقد.pdf");
        assert!(!listed[0].original_changed);
        let key = state.recovery_key(listed[0].token).expect("key");
        let restored = restore(&state, &key, None, MAIN_WINDOW).expect("restores");
        assert_eq!(restored.replaced, None);
        let (id, info) = opened(&restored.outcome);
        assert_eq!((info["modified"].as_bool(), info["canUndo"].as_bool()), (Some(true), Some(true)));
        assert!((width(&state, id) - upright).abs() > 1.0, "the page is still turned");

        // Undo goes back to the file as it is on disk.
        let history = state.history(id).expect("history");
        let original = lock(&history).undo().expect("undo");
        assert_eq!(&*original, std::fs::read(&file).expect("read").as_slice());
        drop(history);

        // Saving removes the recovery file.
        lock(&state.history(id).expect("history")).redo();
        crate::editing::save_to_for_tests(&state, id, file.clone()).expect("saves");
        assert!(state.store().expect("store").recoveries().expect("lists").is_empty());
        assert!(!state.recovery().expect("files").exists(&key));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_changed_original_is_offered_but_saved_somewhere_new() {
        let (dir, file) = scratch("chrome/mixed-inline.pdf", "a.pdf");
        let state = launch(&dir);
        let (id, _) = opened(&crate::commands::open_path(&state, &file, None, MAIN_WINDOW).expect("opens"));
        rotate(&state, id);
        write_now(&state, id).expect("autosaves");
        drop(state);
        // Someone else saves over the original meanwhile.
        std::fs::copy(fixture("chrome/en-paragraphs.pdf"), &file).expect("replace");

        let state = launch(&dir);
        let listed = list(&state);
        assert!(listed[0].original_changed);
        let key = state.recovery_key(listed[0].token).expect("key");
        let (id, info) = opened(&restore(&state, &key, None, MAIN_WINDOW).expect("restores").outcome);
        assert_eq!((info["modified"].as_bool(), info["canUndo"].as_bool()), (Some(true), Some(false)));
        assert!(state.save_as_first(id));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn undoing_to_the_saved_revision_or_closing_removes_the_recovery_file() {
        let (dir, file) = scratch("chrome/en-paragraphs.pdf", "b.pdf");
        let state = launch(&dir);
        let (id, _) = opened(&crate::commands::open_path(&state, &file, None, MAIN_WINDOW).expect("opens"));
        let count = |state: &AppState| state.store().expect("store").recoveries().expect("lists").len();
        rotate(&state, id);
        write_now(&state, id).expect("autosaves");
        assert_eq!(count(&state), 1);
        let history = state.history(id).expect("history");
        let bytes = lock(&history).undo().expect("undo");
        drop(history);
        state.replace_revision(id, state.document(id).expect("doc").revision(bytes).expect("opens"));
        write_now(&state, id).expect("autosaves");
        assert_eq!(count(&state), 0, "back at the saved revision: nothing to recover");

        rotate(&state, id);
        write_now(&state, id).expect("autosaves");
        assert_eq!(count(&state), 1);
        state.remove_document(id);
        assert_eq!(count(&state), 0, "closed without saving: discarded");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_encrypted_recovery_file_asks_for_its_password() {
        let (dir, file) = scratch("derived/encrypted.pdf", "secret.pdf");
        let state = launch(&dir);
        let outcome =
            crate::commands::open_path(&state, &file, Some("safha-test"), MAIN_WINDOW).expect("opens");
        let (id, _) = opened(&outcome);
        rotate(&state, id);
        write_now(&state, id).expect("autosaves");
        let key = state.recovery_keys().into_iter().next().expect("key");
        // The recovery file is as encrypted as the original.
        let bytes = state.recovery().expect("files").read(&key).expect("reads");
        assert!(Engine::global(&PdfiumSource::Auto).expect("engine").open_bytes(bytes, None).is_err());
        drop(state);

        let state = launch(&dir);
        let token = list(&state)[0].token;
        let key = state.recovery_key(token).expect("key");
        let asked = restore(&state, &key, None, MAIN_WINDOW).expect("asks");
        let OpenOutcome::NeedsPassword { token, retry: false, .. } = asked.outcome else {
            panic!("an encrypted recovery file must ask for its password");
        };
        assert_eq!(state.pending_recovery(token).as_deref(), Some(key.as_str()));
        let wrong = restore(&state, &key, Some("not it"), MAIN_WINDOW).expect("asks again");
        assert!(matches!(wrong.outcome, OpenOutcome::NeedsPassword { retry: true, .. }));
        let right = restore(&state, &key, Some("safha-test"), MAIN_WINDOW).expect("restores");
        assert!(matches!(right.outcome, OpenOutcome::Opened { .. }));
        let _ = std::fs::remove_dir_all(dir);
    }
}
