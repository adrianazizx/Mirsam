//! Background OCR jobs (spec §8: "OCR must never block the app"; ADR 0035).
//!
//! A job is a document (by its file) and a list of pages. One worker thread runs
//! the jobs one after another, oldest first, while their documents are open:
//!
//! 1. **Draft pass** over every page: the platform's OCR (Apple Vision; Tesseract
//!    elsewhere) at 200 dpi, written as a `Draft` text layer in batches, so the
//!    document is searchable within moments.
//! 2. **Quality pass**, page by page: the page at 300 dpi through the quality
//!    pipeline ([`QualityReader`]: Nuqta 4B and 0.8B, re-reading and reconciling),
//!    word boxes aligned to the draft reader's lines ([`word_boxes`]), written as
//!    the `Final` layer in the draft's place.
//!
//! Every page result is kept in the app's state store ([`JobBook`]) before it is
//! written into the document, with its words' confidence and status for the
//! pages-needing-review list. A job survives a restart: on the next start it is
//! queued again, the results the document lacks (unsaved when the app quit) are
//! written back without reading the page again, and it carries on with the pages
//! left. The models' servers are shared by all jobs and stopped after a while
//! with nothing to do. Cancel stops at once (the servers are stopped mid-page);
//! pause stops after the page being read.
//!
//! The worker knows documents only through [`JobHost`] (the shell: which files
//! are open, rendering, writing a layer as an edit) and engines only through
//! [`JobEngines`], so tests run it with scripted ones. Files are named by their
//! canonical path ([`canonical_path`]), as the state store keys them.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::boxes::{visual_order, word_boxes};
use super::check::{PageCheck, TextInfo, check_page};
use super::draft::{
    DraftPage, DraftReader, PixelBox, TesseractReader, VisionReader, first_strong_rtl, vision_helper,
};
use super::layer::{Clear, LayerLine, LayerQuality, LayerState, LayerWord, PageLayer};
use super::llama::{Device, NuqtaEngine, NuqtaModel, ServerConfig};
use super::pipeline::{PageResult, QualityReader, Settings};
use super::reconcile::WordStatus;
pub use super::region::Language;
use super::region::{Constraint, constrain, merge_region, region_scale, shift_page};
use super::review::{
    Action, DecideError, Decision, PageView, ReviewFilter, WordReview, decide, resolve, review_page,
    sort_review,
};
pub use super::review::{ReviewPage, ReviewWord};
use super::{OcrEngine, OcrError, OcrImage};
use crate::models::{ENSEMBLE, Hardware, ModelStore, PRIMARY, RUNTIME};
use crate::store::canonical_path;

pub type JobId = i64;

/// Pixels per point for the quality pass: 300 dpi (spec §8, pipeline step 1).
pub const QUALITY_SCALE: f32 = 300.0 / 72.0;
/// Pixels per point for the draft pass: 200 dpi, plenty for the platform's OCR.
pub const DRAFT_SCALE: f32 = 200.0 / 72.0;
/// A page's longest side in pixels at most (a large-format page reads at less than 300 dpi).
pub const MAX_SIDE: f32 = 7000.0;
/// Servers with nothing to do for this long are stopped (the 4B holds 11 GiB).
pub const IDLE_STOP: Duration = Duration::from_secs(300);
/// Draft layers are written in batches: this many pages, or this often.
const DRAFT_BATCH: usize = 8;
const DRAFT_FLUSH: Duration = Duration::from_millis(1500);

/// Where a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    /// Waiting its turn, or for its document to be opened.
    Queued,
    Running,
    Paused,
    Done,
    Cancelled,
    /// Stopped by an error; `error` names it.
    Failed,
}

impl JobState {
    pub fn name(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Done => "done",
            JobState::Cancelled => "cancelled",
            JobState::Failed => "failed",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "queued" => JobState::Queued,
            "running" => JobState::Running,
            "paused" => JobState::Paused,
            "done" => JobState::Done,
            "cancelled" => JobState::Cancelled,
            "failed" => JobState::Failed,
            _ => return None,
        })
    }

    fn active(self) -> bool {
        matches!(self, JobState::Queued | JobState::Running)
    }
}

/// What a job does (ADR 0038).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobKind {
    /// Draft, then the quality pass, over its pages.
    #[default]
    Recognize,
    /// One area of one page read again with the quality pipeline; the result
    /// replaces that area's text. `rect`: displayed page points (left, top,
    /// right, bottom).
    Region {
        rect: [f32; 4],
        #[serde(default)]
        language: Language,
        /// Only numbers are expected there.
        #[serde(default)]
        numbers: bool,
    },
    /// The pages' existing text layers compared with a quick draft (ADR 0038).
    Check,
}

/// A job's kind and settings, as the state store keeps them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOptions {
    #[serde(default)]
    pub kind: JobKind,
    /// The layers written take the place of another tool's invisible text
    /// (a poor text layer), which is removed by byte range.
    #[serde(default)]
    pub replace: bool,
}

/// A job as the state store keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct JobRecord {
    pub id: JobId,
    /// The document's file (never shown to the UI).
    pub path: PathBuf,
    pub name: String,
    /// The document's page count when the job was made; a document that no
    /// longer has it is not resumed.
    pub page_count: usize,
    /// Zero-based, in the order they are read.
    pub pages: Vec<usize>,
    pub state: JobState,
    pub error: Option<String>,
    pub options: JobOptions,
}

/// A word of a quality result, kept for the review list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrWord {
    pub text: String,
    /// The primary engine's probability for the word (0–1).
    pub probability: f32,
    pub status: WordStatus,
    /// The word's box on the displayed page in points (left, top, right, bottom).
    pub bounds: Option<[f32; 4]>,
    /// The box came from widths, not from the guide (ADR 0031): it may be off.
    pub placed_by_width: bool,
    /// Where it is in the page's layer: line, and place in the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[u32; 2]>,
    /// What a person decided about it in review (ADR 0037).
    #[serde(default, skip_serializing_if = "WordReview::is_open")]
    pub review: WordReview,
}

