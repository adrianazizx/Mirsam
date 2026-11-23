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
use safha_core::fonts::FontFamily;
use safha_core::models::{ModelStore, PRIMARY, RUNTIME};
use safha_core::ocr::convert::{PageConversion, paragraphs, sample_colors};
use safha_core::ocr::jobs::{
    JobEngines, JobHost, JobId, JobKind, JobOptions, JobRunner, JobStatus, Language, LocalEngines, MAX_SIDE,
    RenderedPage, ReviewPage,
};
use safha_core::ocr::{
    Action, LayerQuality, LayerState, OcrImage, PageCheck, PageLayer, ReviewFilter, TextInfo,
};
use safha_core::{Document, TileRequest};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, http};

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

    fn states(&self, path: &Path) -> Option<Vec<LayerState>> {
        self.document(path)?.1.text_layer_states().ok()
    }

    fn render_area(&self, path: &Path, page: usize, rect: [f32; 4], scale: f32) -> Result<RenderedPage, String> {
        let (_, document) = self.document(path).ok_or("the document is not open")?;
        let bitmap = render_rect(&document, page, rect, scale).map_err(|err| err.to_string())?;
        let placement = document.ocr_placement(page).map_err(|err| err.to_string())?;
        Ok(RenderedPage { image: OcrImage::from_bitmap(&bitmap), scale, placement })
    }

    fn text_info(&self, path: &Path, page: usize) -> Result<TextInfo, String> {
        let (_, document) = self.document(path).ok_or("the document is not open")?;
        document.text_info(page).map_err(|err| err.to_string())
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
    replace: Option<bool>,
) -> CommandResult<JobWire> {
    let options = JobOptions { kind: JobKind::Recognize, replace: replace.unwrap_or(false) };
    submit(&state, &ocr, document_id, pages, options)
}

/// Reads an area of a page again (ADR 0038): `rect` in displayed points
/// (left, top, right, bottom), in a language, or as numbers only.
#[tauri::command]
pub async fn start_ocr_region(
    state: State<'_, AppState>,
    ocr: State<'_, OcrState>,
    document_id: u32,
    page: usize,
    rect: [f32; 4],
    language: Option<Language>,
    numbers: Option<bool>,
) -> CommandResult<JobWire> {
    let options = JobOptions {
        kind: JobKind::Region { rect, language: language.unwrap_or_default(), numbers: numbers.unwrap_or(false) },
        replace: false,
    };
    submit(&state, &ocr, document_id, Some(vec![page]), options)
}

/// Checks the existing text layers of `pages` (every page when absent) against
/// a quick draft (ADR 0038). Needs no models.
#[tauri::command]
pub async fn check_text_layers(
    state: State<'_, AppState>,
    ocr: State<'_, OcrState>,
    document_id: u32,
    pages: Option<Vec<usize>>,
) -> CommandResult<JobWire> {
    let runner = ocr.runner()?;
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let path = canonical(&state, document_id)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let page_count = document.page_count();
    let pages = pages.unwrap_or_else(|| (0..page_count).collect());
    let options = JobOptions { kind: JobKind::Check, replace: false };
    let job = runner
        .submit(&path, &name, page_count, &pages, options)
        .map_err(|err| CommandError::new("invalid_edit", err))?;
    Ok(wire(&state, job))
}

/// The latest verdict on each checked page of a document (ADR 0038).
#[tauri::command]
pub async fn ocr_layer_checks(
    state: State<'_, AppState>,
    ocr: State<'_, OcrState>,
    document_id: u32,
) -> CommandResult<Vec<PageCheck>> {
    let Some(runner) = ocr.runner.as_ref() else { return Ok(Vec::new()) };
    let path = canonical(&state, document_id)?;
    runner.checks(&path).map_err(|err| CommandError::new("io_error", err))
}

/// Queues a job that reads with the models: they must be installed.
fn submit(
    state: &AppState,
    ocr: &OcrState,
    document_id: u32,
    pages: Option<Vec<usize>>,
    options: JobOptions,
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
    let path = canonical(state, document_id)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let page_count = document.page_count();
    let pages = pages.unwrap_or_else(|| (0..page_count).collect());
    let job = runner
        .submit(&path, &name, page_count, &pages, options)
        .map_err(|err| CommandError::new("invalid_edit", err))?;
    Ok(wire(state, job))
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

/// The pages-needing-review list of a document (spec §5.8), from its OCR jobs,
/// as the document shows them (ADR 0037); every number too when `numbers`.
#[tauri::command]
pub async fn ocr_review(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    numbers: Option<bool>,
) -> CommandResult<Vec<ReviewPage>> {
    let path = canonical(&state, document_id)?;
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let filter = ReviewFilter { numbers: numbers.unwrap_or(false) };
    blocking(move || {
        let ocr = app.state::<OcrState>();
        let states = document.text_layer_states()?;
        ocr.runner()?.review(&path, &states, filter).map_err(|err| CommandError::new("io_error", err))
    })
    .await
}

/// What a review decision did: the list after it, and the edit when the
/// word's text went into the document.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideOutcome {
    review: Vec<ReviewPage>,
    outcome: Option<EditOutcome>,
}

