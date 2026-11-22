//! OCR model downloads (ADR 0034): thin commands over `safha_core::models`.
//!
//! A download runs on its own thread and reports to every window through
//! [`PROGRESS_EVENT`]; the command that starts it returns at once. The UI gets
//! package ids, sizes and states, never paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use safha_core::models::{
    Catalog, Downloader, Hardware, InstalledPackage, ModelError, ModelStore, PackageStatus, Phase,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::{CommandError, CommandResult};
use crate::navigation::blocking;
use crate::state::lock;

/// Event sent to every window as a download moves on.
pub const PROGRESS_EVENT: &str = "ocr-model-progress";

/// Progress is sent at most this often (a 5 GB download reads 20,000 pieces).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

pub struct ModelsState {
    /// Absent when the app has no data directory: nothing can be installed.
    store: Option<ModelStore>,
    catalog: Catalog,
    /// Running downloads by package id, with their cancel flags.
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    hardware: OnceLock<Hardware>,
}

impl ModelsState {
    /// The store in `root`, tidied of an earlier run's leftovers.
    pub fn new(root: Option<PathBuf>) -> Self {
        let store = root.map(ModelStore::new);
        if let Some(store) = &store {
            store.clean();
        }
        Self {
            store,
            catalog: Catalog::builtin(),
            jobs: Mutex::new(HashMap::new()),
            hardware: OnceLock::new(),
        }
    }

    /// An installed package, for the OCR pipeline.
    #[allow(dead_code, reason = "the OCR commands (M4 pipeline) are its first users")]
    pub fn installed(&self, id: &str) -> Option<InstalledPackage> {
        self.store.as_ref()?.installed(id)
    }

    fn running(&self, id: &str) -> bool {
        lock(&self.jobs).contains_key(id)
    }
}

/// One package as the settings panel lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageWire {
    #[serde(flatten)]
    status: PackageStatus,
    /// OCR cannot run without it.
    required: bool,
    /// A download of it is running.
    downloading: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsOverview {
    packages: Vec<PackageWire>,
    hardware: Hardware,
    /// False when there is nowhere to install (no app data directory).
    available: bool,
}

/// Where a download is, sent as [`PROGRESS_EVENT`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobUpdate {
    Progress {
        phase: Phase,
        received: u64,
        total: u64,
    },
    Installed,
    /// Stopped by the user; `received` bytes are kept to resume.
    Cancelled,
    Failed {
        code: &'static str,
        /// For `model_disk_full`: bytes needed and free.
        needed: Option<u64>,
        available: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    id: String,
    #[serde(flatten)]
    update: JobUpdate,
}

fn unavailable() -> CommandError {
    CommandError::new("model_unavailable", "no app data directory to install models into")
}

fn unknown(id: &str) -> CommandError {
    CommandError::new("model_unknown", format!("no OCR model called {id:?}"))
}

fn model_error(err: ModelError) -> CommandError {
    CommandError::new(err.code(), err.to_string())
}

/// The catalog for this machine, what is installed, and the hardware.
#[tauri::command]
pub async fn ocr_models(app: AppHandle) -> CommandResult<ModelsOverview> {
    blocking(move || {
        let state = app.state::<ModelsState>();
        let hardware = state.hardware.get_or_init(Hardware::detect).clone();
        let required: Vec<String> = state.catalog.required().iter().map(|p| p.id.clone()).collect();
        let packages = match &state.store {
            Some(store) => store
                .status(&state.catalog)
                .into_iter()
                .map(|status| PackageWire {
                    required: required.contains(&status.id),
                    downloading: state.running(&status.id),
                    status,
                })
                .collect(),
            None => Vec::new(),
        };
        Ok(ModelsOverview { packages, hardware, available: state.store.is_some() })
    })
    .await
}