/// One page's OCR result as kept: its text layer, and for the quality pass its words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOcr {
    pub page: usize,
    pub layer: PageLayer,
    #[serde(default)]
    pub words: Vec<OcrWord>,
    #[serde(default)]
    pub truncated: bool,
    /// The engines that read it.
    pub engine: String,
    pub seconds: f32,
    /// A person checked the page as a whole (ADR 0037).
    #[serde(default)]
    pub checked: bool,
    /// Decisions taken in review, each after its parent (ADR 0037).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<Decision>,
    /// The digest of `layer` as written.
    #[serde(default)]
    pub digest: Option<u32>,
}

impl PageOcr {
    pub fn quality(&self) -> LayerQuality {
        self.layer.quality
    }
}

/// Which pass is reading a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Pass {
    Draft,
    Quality,
    /// Comparing a page's existing text layer with a draft (ADR 0038).
    Check,
}

/// A job as the UI sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub id: JobId,
    #[serde(skip)]
    pub path: PathBuf,
    pub name: String,
    pub pages: usize,
    pub state: JobState,
    pub error: Option<String>,
    /// Pages with a draft or a final layer.
    pub drafted: usize,
    /// Pages with the quality result.
    pub finished: usize,
    /// The page being read now, and by which pass.
    pub current: Option<(usize, Pass)>,
    pub options: JobOptions,
}

/// A page as the document shows it, from its kept result (ADR 0037).
#[derive(Clone, Debug, PartialEq)]
pub struct PageState {
    pub view: PageView,
    /// Converted to page text (ADR 0039).
    pub converted: bool,
    /// The document's layer is not one the review knows.
    pub stale: bool,
}

/// What a review decision did.
#[derive(Clone, Debug, PartialEq)]
pub struct Decided {
    /// The page after it.
    pub view: PageView,
    /// The layer to write into the document (the word's text changed).
    pub layer: Option<PageLayer>,
}

/// The final results kept for the document from `path` (only page `only`'s
/// when given), by page, each with its job, oldest job first.
fn final_rows(
    shared: &Shared,
    path: &Path,
    only: Option<usize>,
) -> Result<BTreeMap<usize, Vec<(JobId, PageOcr)>>, String> {
    let path = canonical_path(path);
    let ids: Vec<JobId> = shared
        .jobs()
        .values()
        .filter(|j| j.record.path == path && j.record.options.kind != JobKind::Check)
        .map(|j| j.record.id)
        .collect();
    let mut rows: BTreeMap<usize, Vec<(JobId, PageOcr)>> = BTreeMap::new();
    for id in ids {
        let pages = match only {
            Some(page) => shared.book.page(id, page)?.into_iter().collect(),
            None => shared.book.pages(id)?,
        };
        for page in pages {
            if page.quality() == LayerQuality::Final {
                rows.entry(page.page).or_default().push((id, page));
            }
        }
    }
    Ok(rows)
}

/// The app's state store, for jobs and their page results.
pub trait JobBook: Send + Sync {
    fn create_job(
        &self,
        path: &Path,
        name: &str,
        page_count: usize,
        pages: &[usize],
        options: &JobOptions,
    ) -> Result<JobId, String>;
    fn jobs(&self) -> Result<Vec<JobRecord>, String>;
    fn set_job_state(&self, id: JobId, state: JobState, error: Option<&str>) -> Result<(), String>;
    fn set_job_path(&self, id: JobId, path: &Path, name: &str) -> Result<(), String>;
    fn save_page(&self, id: JobId, page: &PageOcr) -> Result<(), String>;
    fn pages(&self, id: JobId) -> Result<Vec<PageOcr>, String>;
    /// One page's result of a job.
    fn page(&self, id: JobId, page: usize) -> Result<Option<PageOcr>, String> {
        Ok(self.pages(id)?.into_iter().find(|p| p.page == page))
    }
    fn delete_job(&self, id: JobId) -> Result<(), String>;
    /// Keeps a check job's verdict on a page (ADR 0038).
    fn save_check(&self, id: JobId, check: &PageCheck) -> Result<(), String>;
    fn checks(&self, id: JobId) -> Result<Vec<PageCheck>, String>;
}

/// A page rendered for OCR.
pub struct RenderedPage {
    pub image: OcrImage,
    /// Pixels per point it was rendered at.
    pub scale: f32,
    /// [`PageLayer::placement`] for the page as it was rendered.
    pub placement: [f32; 6],
}

/// The open documents, as the worker needs them.
pub trait JobHost: Send + Sync {
    /// The document from `path` is open, with this many pages.
    fn page_count(&self, path: &Path) -> Option<usize>;
    /// Renders `page` of the open document at `scale` pixels per point.
    fn render(&self, path: &Path, page: usize, scale: f32) -> Result<RenderedPage, String>;
    /// The OCR layer each page of the open document has.
    fn layers(&self, path: &Path) -> Option<Vec<Option<LayerQuality>>>;
    /// What each page of the open document holds of our OCR, with digests (ADR 0037).
    fn states(&self, path: &Path) -> Option<Vec<LayerState>> {
        Some(
            self.layers(path)?
                .into_iter()
                .map(|quality| LayerState { quality, digest: None, converted: None })
                .collect(),
        )
    }
    /// Renders an area of `page` (`rect`: displayed points, left, top, right,
    /// bottom) at `scale`: the image is the area alone.
    fn render_area(
        &self,
        path: &Path,
        page: usize,
        rect: [f32; 4],
        scale: f32,
    ) -> Result<RenderedPage, String> {
        let full = self.render(path, page, scale)?;
        let s = full.scale;
        let px = |v: f32| (v * s).round().max(0.0) as u32;
        let image = full
            .image
            .crop(px(rect[0]), px(rect[1]), px(rect[2]), px(rect[3]))
            .map_err(|err| err.to_string())?;
        Ok(RenderedPage { image, scale: s, placement: full.placement })
    }
    /// The page's existing text, for checking it (ADR 0038).
    fn text_info(&self, _path: &Path, _page: usize) -> Result<TextInfo, String> {
        Err("not supported".into())
    }
    /// Writes `layers` into the open document as one edit. `Ok(false)`: it is not open.
    fn apply(&self, path: &Path, layers: &[PageLayer]) -> Result<bool, String>;
    /// A job moved on.
    fn changed(&self, status: &JobStatus);
}

/// A shared recogniser.
pub type SharedEngine = Arc<dyn OcrEngine + Send + Sync>;