/// A review decision on a word (`word`: its index among the page's words) or
/// on the page (`word` absent): accept, check, or a correction, which goes
/// into the text layer as an undoable edit (ADR 0037).
#[tauri::command]
pub async fn ocr_decide(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    word: Option<usize>,
    action: Action,
    numbers: Option<bool>,
) -> CommandResult<DecideOutcome> {
    let path = canonical(&state, document_id)?;
    let filter = ReviewFilter { numbers: numbers.unwrap_or(false) };
    blocking(move || {
        let state = app.state::<AppState>();
        let ocr = app.state::<OcrState>();
        let runner = ocr.runner()?;
        let history =
            state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
        let mut history = lock(&history);
        let document =
            state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
        if matches!(action, Action::Edit { .. }) && document.is_signed() {
            return Err(CommandError::new("ocr_signed_document", "the document is signed"));
        }
        let states = document.text_layer_states()?;
        let decided = runner
            .decide(&path, &states, page, word, action)
            .map_err(|code| CommandError::new(review_code(&code), code))?;
        let (outcome, states) = match decided.layer {
            Some(layer) => {
                let (revision, update) = document.with_text_layers(&[layer])?;
                history.push_content(update);
                let states = revision.text_layer_states()?;
                let page_count = revision.page_count();
                state.replace_revision(document_id, revision);
                (Some(EditOutcome::new(page_count, &history, Change::Content)), states)
            }
            None => (None, states),
        };
        drop(history);
        let review = runner.review(&path, &states, filter).map_err(|err| CommandError::new("io_error", err))?;
        Ok(DecideOutcome { review, outcome })
    })
    .await
}

/// A review refusal's stable code (`safha_core::ocr::review::DecideError`);
/// anything else is the state store failing.
fn review_code(code: &str) -> &'static str {
    match code {
        "ocr_review_no_word" => "ocr_review_no_word",
        "ocr_review_too_long" => "ocr_review_too_long",
        "ocr_review_converted" => "ocr_review_converted",
        "ocr_review_stale" => "ocr_review_stale",
        _ => "io_error",
    }
}

/// Converts recognised pages into page text (ADR 0039): paragraphs the Edit
/// tool opens, the scan kept behind (`keep_scan`) or removed. A page with a
/// word still needing a decision is refused (`ocr_convert_unreviewed`, its
/// page numbers in the detail), as is one with no result of ours
/// (`ocr_convert_unrecognized`).
#[tauri::command]
pub async fn ocr_convert(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    pages: Option<Vec<usize>>,
    keep_scan: bool,
    family: Option<FontFamily>,
) -> CommandResult<EditOutcome> {
    let path = canonical(&state, document_id)?;
    blocking(move || {
        let state = app.state::<AppState>();
        let ocr = app.state::<OcrState>();
        let runner = ocr.runner()?;
        let history =
            state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
        let mut history = lock(&history);
        let document =
            state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
        if document.is_signed() {
            return Err(CommandError::new("ocr_signed_document", "the document is signed"));
        }
        let states = document.text_layer_states()?;
        // Every page with a recognised layer, when no pages are named.
        let strict = pages.is_some();
        let pages = pages.unwrap_or_else(|| {
            (0..document.page_count())
                .filter(|&p| states.get(p).is_some_and(|s| s.quality == Some(LayerQuality::Final)))
                .collect()
        });
        let mut conversions = Vec::new();
        let (mut unreviewed, mut unrecognized) = (Vec::new(), Vec::new());
        for &page in &pages {
            let view = runner.page_view(&path, &states, page).map_err(|err| CommandError::new("io_error", err))?;
            let doc = states.get(page).copied().unwrap_or_default();
            let Some(view) = view.filter(|v| !v.converted && !v.stale && doc.quality == Some(LayerQuality::Final))
            else {
                unrecognized.push(page + 1);
                continue;
            };
            if view.view.needs_decision() {
                unreviewed.push(page + 1);
                continue;
            }
            conversions.push(conversion(&document, page, view.view.layer, doc.digest, keep_scan, family)?);
        }
        let list = |pages: &[usize]| pages.iter().map(usize::to_string).collect::<Vec<_>>().join(",");
        if !unreviewed.is_empty() {
            return Err(CommandError::new("ocr_convert_unreviewed", list(&unreviewed)));
        }
        if (strict && !unrecognized.is_empty()) || conversions.is_empty() {
            return Err(CommandError::new("ocr_convert_unrecognized", list(&unrecognized)));
        }
        let (revision, update) = document.with_conversion(&conversions)?;
        history.push_content(update);
        let page_count = revision.page_count();
        state.replace_revision(document_id, revision);
        Ok(EditOutcome::new(page_count, &history, Change::Content))
    })
    .await
}

