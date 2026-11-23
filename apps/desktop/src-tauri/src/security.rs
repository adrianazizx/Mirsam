//! Encryption and permissions on save (ADR 0043): a thin command over
//! `mirsam_core::save::save_secured`.

use std::path::PathBuf;

use mirsam_core::edit::EditHistory;
use mirsam_core::security::{SecurityChange, SecurityInfo};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, lock};

/// The document after it was saved with another protection.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecureOutcome {
    /// The file's display name (new after Save As).
    name: String,
    /// Always false: the file on disk is what is shown.
    modified: bool,
    security: SecurityInfo,
}

/// Saves the document with its passwords and permissions set, changed or
/// removed (`change`), over its own file, or, with `save_as`, to a file the
/// user picks. Earlier edits are written in, and undo starts over: the saved
/// file is a new base (its bytes are encrypted differently). `None` when the
/// user cancels the picker.
#[tauri::command]
pub async fn secure_document(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    change: SecurityChange,
    save_as: bool,
) -> CommandResult<Option<SecureOutcome>> {
    let current = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let path = if save_as {
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
        path
    } else {
        if state.save_as_first(document_id) {
            return Err(CommandError::new("save_as_required", "the original changed since"));
        }
        current
    };
    secure_to(&state, document_id, path, &change).map(Some)
}

fn secure_to(
    state: &AppState,
    document_id: u32,
    path: PathBuf,
    change: &SecurityChange,
) -> CommandResult<SecureOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let reopened = mirsam_core::save::save_secured(&document, &path, change)?;
    let security = reopened.security();
    *history = EditHistory::new(reopened.bytes());
    state.replace_revision(document_id, reopened);
    let name = state.set_path(document_id, path.clone()).unwrap_or_default();
    if let Some(store) = state.store()
        && let Err(err) = store.record_open(&path)
    {
        eprintln!("mirsam: could not record a recent file: {err}");
    }
    // Saved: its recovery file goes (ADR 0040).
    crate::recovery::write_now(state, document_id)?;
    Ok(SecureOutcome { name, modified: false, security })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirsam_core::security::{Cipher, Permissions, Protection};

    #[test]
    fn the_outcome_names_the_new_protection() {
        let change = SecurityChange::Set(Protection {
            cipher: Cipher::Aes256,
            user_password: "x".into(),
            owner_password: String::new(),
            permissions: Permissions::all(),
        });
        let json = serde_json::to_string(&change).expect("serialises");
        assert!(json.contains("\"kind\":\"set\"") && json.contains("userPassword"), "{json}");
        let back: SecurityChange = serde_json::from_str(&json).expect("round trips");
        assert_eq!(back, change);
    }
}