/// The engines jobs use.
pub trait JobEngines: Send {
    /// The draft reader, started on first use; `None` where this machine has none.
    fn draft(&mut self) -> Option<&mut dyn DraftReader>;
    /// The quality pipeline's primary and partner, started on first use and kept
    /// for later jobs.
    fn quality(&mut self) -> Result<(SharedEngine, Option<SharedEngine>), OcrError>;
    /// Nothing to read since `since`: stop what has been idle long enough.
    fn idle(&mut self, since: Instant);
    /// Stops what is running now, from any thread (cancelling a page mid-read).
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interrupt {
    Pause,
    Cancel,
}

struct Job {
    record: JobRecord,
    drafted: HashSet<usize>,
    finished: HashSet<usize>,
    current: Option<(usize, Pass)>,
    interrupt: Option<Interrupt>,
}

impl Job {
    fn status(&self) -> JobStatus {
        JobStatus {
            id: self.record.id,
            path: self.record.path.clone(),
            name: self.record.name.clone(),
            pages: self.record.pages.len(),
            state: self.record.state,
            error: self.record.error.clone(),
            drafted: self.drafted.len(),
            finished: self.finished.len(),
            current: self.current,
            options: self.record.options.clone(),
        }
    }
}

struct Shared {
    book: Arc<dyn JobBook>,
    host: Arc<dyn JobHost>,
    stop_engines: Arc<dyn Fn() + Send + Sync>,
    jobs: Mutex<BTreeMap<JobId, Job>>,
    signal: Mutex<Signal>,
    wake: Condvar,
    /// Decisions on a page are read, changed and kept one at a time.
    deciding: Mutex<()>,
}

#[derive(Default)]
struct Signal {
    quit: bool,
    /// Something changed since the worker last looked.
    pending: bool,
}

impl Shared {
    fn jobs(&self) -> MutexGuard<'_, BTreeMap<JobId, Job>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_state(&self, id: JobId, state: JobState, error: Option<&str>) {
        let status = {
            let mut jobs = self.jobs();
            let Some(job) = jobs.get_mut(&id) else { return };
            job.record.state = state;
            job.record.error = error.map(str::to_owned);
            if !matches!(state, JobState::Running) {
                job.current = None;
            }
            job.status()
        };
        if let Err(err) = self.book.set_job_state(id, state, error) {
            eprintln!("safha: could not record an OCR job's state: {err}");
        }
        self.host.changed(&status);
    }

    fn notify(&self, id: JobId) {
        let status = self.jobs().get(&id).map(Job::status);
        if let Some(status) = status {
            self.host.changed(&status);
        }
    }

    fn interrupt(&self, id: JobId) -> Option<Interrupt> {
        self.jobs().get(&id).and_then(|job| job.interrupt)
    }

    fn signal(&self) -> MutexGuard<'_, Signal> {
        self.signal.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wake(&self) {
        self.signal().pending = true;
        self.wake.notify_all();
    }

    fn quitting(&self) -> bool {
        self.signal().quit
    }
}

/// The background job queue and its worker thread.
pub struct JobRunner {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl JobRunner {
    /// Loads the jobs in `book` (a job that was running when the app quit is
    /// queued again) and starts the worker.
    pub fn start(
        book: Arc<dyn JobBook>,
        host: Arc<dyn JobHost>,
        engines: Box<dyn JobEngines>,
    ) -> Result<Self, String> {
        let mut jobs = BTreeMap::new();
        for mut record in book.jobs()? {
            if record.state == JobState::Running {
                record.state = JobState::Queued;
            }
            let (drafted, finished) = if record.options.kind == JobKind::Check {
                let checked: HashSet<usize> = book.checks(record.id)?.iter().map(|c| c.page).collect();
                (checked.clone(), checked)
            } else {
                let pages = book.pages(record.id)?;
                (
                    pages.iter().map(|p| p.page).collect(),
                    pages.iter().filter(|p| p.quality() == LayerQuality::Final).map(|p| p.page).collect(),
                )
            };
            jobs.insert(record.id, Job { record, drafted, finished, current: None, interrupt: None });
        }
        let shared = Arc::new(Shared {
            book,
            host,
            stop_engines: engines.stopper(),
            jobs: Mutex::new(jobs),
            signal: Mutex::new(Signal::default()),
            wake: Condvar::new(),
            deciding: Mutex::new(()),
        });
        let worker = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("ocr-jobs".into())
                .spawn(move || work(&shared, engines))
                .map_err(|err| err.to_string())?
        };
        Ok(Self { shared, worker: Mutex::new(Some(worker)) })
    }

    /// Queues OCR of `pages` of the document from `path`.
    pub fn submit(
        &self,
        path: &Path,
        name: &str,
        page_count: usize,
        pages: &[usize],
        options: JobOptions,
    ) -> Result<JobStatus, String> {
        let path = &canonical_path(path);
        let mut seen = HashSet::new();
        let pages: Vec<usize> = pages.iter().copied().filter(|&p| p < page_count && seen.insert(p)).collect();
        if pages.is_empty() {
            return Err("no pages".into());
        }
        if let JobKind::Region { rect, .. } = &options.kind
            && (pages.len() != 1
                || !(rect.iter().all(|v| v.is_finite()) && rect[2] > rect[0] && rect[3] > rect[1]))
        {
            return Err("an area of one page".into());
        }
        let id = self.shared.book.create_job(path, name, page_count, &pages, &options)?;
        let record = JobRecord {
            id,
            path: path.to_owned(),
            name: name.to_owned(),
            page_count,
            pages,
            state: JobState::Queued,
            error: None,
            options,
        };
        let job =
            Job { record, drafted: HashSet::new(), finished: HashSet::new(), current: None, interrupt: None };
        let status = job.status();
        self.shared.jobs().insert(id, job);
        self.shared.host.changed(&status);
        self.shared.wake();
        Ok(status)
    }

    /// Every job of the document from `path`, oldest first.
    pub fn jobs(&self, path: &Path) -> Vec<JobStatus> {
        let path = canonical_path(path);
        self.shared.jobs().values().filter(|j| j.record.path == path).map(Job::status).collect()
    }

    /// Every job, oldest first.
    pub fn all(&self) -> Vec<JobStatus> {
        self.shared.jobs().values().map(Job::status).collect()
    }

