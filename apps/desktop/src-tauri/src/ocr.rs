//! Background OCR (ADR 0035): thin commands over `safha_core::ocr::jobs`.
//!
//! The job runner lives in the engine; this module is its [`JobHost`]: it finds
//! a job's document among the open ones (by file), renders its pages, writes
//! each batch of text layers as an edit (undoable, autosaved, the search index
//! rebuilt, like any other), and tells every window how jobs move on through
//! [`JOB_EVENT`]. A document that received a layer also gets
//! [`APPLIED_EVENT`] in its own window, with the edit's outcome. The UI names
//! documents by id and jobs by number, never by path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use safha_core::edit::Change;
use safha_core::models::{ModelStore, PRIMARY, RUNTIME};
use safha_core::ocr::jobs::{
    JobEngines, JobHost, JobId, JobRunner, JobStatus, LocalEngines, MAX_SIDE, RenderedPage, ReviewPage,
};
use safha_core::ocr::{LayerQuality, OcrImage, PageLayer};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::editing::EditOutcome;
use crate::error::{CommandError, CommandResult};
use crate::navigation::blocking;
use crate::state::{AppState, lock};

/// Sent to every window when a job moves on.
pub const JOB_EVENT: &str = "ocr-job";
/// Sent to a document's window when OCR text was written into it.
pub const APPLIED_EVENT: &str = "ocr-applied";

/// A job as the UI sees it: its status, and the open documents it belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobWire {
    #[serde(flatten)]
    job: JobStatus,
    document_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Applied {
    document_id: u32,
    outcome: EditOutcome,
    /// The pages written, and whether each is now the draft or the final text.
    pages: Vec<(usize, LayerQuality)>,
}

/// The OCR job runner, when the app has a state directory to keep jobs in.
pub struct OcrState {
    runner: Option<JobRunner>,
    models: Option<ModelStore>,
}

impl OcrState {
    /// Starts the runner over the state store in `state_dir` (a connection of
    /// its own to the same file) and the models in `models_root`.
    pub fn start(app: AppHandle, state_dir: Option<&Path>, models_root: Option<PathBuf>) -> Self {
        let models = models_root.map(ModelStore::new);
        let (Some(dir), Some(store)) = (state_dir, models.clone()) else {
            return Self { runner: None, models };
        };
        let book = match safha_core::store::Store::open(&dir.join("state.sqlite3")) {
            Ok(book) => Arc::new(book),
            Err(err) => {
                eprintln!("safha: OCR jobs cannot be kept: {err}");
                return Self { runner: None, models };
            }
        };
        let logs = dir.join("ocr-logs");
        let _ = std::fs::create_dir_all(&logs);
        let engines: Box<dyn JobEngines> = Box::new(LocalEngines::new(store, helpers_dir(dir), Some(logs)));
        match JobRunner::start(book, Arc::new(Host { app }), engines) {
            Ok(runner) => Self { runner: Some(runner), models },
            Err(err) => {
                eprintln!("safha: the OCR job runner did not start: {err}");
                Self { runner: None, models }
            }
        }
    }

    fn runner(&self) -> CommandResult<&JobRunner> {
        self.runner.as_ref().ok_or_else(|| CommandError::new("ocr_unavailable", "no state directory"))
    }

    /// A document opened or closed.
    pub fn wake(&self) {
        if let Some(runner) = &self.runner {
            runner.wake();
        }
    }

    /// The document from `from` was saved as `to`.
    pub fn moved(&self, from: &Path, to: &Path, name: &str) {
        if let Some(runner) = &self.runner {
            runner.moved(from, to, name);
        }
    }

    /// The app is quitting: the servers stop now (they hold gigabytes), and the
    /// running job resumes at the next start.
    pub fn shutdown(&self) {
        if let Some(runner) = &self.runner {
            runner.shutdown();
        }
    }
}

/// Where helper programs (the Vision helper) are built: the platform's cache.
fn helpers_dir(state_dir: &Path) -> PathBuf {
    dirs::cache_dir()
        .map(|cache| cache.join("app.mirsam.desktop").join("helpers"))
        .unwrap_or_else(|| state_dir.join("helpers"))
}

struct Host {
    app: AppHandle,
}

impl Host {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    fn document(&self, path: &Path) -> Option<(u32, Arc<safha_core::Document>)> {
        let state = self.state();
        let id = state.documents_of(path).into_iter().next()?;
        Some((id, state.document(id)?))
    }
}

impl JobHost for Host {
    fn page_count(&self, path: &Path) -> Option<usize> {
        self.document(path).map(|(_, document)| document.page_count())
    }

    fn render(&self, path: &Path, page: usize, scale: f32) -> Result<RenderedPage, String> {
        let (_, document) = self.document(path).ok_or("the document is not open")?;
        let size = document.page_size(page).map_err(|err| err.to_string())?;
        // A large-format page reads at less than the wanted resolution.
        let scale = scale.min(MAX_SIDE / size.width.max(size.height).max(1.0));
        let bitmap = document.render_page(page, scale).map_err(|err| err.to_string())?;
        let placement = document.ocr_placement(page).map_err(|err| err.to_string())?;
        Ok(RenderedPage { image: OcrImage::from_bitmap(&bitmap), scale, placement })
    }

    fn layers(&self, path: &Path) -> Option<Vec<Option<LayerQuality>>> {
        self.document(path)?.1.text_layer_qualities().ok()
    }