/// Page `page`'s recognised `layer` ready to convert, with its colours
/// sampled from the page as rendered at 100 dpi.
fn conversion(
    document: &Document,
    page: usize,
    layer: PageLayer,
    digest: Option<u32>,
    keep_scan: bool,
    family: Option<FontFamily>,
) -> CommandResult<PageConversion> {
    let scale = 100.0 / 72.0;
    let image = OcrImage::from_bitmap(&document.render_page(page, scale)?);
    let found = paragraphs(&layer);
    let area = found.iter().fold([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY], |a, p| {
        [a[0].min(p.bounds[0]), a[1].min(p.bounds[1]), a[2].max(p.bounds[2]), a[3].max(p.bounds[3])]
    });
    let colors = if area.iter().all(|v| v.is_finite()) { sample_colors(&image, scale, area) } else { Default::default() };
    Ok(PageConversion { page, from: digest.unwrap_or_else(|| layer.digest()), layer, keep_scan, family, colors })
}

/// `rect` of page `page` (displayed points: left, top, right, bottom) rendered
/// at `scale` pixels per point.
pub(crate) fn render_rect(
    document: &Document,
    page: usize,
    rect: [f32; 4],
    scale: f32,
) -> safha_core::Result<safha_core::Bitmap> {
    let size = document.page_size(page)?;
    let px = |v: f32, max: f32| (v.clamp(0.0, max) * scale).round() as u32;
    let (x0, y0) = (px(rect[0], size.width), px(rect[1], size.height));
    let (x1, y1) = (px(rect[2], size.width), px(rect[3], size.height));
    document.render_tile(
        page,
        TileRequest { scale, x: x0, y: y0, width: x1.saturating_sub(x0).max(1), height: y1.saturating_sub(y0).max(1) },
    )
}

/// `safha://localhost/ocr-crop?doc=&page=&x0=&y0=&x1=&y1=&h=`: an area of a
/// page (displayed points) as a BMP the review shows beside a word, `h`
/// pixels tall at most (ADR 0037).
pub fn crop_response<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    uri: &http::Uri,
) -> Result<Vec<u8>, (http::StatusCode, &'static str)> {
    let query: std::collections::HashMap<&str, &str> =
        uri.query().unwrap_or_default().split('&').filter_map(|pair| pair.split_once('=')).collect();
    let number = |name: &str| query.get(name).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite());
    let bad = (http::StatusCode::BAD_REQUEST, "invalid_render_request");
    let doc = query.get("doc").and_then(|v| v.parse::<u32>().ok()).ok_or(bad)?;
    let page = query.get("page").and_then(|v| v.parse::<usize>().ok()).ok_or(bad)?;
    let rect = [number("x0").ok_or(bad)?, number("y0").ok_or(bad)?, number("x1").ok_or(bad)?, number("y1").ok_or(bad)?];
    let height = number("h").unwrap_or(120.0).clamp(16.0, 600.0);
    if rect[2] <= rect[0] || rect[3] <= rect[1] {
        return Err(bad);
    }
    let document = app
        .state::<AppState>()
        .document(doc)
        .ok_or((http::StatusCode::NOT_FOUND, "unknown_document"))?;
    // Tall enough to read; never more than 12 pixels a point, nor 4,000 pixels wide.
    let scale = (height / (rect[3] - rect[1])).min(12.0).min(4000.0 / (rect[2] - rect[0]));
    let bitmap = render_rect(&document, page, rect, scale)
        .map_err(|_| (http::StatusCode::UNPROCESSABLE_ENTITY, "engine_error"))?;
    Ok(bitmap.encode_bmp())
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
            options: JobOptions::default(),
        };
        let json = serde_json::to_value(JobWire { job, document_ids: vec![4] }).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "id": 7, "name": "secret.pdf", "pages": 3, "state": "running", "error": null,
                "drafted": 3, "finished": 1, "current": [1, "quality"], "documentIds": [4],
                "options": { "kind": { "kind": "recognize" }, "replace": false }
            })
        );
    }
}