    /// Stops a job after the page being read; it keeps what it did.
    pub fn pause(&self, id: JobId) -> Option<JobStatus> {
        let state = self.shared.jobs().get(&id)?.record.state;
        match state {
            JobState::Running => {
                self.shared.jobs().get_mut(&id)?.interrupt = Some(Interrupt::Pause);
                self.status(id)
            }
            JobState::Queued => {
                self.shared.set_state(id, JobState::Paused, None);
                self.status(id)
            }
            _ => self.status(id),
        }
    }

    /// Queues a paused, cancelled or failed job again: it carries on where it stopped.
    pub fn resume(&self, id: JobId) -> Option<JobStatus> {
        let state = self.shared.jobs().get(&id)?.record.state;
        if matches!(state, JobState::Paused | JobState::Cancelled | JobState::Failed) {
            if let Some(job) = self.shared.jobs().get_mut(&id) {
                job.interrupt = None;
            }
            self.shared.set_state(id, JobState::Queued, None);
            self.shared.wake();
        }
        self.status(id)
    }

    /// Stops a job now. What it wrote stays (and can be undone).
    pub fn cancel(&self, id: JobId) -> Option<JobStatus> {
        let state = self.shared.jobs().get(&id)?.record.state;
        match state {
            JobState::Running => {
                self.shared.jobs().get_mut(&id)?.interrupt = Some(Interrupt::Cancel);
                // A quality page can take minutes: stop the servers under it.
                if self
                    .shared
                    .jobs()
                    .get(&id)
                    .and_then(|j| j.current)
                    .is_some_and(|(_, pass)| pass == Pass::Quality)
                {
                    (self.shared.stop_engines)();
                }
            }
            JobState::Queued | JobState::Paused => self.shared.set_state(id, JobState::Cancelled, None),
            _ => {}
        }
        self.status(id)
    }

    /// Forgets a job that is not running, with its review list.
    pub fn remove(&self, id: JobId) -> Result<(), String> {
        if self.shared.jobs().get(&id).is_some_and(|j| j.record.state == JobState::Running) {
            return Err("the job is running".into());
        }
        self.shared.book.delete_job(id)?;
        self.shared.jobs().remove(&id);
        Ok(())
    }

    pub fn status(&self, id: JobId) -> Option<JobStatus> {
        self.shared.jobs().get(&id).map(Job::status)
    }

    /// The pages-needing-review list for the document from `path`, whose
    /// pages hold `states` (spec §5.8, ADR 0037): each page's result and
    /// decisions as the document shows them, pages needing a decision first.
    pub fn review(
        &self,
        path: &Path,
        states: &[LayerState],
        filter: ReviewFilter,
    ) -> Result<Vec<ReviewPage>, String> {
        let rows = final_rows(&self.shared, path, None)?;
        let mut list: Vec<ReviewPage> = rows
            .iter()
            .filter_map(|(page, rows)| {
                let rows: Vec<(JobId, &PageOcr)> = rows.iter().map(|(id, row)| (*id, row)).collect();
                let resolved = resolve(&rows, states.get(*page).copied().unwrap_or_default())?;
                review_page(&resolved, filter)
            })
            .collect();
        sort_review(&mut list);
        Ok(list)
    }

    /// Page `page` as the document from `path` shows it (`states`): its result
    /// with the decisions on its branch applied, and whether it was converted
    /// to page text. `None` when the page has no result of ours.
    pub fn page_view(
        &self,
        path: &Path,
        states: &[LayerState],
        page: usize,
    ) -> Result<Option<PageState>, String> {
        let rows = final_rows(&self.shared, path, Some(page))?;
        let Some(rows) = rows.get(&page) else { return Ok(None) };
        let rows: Vec<(JobId, &PageOcr)> = rows.iter().map(|(id, row)| (*id, row)).collect();
        Ok(resolve(&rows, states.get(page).copied().unwrap_or_default()).map(|r| PageState {
            view: r.view,
            converted: r.converted,
            stale: r.stale,
        }))
    }

    /// Takes a review decision on `word` (`None`: the page) of page `page` of
    /// the document from `path` (`states`), and keeps it. Returns the layer to
    /// write when the word's text changed (ADR 0037).
    pub fn decide(
        &self,
        path: &Path,
        states: &[LayerState],
        page: usize,
        word: Option<usize>,
        action: Action,
    ) -> Result<Decided, String> {
        let _one_at_a_time = self.shared.deciding.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = final_rows(&self.shared, path, Some(page))?;
        let rows = rows.get(&page).ok_or_else(|| DecideError::NoWord.code().to_owned())?;
        let refs: Vec<(JobId, &PageOcr)> = rows.iter().map(|(id, row)| (*id, row)).collect();
        let resolved = resolve(&refs, states.get(page).copied().unwrap_or_default())
            .ok_or_else(|| DecideError::NoWord.code().to_owned())?;
        let edits = matches!(action, Action::Edit { .. });
        if edits && resolved.converted {
            return Err(DecideError::Converted.code().into());
        }
        if edits && resolved.stale {
            return Err(DecideError::Stale.code().into());
        }
        let (_, row) = rows
            .iter()
            .find(|(id, _)| *id == resolved.job)
            .ok_or_else(|| DecideError::NoWord.code().to_owned())?;
        let mut row = row.clone();
        let (view, layer) =
            decide(&mut row, resolved.node, word, action).map_err(|err| err.code().to_owned())?;
        self.shared.book.save_page(resolved.job, &row)?;
        Ok(Decided { view, layer })
    }

    /// The latest verdict of a check job on each page of the document from `path` (ADR 0038).
    pub fn checks(&self, path: &Path) -> Result<Vec<PageCheck>, String> {
        let path = canonical_path(path);
        let ids: Vec<JobId> = self
            .shared
            .jobs()
            .values()
            .filter(|j| j.record.path == path && j.record.options.kind == JobKind::Check)
            .map(|j| j.record.id)
            .collect();
        let mut latest: BTreeMap<usize, PageCheck> = BTreeMap::new();
        for id in ids {
            for check in self.shared.book.checks(id)? {
                latest.insert(check.page, check);
            }
        }
        Ok(latest.into_values().collect())
    }

    /// The document was saved under another name: its jobs follow it.
    pub fn moved(&self, from: &Path, to: &Path, name: &str) {
        let (from, to) = (&canonical_path(from), &canonical_path(to));
        let ids: Vec<JobId> =
            self.shared.jobs().values().filter(|j| &j.record.path == from).map(|j| j.record.id).collect();
        for id in ids {
            if let Err(err) = self.shared.book.set_job_path(id, to, name) {
                eprintln!("safha: could not move an OCR job: {err}");
            }
            if let Some(job) = self.shared.jobs().get_mut(&id) {
                job.record.path = to.to_owned();
                job.record.name = name.to_owned();
            }
        }
    }