    fn apply(&self, path: &Path, layers: &[PageLayer]) -> Result<bool, String> {
        let state = self.state();
        let Some(id) = state.documents_of(path).into_iter().next() else { return Ok(false) };
        let Some(history) = state.history(id) else { return Ok(false) };
        let mut history = lock(&history);
        let Some(document) = state.document(id) else { return Ok(false) };
        let (revision, update) = document.with_text_layers(layers).map_err(|err| err.to_string())?;
        history.push_content(update);
        let page_count = revision.page_count();
        state.replace_revision(id, revision);
        let outcome = EditOutcome::new(page_count, &history, Change::Content);
        drop(history);
        let applied =
            Applied { document_id: id, outcome, pages: layers.iter().map(|l| (l.page, l.quality)).collect() };
        if let Some(window) = state.window_of(id)
            && let Err(err) = self.app.emit_to(window.as_str(), APPLIED_EVENT, applied)
        {
            eprintln!("safha: could not report OCR text: {err}");
        }
        Ok(true)
    }

    fn changed(&self, status: &JobStatus) {
        let wire = JobWire { document_ids: self.state().documents_of(&status.path), job: status.clone() };
        if let Err(err) = self.app.emit(JOB_EVENT, wire) {
            eprintln!("safha: could not report an OCR job: {err}");
        }
    }
}

fn wire(state: &AppState, job: JobStatus) -> JobWire {
    JobWire { document_ids: state.documents_of(&job.path), job }
}

fn canonical(state: &AppState, document_id: u32) -> CommandResult<PathBuf> {
    let path = state.path_of(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    Ok(safha_core::store::canonical_path(&path))
}

/// Queues OCR of `pages` (zero-based; every page when absent) of a document.
#[tauri::command]
pub async fn start_ocr(
    state: State<'_, AppState>,
    ocr: State<'_, OcrState>,
    document_id: u32,
    pages: Option<Vec<usize>>,
) -> CommandResult<JobWire> {
    let runner = ocr.runner()?;
    let models =
        ocr.models.as_ref().ok_or_else(|| CommandError::new("model_unavailable", "no models folder"))?;
    for id in [PRIMARY, RUNTIME] {
        if models.installed(id).is_none() {
            return Err(CommandError::new("ocr_model_missing", format!("{id} is not installed")));
        }
    }
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    // A text layer after a signature would show as a change to the signed document.
    if document.is_signed() {
        return Err(CommandError::new("ocr_signed_document", "the document is signed"));
    }
    let path = canonical(&state, document_id)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let page_count = document.page_count();
    let pages = pages.unwrap_or_else(|| (0..page_count).collect());
    let job = runner
        .submit(&path, &name, page_count, &pages)
        .map_err(|err| CommandError::new("invalid_edit", err))?;
    Ok(wire(&state, job))
}

/// The OCR jobs of a document, oldest first.
#[tauri::command]
pub async fn ocr_jobs(
    state: State<'_, AppState>,
    ocr: State<'_, OcrState>,
    document_id: u32,
) -> CommandResult<Vec<JobWire>> {
    let Some(runner) = ocr.runner.as_ref() else { return Ok(Vec::new()) };
    let path = canonical(&state, document_id)?;
    Ok(runner.jobs(&path).into_iter().map(|job| wire(&state, job)).collect())
}

/// What a job control does.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobAction {
    Pause,
    Resume,
    Cancel,
}

#[tauri::command]
pub async fn control_ocr_job(
    state: State<'_, AppState>,
    ocr: State<'_, OcrState>,
    job_id: JobId,
    action: JobAction,
) -> CommandResult<JobWire> {
    let runner = ocr.runner()?;
    let job = match action {
        JobAction::Pause => runner.pause(job_id),
        JobAction::Resume => runner.resume(job_id),
        JobAction::Cancel => runner.cancel(job_id),
    };
    let job = job.ok_or_else(|| CommandError::new("ocr_unknown_job", format!("no job {job_id}")))?;
    Ok(wire(&state, job))
}

/// Forgets a job that is not running, and its part of the review list.
#[tauri::command]
pub async fn remove_ocr_job(ocr: State<'_, OcrState>, job_id: JobId) -> CommandResult<()> {
    ocr.runner()?.remove(job_id).map_err(|err| CommandError::new("ocr_job_running", err))
}

/// The pages-needing-review list of a document (spec §5.8), from its OCR jobs.
#[tauri::command]
pub async fn ocr_review(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Vec<ReviewPage>> {
    let path = canonical(&state, document_id)?;
    blocking(move || {
        let ocr = app.state::<OcrState>();
        ocr.runner()?.review(&path).map_err(|err| CommandError::new("io_error", err))
    })
    .await
}

#[cfg(test)]
mod tests {
    use safha_core::ocr::jobs::{JobState, Pass};

    use super::*;

    #[test]
    fn jobs_reach_the_ui_without_their_path() {
        let job = JobStatus {
            id: 7,
            path: PathBuf::from("/private/secret.pdf"),
            name: "secret.pdf".into(),
            pages: 3,
            state: JobState::Running,
            error: None,
            drafted: 3,
            finished: 1,
            current: Some((1, Pass::Quality)),
        };
        let json = serde_json::to_value(JobWire { job, document_ids: vec![4] }).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "id": 7, "name": "secret.pdf", "pages": 3, "state": "running", "error": null,
                "drafted": 3, "finished": 1, "current": [1, "quality"], "documentIds": [4]
            })
        );
    }
}
