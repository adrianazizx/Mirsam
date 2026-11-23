//! Certificate signatures (ADR 0047): thin commands over `safha_core::sign`.
//!
//! - **Identities.** A PKCS#12 file is picked through a native dialog (the
//!   UI gets its name, never its path), its password asked for, and the
//!   identity kept in memory for this run of the app only. On macOS the
//!   keychain's signing identities are listed too; their keys stay there.
//! - **Signing** writes a new file: the document's bytes (rewritten first
//!   when it is unsigned and edited, as saving would) plus one incremental
//!   update with the signature, over its own file or to one the user picks.
//!   The signed file is then what is shown, and undo starts over from it.
//! - **Checking** a document's signatures trusts the operating system's
//!   roots.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use safha_core::edit::EditHistory;
use safha_core::sign::{
    CertificateSummary, HttpTimeStamper, Identity, MAX_PKCS12_BYTES, SignOptions, SignatureReport,
    TimeStamper, TrustStore, VisibleSignature,
};

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, lock};

/// The setting that holds the time-stamping authority's address.
pub const TSA_SETTING: &str = "signing.tsaUrl";

/// Identities read from PKCS#12 files this run, and files waiting for their
/// password. Never written anywhere.
#[derive(Default)]
pub struct SigningState {
    identities: Mutex<HashMap<u32, Identity>>,
    pending: Mutex<HashMap<u32, (String, Vec<u8>)>>,
    next: AtomicU32,
}

impl SigningState {
    fn token(&self) -> u32 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// A signing identity as the UI lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInfo {
    /// `file:N` (read from a PKCS#12 file this run) or `keychain:<SHA-256>`.
    id: String,
    source: &'static str,
    certificate: CertificateSummary,
}

/// The identities available to sign with.
#[tauri::command]
pub async fn signing_identities(signing: State<'_, SigningState>) -> CommandResult<Vec<IdentityInfo>> {
    let mut list: Vec<IdentityInfo> = lock(&signing.identities)
        .iter()
        .map(|(token, identity)| IdentityInfo {
            id: format!("file:{token}"),
            source: "file",
            certificate: identity.summary(),
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    list.extend(keychain_identities().await?);
    Ok(list)
}

#[cfg(target_os = "macos")]
async fn keychain_identities() -> CommandResult<Vec<IdentityInfo>> {
    let found = tauri::async_runtime::spawn_blocking(safha_core::sign::keychain::identities)
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))?;
    // A keychain that cannot be searched leaves the files.
    Ok(found
        .unwrap_or_default()
        .into_iter()
        .map(|certificate| IdentityInfo {
            id: format!("keychain:{}", certificate.fingerprint),
            source: "keychain",
            certificate,
        })
        .collect())
}

#[cfg(not(target_os = "macos"))]
async fn keychain_identities() -> CommandResult<Vec<IdentityInfo>> {
    Ok(Vec::new())
}

/// A PKCS#12 file picked, waiting for its password.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedCertificate {
    token: u32,
    name: String,
}

/// Asks for a PKCS#12 file (`.p12`, `.pfx`). `None` when the user cancels.
/// `unlock_certificate` takes its password.
#[tauri::command]
pub async fn pick_certificate_file(
    app: AppHandle,
    signing: State<'_, SigningState>,
) -> CommandResult<Option<PickedCertificate>> {
    let picked = app
        .dialog()
        .file()
        .add_filter("PKCS #12", &["p12", "pfx"])
        .blocking_pick_file()
        .and_then(|file| file.into_path().ok());
    let Some(path) = picked else { return Ok(None) };
    let length = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX);
    if length > MAX_PKCS12_BYTES as u64 {
        return Err(CommandError::new("certificate_unreadable", "the file is too large for a PKCS#12 file"));
    }
    let bytes =
        std::fs::read(&path).map_err(|err| CommandError::new("certificate_unreadable", err.to_string()))?;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let token = signing.token();
    lock(&signing.pending).insert(token, (name.clone(), bytes));
    Ok(Some(PickedCertificate { token, name }))
}

/// Opens a picked PKCS#12 file with `password`. A wrong one fails with
/// `certificate_password` and the file stays picked, to try again.
#[tauri::command]
pub async fn unlock_certificate(
    signing: State<'_, SigningState>,
    token: u32,
    password: String,
) -> CommandResult<IdentityInfo> {
    let bytes = lock(&signing.pending)
        .get(&token)
        .map(|(_, bytes)| bytes.clone())
        .ok_or_else(|| CommandError::new("certificate_unreadable", "no such picked file"))?;
    let identity = tauri::async_runtime::spawn_blocking(move || Identity::from_pkcs12(&bytes, &password))
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))??;
    lock(&signing.pending).remove(&token);
    let id = signing.token();
    let certificate = identity.summary();
    lock(&signing.identities).insert(id, identity);
    Ok(IdentityInfo { id: format!("file:{id}"), source: "file", certificate })
}

/// Drops a picked file without opening it.
#[tauri::command]
pub fn cancel_certificate(signing: State<'_, SigningState>, token: u32) {
    lock(&signing.pending).remove(&token);
}

/// Forgets an identity read from a file.
#[tauri::command]
pub fn forget_identity(signing: State<'_, SigningState>, id: String) {
    if let Some(token) = id.strip_prefix("file:").and_then(|token| token.parse::<u32>().ok()) {
        lock(&signing.identities).remove(&token);
    }
}