    /// Documents were opened or closed: a waiting job may run now.
    pub fn wake(&self) {
        self.shared.wake();
    }

    /// Stops the worker (the page being read is abandoned; its job resumes next time).
    pub fn shutdown(&self) {
        self.shared.signal().quit = true;
        (self.shared.stop_engines)();
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = worker.join();
        }
    }
}

impl Drop for JobRunner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------- the worker ----------

fn work(shared: &Shared, mut engines: Box<dyn JobEngines>) {
    let mut idle_since = Instant::now();
    loop {
        {
            let mut signal = shared.signal();
            if signal.quit {
                return;
            }
            signal.pending = false;
        }
        let candidates: Vec<(JobId, PathBuf, usize)> = shared
            .jobs()
            .values()
            .filter(|j| j.record.state.active() && j.interrupt.is_none())
            .map(|j| (j.record.id, j.record.path.clone(), j.record.page_count))
            .collect();
        let mut next = None;
        for (id, path, page_count) in candidates {
            match shared.host.page_count(&path) {
                Some(count) if count == page_count => {
                    next = Some((id, path));
                    break;
                }
                // Opened, but not the document the job was made for.
                Some(_) => shared.set_state(id, JobState::Failed, Some("ocr_document_changed")),
                None => {}
            }
        }
        let Some((id, path)) = next else {
            engines.idle(idle_since);
            let signal = shared.signal();
            if !signal.quit && !signal.pending {
                let _ = shared.wake.wait_timeout(signal, Duration::from_secs(30));
            }
            continue;
        };
        shared.set_state(id, JobState::Running, None);
        let outcome = run(shared, engines.as_mut(), id, &path);
        let interrupt = shared.interrupt(id);
        if let Some(job) = shared.jobs().get_mut(&id) {
            job.interrupt = None;
        }
        match (outcome, interrupt) {
            (_, Some(Interrupt::Cancel)) => shared.set_state(id, JobState::Cancelled, None),
            (Ok(Finish::Done), _) => shared.set_state(id, JobState::Done, None),
            (_, Some(Interrupt::Pause)) => shared.set_state(id, JobState::Paused, None),
            (Ok(Finish::Closed), _) => shared.set_state(id, JobState::Queued, None),
            (Ok(Finish::Stopped), _) => shared.set_state(id, JobState::Queued, None),
            (Err(code), _) => shared.set_state(id, JobState::Failed, Some(&code)),
        }
        idle_since = Instant::now();
    }
}

enum Finish {
    Done,
    /// The document was closed: the job waits for it.
    Closed,
    /// Interrupted (pause, cancel, or the app quitting).
    Stopped,
}

fn stop_requested(shared: &Shared, id: JobId) -> bool {
    shared.interrupt(id).is_some() || shared.quitting()
}

fn set_current(shared: &Shared, id: JobId, current: Option<(usize, Pass)>) {
    if let Some(job) = shared.jobs().get_mut(&id) {
        job.current = current;
    }
    shared.notify(id);
}

fn run(shared: &Shared, engines: &mut dyn JobEngines, id: JobId, path: &Path) -> Result<Finish, String> {
    let options = match shared.jobs().get(&id) {
        Some(job) => job.record.options.clone(),
        None => return Ok(Finish::Done),
    };
    match options.kind {
        JobKind::Recognize => run_recognize(shared, engines, id, path, options.replace),
        JobKind::Region { rect, language, numbers } => {
            run_region(shared, engines, id, path, rect, Constraint { language, numbers })
        }
        JobKind::Check => run_check(shared, engines, id, path),
    }
}