/// Starts downloading and installing a package. Returns at once; the outcome
/// arrives as [`PROGRESS_EVENT`]s. `model_busy` if it is already downloading.
#[tauri::command]
pub fn download_ocr_model(app: AppHandle, state: State<'_, ModelsState>, id: String) -> CommandResult<()> {
    let package = state.catalog.get(&id).cloned().ok_or_else(|| unknown(&id))?;
    if state.store.is_none() {
        return Err(unavailable());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = lock(&state.jobs);
        if jobs.contains_key(&id) {
            return Err(CommandError::new("model_busy", format!("{id} is already downloading")));
        }
        jobs.insert(id.clone(), Arc::clone(&cancel));
    }
    let job = id.clone();
    std::thread::Builder::new()
        .name(format!("model download {id}"))
        .spawn(move || {
            let id = job;
            let state = app.state::<ModelsState>();
            let send = |update: JobUpdate| {
                let _ = app.emit(PROGRESS_EVENT, ProgressEvent { id: id.clone(), update });
            };
            let mut last: Option<(Instant, Phase)> = None;
            let result = match &state.store {
                Some(store) => store.install(&package, &Downloader::default(), &cancel, &mut |progress| {
                    let due = last.is_none_or(|(at, phase)| {
                        phase != progress.phase || at.elapsed() >= PROGRESS_INTERVAL
                    });
                    if due || progress.received == progress.total {
                        last = Some((Instant::now(), progress.phase));
                        send(JobUpdate::Progress {
                            phase: progress.phase,
                            received: progress.received,
                            total: progress.total,
                        });
                    }
                }),
                None => Err(ModelError::Io(std::io::Error::other("no store"))),
            };
            lock(&state.jobs).remove(&id);
            send(match result {
                Ok(_) => JobUpdate::Installed,
                Err(ModelError::Cancelled) => JobUpdate::Cancelled,
                Err(err) => {
                    eprintln!("safha: installing the OCR model {id} failed: {err}");
                    let (needed, available) = match err {
                        ModelError::DiskFull { needed, available } if needed > 0 => {
                            (Some(needed), Some(available))
                        }
                        _ => (None, None),
                    };
                    JobUpdate::Failed { code: err.code(), needed, available }
                }
            });
        })
        .map_err(|err| {
            lock(&state.jobs).remove(&id);
            CommandError::new("model_install_failed", err.to_string())
        })?;
    Ok(())
}

/// Stops a running download; what was received is kept to resume.
#[tauri::command]
pub fn cancel_ocr_model_download(state: State<'_, ModelsState>, id: String) -> CommandResult<()> {
    if let Some(cancel) = lock(&state.jobs).get(&id) {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Removes a package and any partial download of it. `model_busy` while it is
/// downloading (cancel first).
#[tauri::command]
pub async fn delete_ocr_model(app: AppHandle, id: String) -> CommandResult<()> {
    blocking(move || {
        let state = app.state::<ModelsState>();
        let package = state.catalog.get(&id).ok_or_else(|| unknown(&id))?;
        let store = state.store.as_ref().ok_or_else(unavailable)?;
        if state.running(&id) {
            return Err(CommandError::new("model_busy", format!("{id} is downloading")));
        }
        store.delete(package).map_err(model_error)
    })
    .await
}

/// Where models are installed: `SAFHA_MODELS_DIR` (development), else
/// `ocr-models` in the state directory this copy of the app holds (ADR 0046),
/// so two copies never download into the same folder. `None` when this copy
/// holds no state directory.
pub fn models_root(state_dir: Option<&Path>) -> Option<PathBuf> {
    std::env::var_os("SAFHA_MODELS_DIR")
        .map(PathBuf::from)
        .or_else(|| state_dir.map(|dir| dir.join("ocr-models")))
}

#[cfg(test)]
mod tests {
    use safha_core::models::PackageState;

    use super::*;

    #[test]
    fn events_and_listings_have_the_ui_shape() {
        let event = ProgressEvent {
            id: "nuqta-4b".into(),
            update: JobUpdate::Progress { phase: Phase::Downloading, received: 5, total: 10 },
        };
        assert_eq!(
            serde_json::to_value(&event).expect("json"),
            serde_json::json!({"id": "nuqta-4b", "kind": "progress", "phase": "downloading", "received": 5, "total": 10})
        );
        let failed = ProgressEvent {
            id: "x".into(),
            update: JobUpdate::Failed { code: "model_disk_full", needed: Some(2), available: Some(1) },
        };
        assert_eq!(
            serde_json::to_value(&failed).expect("json"),
            serde_json::json!({"id": "x", "kind": "failed", "code": "model_disk_full", "needed": 2, "available": 1})
        );

        let catalog = Catalog::builtin();
        let store = ModelStore::new(std::env::temp_dir().join("safha-shell-models-none"));
        let status = store.status(&catalog).remove(0);
        assert_eq!(status.state, PackageState::NotInstalled);
        let wire = PackageWire { status, required: true, downloading: false };
        let value = serde_json::to_value(&wire).expect("json");
        assert_eq!(value["id"], "nuqta-4b");
        assert_eq!(value["role"], "primary");
        assert_eq!(value["downloadSize"], 5_511_442_432u64);
        assert_eq!(value["state"], serde_json::json!({"kind": "notInstalled"}));
        assert_eq!(value["required"], true);
        assert!(value.get("files").is_none(), "no URLs or paths reach the UI");
    }

    #[test]
    fn jobs_are_tracked_by_id() {
        let state = ModelsState::new(None);
        assert!(!state.running("nuqta-4b"));
        lock(&state.jobs).insert("nuqta-4b".into(), Arc::new(AtomicBool::new(false)));
        assert!(state.running("nuqta-4b"));
        assert!(state.installed("nuqta-4b").is_none());
    }
}