/// Where a visible signature goes and what it shows: a box on the displayed
/// page (top-left corner and size, in points), a kept signature (by its
/// store id) and the caption lines, worded by the UI.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleArg {
    page: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    signature: Option<i64>,
    caption: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignRequest {
    identity: String,
    reason: Option<String>,
    location: Option<String>,
    contact: Option<String>,
    /// Time-stamp from the configured authority (PAdES B-T).
    timestamp: bool,
    visible: Option<VisibleArg>,
}

/// The document after it was signed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignOutcome {
    /// The file's display name (new after Save As).
    name: String,
    page_count: usize,
    /// Always false: the signed file on disk is what is shown.
    modified: bool,
    signatures: Vec<SignatureReport>,
}

/// Signs the document and saves it over its own file, or, with `save_as`,
/// to a file the user picks (`None` when they cancel the picker).
#[tauri::command]
pub async fn sign_document(
    app: AppHandle,
    state: State<'_, AppState>,
    signing: State<'_, SigningState>,
    document_id: u32,
    request: SignRequest,
    save_as: bool,
) -> CommandResult<Option<SignOutcome>> {
    let current = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let identity = identity_for(&signing, &request.identity)?;
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let visible = match &request.visible {
        None => None,
        Some(arg) => {
            let graphic = match arg.signature {
                None => None,
                Some(id) => Some(
                    state
                        .store()
                        .and_then(|store| store.signature(id).ok().flatten())
                        .ok_or_else(|| CommandError::new("store_unavailable", "no such kept signature"))?
                        .signature,
                ),
            };
            let view = document.view_transform(arg.page)?;
            Some(VisibleSignature::at(
                arg.page,
                (arg.x, arg.y),
                (arg.width, arg.height),
                &view,
                graphic,
                arg.caption.clone(),
                safha_core::annotate::Signature::DEFAULT_COLOR,
            )?)
        }
    };
    let options = SignOptions {
        reason: request.reason.clone(),
        location: request.location.clone(),
        contact: request.contact.clone(),
        visible,
    };
    let timestamper = if request.timestamp {
        let url = state
            .store()
            .and_then(|store| store.setting(TSA_SETTING).ok().flatten())
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| CommandError::new("timestamp_url", "no time-stamping authority is set"))?;
        Some(HttpTimeStamper::new(&url)?)
    } else {
        None
    };

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

    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let modified = lock(&history).is_modified();
    let signed = tauri::async_runtime::spawn_blocking(move || {
        let stamper = timestamper.as_ref().map(|t| t as &dyn TimeStamper);
        document.sign(&identity, &options, stamper, modified)
    })
    .await
    .map_err(|err| CommandError::new("engine_error", err.to_string()))??;
    finish(&state, document_id, path, signed).map(Some)
}

/// Writes the signed bytes and shows them from now on.
fn finish(
    state: &AppState,
    document_id: u32,
    path: PathBuf,
    signed: std::sync::Arc<[u8]>,
) -> CommandResult<SignOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let reopened = document
        .revision(std::sync::Arc::clone(&signed))
        .map_err(|_| CommandError::new("save_failed", "the signed file would not open"))?;
    if reopened.page_count() != document.page_count() {
        return Err(CommandError::new(
            "save_failed",
            "the signed file would have a different number of pages",
        ));
    }
    safha_core::save::write_atomically(&path, &signed)?;
    let signatures = reopened.signatures(TrustStore::system());
    let page_count = reopened.page_count();
    *history = EditHistory::new(signed);
    state.replace_revision(document_id, reopened);
    let name = state.set_path(document_id, path.clone()).unwrap_or_default();
    if let Some(store) = state.store()
        && let Err(err) = store.record_open(&path)
    {
        eprintln!("safha: could not record a recent file: {err}");
    }
    drop(history);
    // Saved: its recovery file goes (ADR 0040).
    crate::recovery::write_now(state, document_id)?;
    Ok(SignOutcome { name, page_count, modified: false, signatures })
}

fn identity_for(signing: &SigningState, id: &str) -> CommandResult<Identity> {
    if let Some(token) = id.strip_prefix("file:").and_then(|token| token.parse::<u32>().ok()) {
        return lock(&signing.identities)
            .get(&token)
            .cloned()
            .ok_or_else(|| CommandError::new("certificate_unreadable", "that identity is no longer loaded"));
    }
    #[cfg(target_os = "macos")]
    if let Some(fingerprint) = id.strip_prefix("keychain:") {
        return Ok(safha_core::sign::keychain::identity(fingerprint)?);
    }
    Err(CommandError::new("certificate_unreadable", format!("unknown identity {id:?}")))
}

/// The document's signatures, checked against the system's trusted roots.
#[tauri::command]
pub async fn document_signatures(
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Vec<SignatureReport>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    tauri::async_runtime::spawn_blocking(move || document.signatures(TrustStore::system()))
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_request_reads_from_the_ui() {
        let json = r#"{"identity":"file:1","reason":"موافق","location":null,"contact":null,"timestamp":true,
            "visible":{"page":0,"x":10,"y":20,"width":180,"height":72,"signature":3,"caption":"وقّعه سارة"}}"#;
        let request: SignRequest = serde_json::from_str(json).expect("parses");
        assert!(request.timestamp);
        let visible = request.visible.expect("visible");
        assert_eq!((visible.page, visible.signature, visible.caption.as_str()), (0, Some(3), "وقّعه سارة"));
    }

    #[test]
    fn unknown_identities_are_refused() {
        let signing = SigningState::default();
        for id in ["file:9", "nothing", "keychain"] {
            assert!(identity_for(&signing, id).is_err(), "{id}");
        }
    }
}