fn run_recognize(
    shared: &Shared,
    engines: &mut dyn JobEngines,
    id: JobId,
    path: &Path,
    replace: bool,
) -> Result<Finish, String> {
    let clear = if replace { Clear::Page } else { Clear::None };
    let pages = match shared.jobs().get(&id) {
        Some(job) => job.record.pages.clone(),
        None => return Ok(Finish::Done),
    };
    // Results the document lacks (it was not saved when the app quit) go back first.
    let stored = shared.book.pages(id)?;
    if let Some(present) = shared.host.layers(path) {
        let missing: Vec<PageLayer> = stored
            .iter()
            .filter(|p| present.get(p.page).copied().flatten() < Some(p.quality()))
            .map(|p| p.layer.clone())
            .collect();
        if !missing.is_empty() && !shared.host.apply(path, &missing)? {
            return Ok(Finish::Closed);
        }
    }

    // 1. Draft pass.
    let waiting: Vec<usize> = {
        let jobs = shared.jobs();
        let Some(job) = jobs.get(&id) else { return Ok(Finish::Done) };
        pages.iter().copied().filter(|p| !job.drafted.contains(p)).collect()
    };
    let mut batch: Vec<PageLayer> = Vec::new();
    let mut flushed = Instant::now();
    let flush = |batch: &mut Vec<PageLayer>| -> Result<bool, String> {
        if batch.is_empty() {
            return Ok(true);
        }
        let open = shared.host.apply(path, batch)?;
        batch.clear();
        Ok(open)
    };
    let page_count = shared.jobs().get(&id).map_or(0, |j| j.record.page_count);
    let changed = || shared.host.page_count(path).is_some_and(|count| count != page_count);
    for page in waiting {
        if stop_requested(shared, id) {
            return if flush(&mut batch)? { Ok(Finish::Stopped) } else { Ok(Finish::Closed) };
        }
        if changed() {
            return Err("ocr_document_changed".into());
        }
        if engines.draft().is_none() {
            break;
        }
        set_current(shared, id, Some((page, Pass::Draft)));
        let rendered = match render(shared, path, page, DRAFT_SCALE)? {
            Some(rendered) => rendered,
            None => return Ok(Finish::Closed),
        };
        let Some(reader) = engines.draft() else { break };
        let started = Instant::now();
        let draft = match reader.read(&rendered.image) {
            Ok(draft) => draft,
            Err(err) => {
                // The draft is a convenience: without it the quality pass still runs.
                eprintln!("safha: the draft reader failed on page {}: {err}", page + 1);
                break;
            }
        };
        let mut result = draft_page(page, &draft, &rendered, reader.name(), started.elapsed());
        result.layer.clear = clear;
        shared.book.save_page(id, &result)?;
        batch.push(result.layer);
        if let Some(job) = shared.jobs().get_mut(&id) {
            job.drafted.insert(page);
        }
        if (batch.len() >= DRAFT_BATCH || flushed.elapsed() >= DRAFT_FLUSH) && !flush(&mut batch)? {
            return Ok(Finish::Closed);
        }
        if batch.is_empty() {
            flushed = Instant::now();
        }
    }
    if !flush(&mut batch)? {
        return Ok(Finish::Closed);
    }

    // 2. Quality pass.
    let waiting: Vec<usize> = {
        let jobs = shared.jobs();
        let Some(job) = jobs.get(&id) else { return Ok(Finish::Done) };
        pages.iter().copied().filter(|p| !job.finished.contains(p)).collect()
    };
    if waiting.is_empty() {
        return Ok(Finish::Done);
    }
    for page in waiting {
        if stop_requested(shared, id) {
            return Ok(Finish::Stopped);
        }
        if changed() {
            return Err("ocr_document_changed".into());
        }
        set_current(shared, id, Some((page, Pass::Quality)));
        let (primary, partner) = engines.quality().map_err(|err| err.code().to_owned())?;
        let rendered = match render(shared, path, page, QUALITY_SCALE)? {
            Some(rendered) => rendered,
            None => return Ok(Finish::Closed),
        };
        let started = Instant::now();
        let guide = engines.draft().and_then(|reader| reader.read(&rendered.image).ok());
        let reader = QualityReader {
            primary: &*primary,
            partner: partner.as_deref().map(|p| p as &dyn OcrEngine),
            third: None,
            settings: Settings::default(),
        };
        let result = match reader.read_page(&rendered.image) {
            Ok(result) => result,
            Err(_) if stop_requested(shared, id) => return Ok(Finish::Stopped),
            Err(err) => return Err(err.code().to_owned()),
        };
        let name = match &partner {
            Some(partner) => format!("{}+{}", primary.name(), partner.name()),
            None => primary.name().to_owned(),
        };
        let mut result = final_page(page, &result, &rendered, guide.as_ref(), &name, started.elapsed());
        result.layer.clear = clear;
        shared.book.save_page(id, &result)?;
        if let Some(job) = shared.jobs().get_mut(&id) {
            job.drafted.insert(page);
            job.finished.insert(page);
        }
        if !shared.host.apply(path, std::slice::from_ref(&result.layer))? {
            return Ok(Finish::Closed);
        }
        shared.notify(id);
    }
    Ok(Finish::Done)
}

/// Reads one area of a page again with the quality pipeline, every word of it
/// re-read, and puts the result in the place of the area's text (ADR 0038).
fn run_region(
    shared: &Shared,
    engines: &mut dyn JobEngines,
    id: JobId,
    path: &Path,
    rect: [f32; 4],
    constraint: Constraint,
) -> Result<Finish, String> {
    let Some(page) = shared.jobs().get(&id).and_then(|job| job.record.pages.first().copied()) else {
        return Ok(Finish::Done);
    };
    if shared.jobs().get(&id).is_some_and(|job| job.finished.contains(&page)) {
        return Ok(Finish::Done);
    }
    if stop_requested(shared, id) {
        return Ok(Finish::Stopped);
    }
    // What the area's result is merged into: the page as the document shows it.
    let Some(states) = shared.host.states(path) else { return Ok(Finish::Closed) };
    let state = states.get(page).copied().unwrap_or_default();
    let rows = final_rows(shared, path, Some(page))?;
    let base = match rows.get(&page) {
        Some(rows) => {
            let refs: Vec<(JobId, &PageOcr)> = rows.iter().map(|(id, row)| (*id, row)).collect();
            resolve(&refs, state)
        }
        None => None,
    };
    if state.quality.is_none() && state.converted.is_some() {
        return Err("ocr_region_converted".into());
    }
    // A draft has no words to merge with: the page is still being read.
    if state.quality == Some(LayerQuality::Draft) {
        return Err("ocr_region_draft".into());
    }
    match (&base, state.quality) {
        // A layer of ours with no record of its words: merging would lose them.
        (None, Some(_)) => return Err("ocr_region_unknown_layer".into()),
        (Some(base), Some(_)) if base.stale => return Err("ocr_region_unknown_layer".into()),
        _ => {}
    }

    set_current(shared, id, Some((page, Pass::Quality)));
    let (primary, partner) = engines.quality().map_err(|err| err.code().to_owned())?;
    let scale = region_scale(rect);
    let rendered = match shared.host.render_area(path, page, rect, scale) {
        Ok(rendered) => rendered,
        Err(_) if shared.host.page_count(path).is_none() => return Ok(Finish::Closed),
        Err(err) => {
            eprintln!("safha: an area of page {} could not be rendered for OCR: {err}", page + 1);
            return Err("ocr_image_invalid".into());
        }
    };
    let started = Instant::now();
    let guide = engines.draft().and_then(|reader| reader.read(&rendered.image).ok());
    // An area is small: every word of it is checked again (accuracy first).
    let reader = QualityReader {
        primary: &*primary,
        partner: partner.as_deref().map(|p| p as &dyn OcrEngine),
        third: None,
        settings: Settings { low_confidence: 1.01, reread_all_strips: true },
    };
    let mut result = match reader.read_page(&rendered.image) {
        Ok(result) => result,
        Err(_) if stop_requested(shared, id) => return Ok(Finish::Stopped),
        Err(err) => return Err(err.code().to_owned()),
    };
    constrain(&mut result.words, constraint);
    let name = match &partner {
        Some(partner) => format!("{}+{}", primary.name(), partner.name()),
        None => primary.name().to_owned(),
    };
    let mut area = final_page(page, &result, &rendered, guide.as_ref(), &name, started.elapsed());
    shift_page(&mut area, rect[0], rect[1], constraint.language);
    let merged = merge_region(base.as_ref().map(|b| &b.view), area, rect);
    shared.book.save_page(id, &merged)?;
    if let Some(job) = shared.jobs().get_mut(&id) {
        job.drafted.insert(page);
        job.finished.insert(page);
    }
    if !shared.host.apply(path, std::slice::from_ref(&merged.layer))? {
        return Ok(Finish::Closed);
    }
    Ok(Finish::Done)
}

/// Compares each page's existing text layer with a quick draft of its image,
/// and keeps a verdict per page (ADR 0038). Nothing is written.
fn run_check(
    shared: &Shared,
    engines: &mut dyn JobEngines,
    id: JobId,
    path: &Path,
) -> Result<Finish, String> {
    let waiting: Vec<usize> = match shared.jobs().get(&id) {
        Some(job) => job.record.pages.iter().copied().filter(|p| !job.finished.contains(p)).collect(),
        None => return Ok(Finish::Done),
    };
    let page_count = shared.jobs().get(&id).map_or(0, |j| j.record.page_count);
    for page in waiting {
        if stop_requested(shared, id) {
            return Ok(Finish::Stopped);
        }
        match shared.host.page_count(path) {
            None => return Ok(Finish::Closed),
            Some(count) if count != page_count => return Err("ocr_document_changed".into()),
            Some(_) => {}
        }
        set_current(shared, id, Some((page, Pass::Check)));
        let info = match shared.host.text_info(path, page) {
            Ok(info) => info,
            Err(_) if shared.host.page_count(path).is_none() => return Ok(Finish::Closed),
            Err(err) => {
                eprintln!("safha: the text of page {} could not be read: {err}", page + 1);
                return Err("ocr_image_invalid".into());
            }
        };
        let draft = if info.needs_draft() {
            match render(shared, path, page, DRAFT_SCALE)? {
                Some(rendered) => engines.draft().and_then(|reader| reader.read(&rendered.image).ok()),
                None => return Ok(Finish::Closed),
            }
        } else {
            None
        };
        let check = check_page(page, &info, draft.as_ref().map(DraftPage::text).as_deref());
        shared.book.save_check(id, &check)?;
        if let Some(job) = shared.jobs().get_mut(&id) {
            job.drafted.insert(page);
            job.finished.insert(page);
        }
        shared.notify(id);
    }
    Ok(Finish::Done)
}

/// The page rendered, `None` when its document was closed meanwhile. A page that
/// cannot be rendered stops the job with `ocr_image_invalid`.
fn render(shared: &Shared, path: &Path, page: usize, scale: f32) -> Result<Option<RenderedPage>, String> {
    match shared.host.render(path, page, scale) {
        Ok(rendered) => Ok(Some(rendered)),
        Err(_) if shared.host.page_count(path).is_none() => Ok(None),
        Err(err) => {
            eprintln!("safha: page {} could not be rendered for OCR: {err}", page + 1);
            Err("ocr_image_invalid".into())
        }
    }
}

// ---------- assembling a page's result ----------

fn to_points(b: PixelBox, scale: f32) -> [f32; 4] {
    [b[0] / scale, b[1] / scale, b[2] / scale, b[3] / scale]
}

/// A draft reading as a text layer: the reader's own word boxes; where some are
/// missing, the line's box shared among its words by length, in visual order.
pub fn draft_page(
    page: usize,
    draft: &DraftPage,
    rendered: &RenderedPage,
    engine: &str,
    took: Duration,
) -> PageOcr {
    let lines = draft
        .lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .map(|line| {
            let rtl = first_strong_rtl(&line.text).unwrap_or(true);
            let texts: Vec<&str> = if line.words.is_empty() {
                line.text.split_whitespace().collect()
            } else {
                line.words.iter().map(|w| w.text.as_str()).collect()
            };
            let boxes: Vec<PixelBox> =
                if !line.words.is_empty() && line.words.iter().all(|w| w.bounds.is_some()) {
                    line.words.iter().filter_map(|w| w.bounds).collect()
                } else {
                    share_line(line.bounds, &texts, rtl)
                };
            LayerLine {
                rtl,
                words: texts
                    .iter()
                    .zip(boxes)
                    .map(|(text, b)| {
                        let [left, top, right, bottom] = to_points(b, rendered.scale);
                        LayerWord { text: (*text).to_owned(), left, top, right, bottom }
                    })
                    .collect(),
            }
        })
        .collect();
    PageOcr {
        page,
        layer: PageLayer {
            page,
            quality: LayerQuality::Draft,
            lines,
            placement: rendered.placement,
            clear: Clear::None,
        },
        words: Vec::new(),
        truncated: false,
        engine: engine.to_owned(),
        seconds: took.as_secs_f32(),
        checked: false,
        decisions: Vec::new(),
        digest: None,
    }
}

/// `line` divided among `words` (logical order) by their length, in visual order.
fn share_line(line: PixelBox, words: &[&str], rtl: bool) -> Vec<PixelBox> {
    let order = visual_order(words, rtl);
    let length = |w: &str| w.chars().count().max(1) as f32;
    let total: f32 = words.iter().map(|w| length(w) + 1.0).sum::<f32>() - 1.0;
    let unit = (line[2] - line[0]) / total.max(1.0);
    let mut out = vec![line; words.len()];
    let mut x = line[0];
    for i in order {
        let width = length(words[i]) * unit;
        out[i] = [x, line[1], x + width, line[3]];
        x += width + unit;
    }
    out
}

/// A quality reading as a text layer, with its words for the review list. Word
/// boxes come from aligning the words to the draft reader's lines (`guide`).
pub fn final_page(
    page: usize,
    result: &PageResult,
    rendered: &RenderedPage,
    guide: Option<&DraftPage>,
    engine: &str,
    took: Duration,
) -> PageOcr {
    let texts: Vec<String> = result.words.iter().map(|w| w.text.clone()).collect();
    let paragraphs: Vec<usize> = result.words.iter().map(|w| w.line).collect();
    let lines = word_boxes(&rendered.image, &texts, &paragraphs, guide);
    let mut bounds: Vec<Option<[f32; 4]>> = vec![None; texts.len()];
    let mut by_width = vec![false; texts.len()];
    let mut at: Vec<Option<[u32; 2]>> = vec![None; texts.len()];
    let layer_lines = lines
        .iter()
        .enumerate()
        .map(|(l, line)| LayerLine {
            rtl: line.rtl,
            words: line
                .words
                .iter()
                .enumerate()
                .map(|(p, w)| {
                    let b = to_points(w.bounds, rendered.scale);
                    bounds[w.word] = Some(b);
                    by_width[w.word] = !w.anchored;
                    at[w.word] = Some([l as u32, p as u32]);
                    LayerWord {
                        text: texts[w.word].clone(),
                        left: b[0],
                        top: b[1],
                        right: b[2],
                        bottom: b[3],
                    }
                })
                .collect(),
        })
        .collect();
    let layer = PageLayer {
        page,
        quality: LayerQuality::Final,
        lines: layer_lines,
        placement: rendered.placement,
        clear: Clear::None,
    };
    PageOcr {
        page,
        digest: Some(layer.digest()),
        layer,
        words: result
            .words
            .iter()
            .enumerate()
            .map(|(i, w)| OcrWord {
                text: w.text.clone(),
                probability: w.probability,
                status: w.status.clone(),
                bounds: bounds[i],
                placed_by_width: by_width[i],
                at: at[i],
                review: WordReview::Open,
            })
            .collect(),
        truncated: result.truncated,
        engine: engine.to_owned(),
        seconds: took.as_secs_f32(),
        checked: false,
        decisions: Vec::new(),
    }
}

// ---------- the engines on this machine ----------

/// The engines installed on this machine: Nuqta 4B and 0.8B through the
/// downloaded `llama-server` (ADR 0032, ADR 0034), on the GPU when the runtime
/// has a backend for it, and the platform's draft reader.
pub struct LocalEngines {
    models: ModelStore,
    /// Where helper programs are built (the Vision helper).
    helpers: PathBuf,
    device: Device,
    logs: Option<PathBuf>,
    draft: Option<Box<dyn DraftReader>>,
    draft_tried: bool,
    primary: Option<Arc<NuqtaEngine>>,
    partner: Option<Arc<NuqtaEngine>>,
    running: Arc<Mutex<Vec<Arc<NuqtaEngine>>>>,
    last_used: Instant,
}

impl LocalEngines {
    /// `models`: the installed packages; `helpers`: a folder for built helpers;
    /// `logs`: where the servers write their logs (none when `None`).
    pub fn new(models: ModelStore, helpers: PathBuf, logs: Option<PathBuf>) -> Self {
        let device = if Hardware::detect().runtime_gpu.is_some() { Device::Gpu } else { Device::Cpu };
        Self {
            models,
            helpers,
            device,
            logs,
            draft: None,
            draft_tried: false,
            primary: None,
            partner: None,
            running: Arc::new(Mutex::new(Vec::new())),
            last_used: Instant::now(),
        }
    }

    fn start(&self, id: &str, model: NuqtaModel) -> Result<Arc<NuqtaEngine>, OcrError> {
        let runtime =
            self.models.installed(RUNTIME).ok_or_else(|| OcrError::EngineMissing(PathBuf::from(RUNTIME)))?;
        let program = runtime.program.ok_or_else(|| OcrError::EngineMissing(runtime.dir.clone()))?;
        let package = self.models.installed(id).ok_or_else(|| OcrError::ModelMissing(PathBuf::from(id)))?;
        let mut config = ServerConfig::for_model(program, &package.dir, model);
        config.device = self.device;
        config.log = self.logs.as_ref().map(|dir| dir.join(format!("llama-server-{id}.log")));
        let engine = Arc::new(NuqtaEngine::start(&config, model)?);
        self.running.lock().unwrap_or_else(PoisonError::into_inner).push(Arc::clone(&engine));
        Ok(engine)
    }
}

impl JobEngines for LocalEngines {
    fn draft(&mut self) -> Option<&mut dyn DraftReader> {
        if !self.draft_tried {
            self.draft_tried = true;
            self.draft = local_draft_reader(&self.helpers);
        }
        self.draft.as_deref_mut().map(|reader| reader as &mut dyn DraftReader)
    }

    fn quality(&mut self) -> Result<(SharedEngine, Option<SharedEngine>), OcrError> {
        self.last_used = Instant::now();
        if self.primary.as_ref().is_none_or(|e| !e.is_running()) {
            self.primary = Some(self.start(PRIMARY, NuqtaModel::DEFAULT)?);
        }
        if self.partner.as_ref().is_none_or(|e| !e.is_running()) && self.models.installed(ENSEMBLE).is_some()
        {
            self.partner = Some(self.start(ENSEMBLE, NuqtaModel::PARTNER)?);
        }
        let primary: SharedEngine = match self.primary.clone() {
            Some(primary) => primary,
            None => return Err(OcrError::Timeout("no primary engine")),
        };
        let partner: Option<SharedEngine> = self.partner.clone().map(|p| p as SharedEngine);
        Ok((primary, partner))
    }

    fn idle(&mut self, since: Instant) {
        if since.max(self.last_used).elapsed() >= IDLE_STOP
            && (self.primary.is_some() || self.partner.is_some())
        {
            self.primary = None;
            self.partner = None;
            self.running.lock().unwrap_or_else(PoisonError::into_inner).clear();
        }
        if since.elapsed() >= IDLE_STOP {
            self.draft = None;
            self.draft_tried = false;
        }
    }

    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let running = Arc::clone(&self.running);
        Arc::new(move || {
            for engine in running.lock().unwrap_or_else(PoisonError::into_inner).drain(..) {
                engine.stop();
            }
        })
    }
}

/// The draft reader for this machine: Apple Vision on macOS (its helper built in
/// `helpers` on first use), else Tesseract with Arabic and English data if it is
/// installed (`SAFHA_TESSERACT` names the program). `None` when there is neither.
pub fn local_draft_reader(helpers: &Path) -> Option<Box<dyn DraftReader>> {
    if cfg!(target_os = "macos") {
        match vision_helper(helpers).and_then(|program| VisionReader::start(&program)) {
            Ok(reader) => return Some(Box::new(reader)),
            Err(err) => eprintln!("safha: Apple Vision is not available for drafts: {err}"),
        }
    }
    let program =
        std::env::var_os("SAFHA_TESSERACT").map_or_else(|| PathBuf::from("tesseract"), PathBuf::from);
    let languages = std::process::Command::new(&program).arg("--list-langs").output().ok()?;
    let listed = String::from_utf8_lossy(&languages.stdout).into_owned();
    let has = |code: &str| listed.lines().any(|line| line.trim() == code);
    (has("ara") && has("eng")).then(|| {
        Box::new(TesseractReader { program, languages: "ara+eng".into(), data: None, dpi: 200 })
            as Box<dyn DraftReader>
    })
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
