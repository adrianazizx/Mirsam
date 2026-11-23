//! The job runner with a scripted document host and engines.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::super::draft::{DraftLine, DraftWord};
use super::super::{Recognition, Timings, WordConfidence};
use super::*;
use crate::store::Store;

/// Open documents: page count and the layer each page has.
#[derive(Default)]
struct Host {
    documents: Mutex<HashMap<PathBuf, Vec<Option<LayerQuality>>>>,
    /// Pages written, one entry per edit.
    edits: Mutex<Vec<Vec<(usize, LayerQuality)>>>,
    events: Mutex<Vec<JobStatus>>,
}

impl Host {
    fn open(&self, path: &Path, pages: usize) {
        self.documents.lock().expect("lock").insert(canonical_path(path), vec![None; pages]);
    }

    fn close(&self, path: &Path) {
        self.documents.lock().expect("lock").remove(&canonical_path(path));
    }

    fn layers_of(&self, path: &Path) -> Vec<Option<LayerQuality>> {
        self.documents.lock().expect("lock").get(&canonical_path(path)).cloned().unwrap_or_default()
    }

    fn edits(&self) -> Vec<Vec<(usize, LayerQuality)>> {
        self.edits.lock().expect("lock").clone()
    }
}

impl JobHost for Host {
    fn page_count(&self, path: &Path) -> Option<usize> {
        self.documents.lock().expect("lock").get(path).map(Vec::len)
    }

    fn render(&self, _path: &Path, _page: usize, scale: f32) -> Result<RenderedPage, String> {
        // A page with two dark text lines.
        let (w, h) = (300usize, 200usize);
        let mut rgb = vec![255u8; w * h * 3];
        for top in [50usize, 120] {
            for y in top..top + 14 {
                for x in 30..270 {
                    if x % 40 < 34 {
                        rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
        let image = OcrImage::from_rgb(w as u32, h as u32, rgb).map_err(|e| e.to_string())?;
        Ok(RenderedPage { image, scale, placement: [1.0, 0.0, 0.0, 1.0, 0.0, 842.0] })
    }

    fn layers(&self, path: &Path) -> Option<Vec<Option<LayerQuality>>> {
        self.documents.lock().expect("lock").get(path).cloned()
    }

    fn apply(&self, path: &Path, layers: &[PageLayer]) -> Result<bool, String> {
        let mut documents = self.documents.lock().expect("lock");
        let Some(document) = documents.get_mut(path) else { return Ok(false) };
        for layer in layers {
            document[layer.page] = Some(layer.quality);
        }
        self.edits.lock().expect("lock").push(layers.iter().map(|l| (l.page, l.quality)).collect());
        Ok(true)
    }

    fn changed(&self, status: &JobStatus) {
        self.events.lock().expect("lock").push(status.clone());
    }
}

/// Answers every image with `text`; counts reads; can be held until stopped.
struct Scripted {
    name: &'static str,
    text: String,
    reads: Arc<AtomicU32>,
    hold: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl OcrEngine for Scripted {
    fn name(&self) -> &str {
        self.name
    }

    fn recognize(&self, _image: &OcrImage) -> Result<Recognition, OcrError> {
        while self.hold.load(Ordering::SeqCst) {
            if self.stopped.load(Ordering::SeqCst) {
                return Err(OcrError::Exited("stopped".into()));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        let words = self
            .text
            .split_whitespace()
            .map(|w| WordConfidence { word: w.into(), probability: 0.99 })
            .collect();
        Ok(Recognition {
            text: self.text.clone(),
            raw: self.text.clone(),
            words,
            truncated: false,
            timings: Timings::default(),
        })
    }
}

struct Draft;

impl DraftReader for Draft {
    fn name(&self) -> &str {
        "draft"
    }

    fn read(&mut self, _image: &OcrImage) -> Result<DraftPage, OcrError> {
        let word = |text: &str, x0: f32, x1: f32| DraftWord {
            text: text.into(),
            bounds: Some([x0, 50.0, x1, 64.0]),
        };
        Ok(DraftPage {
            lines: vec![DraftLine {
                text: "نص عربي".into(),
                confidence: 0.5,
                bounds: [30.0, 50.0, 270.0, 64.0],
                words: vec![word("عربي", 30.0, 140.0), word("نص", 150.0, 270.0)],
            }],
        })
    }
}

#[derive(Clone)]
struct Engines {
    primary: Arc<Scripted>,
    stopped: Arc<AtomicBool>,
    stops: Arc<AtomicU32>,
}

impl Engines {
    fn new(text: &str) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        Self {
            primary: Arc::new(Scripted {
                name: "primary",
                text: text.into(),
                reads: Arc::new(AtomicU32::new(0)),
                hold: Arc::new(AtomicBool::new(false)),
                stopped: Arc::clone(&stopped),
            }),
            stopped,
            stops: Arc::new(AtomicU32::new(0)),
        }
    }

    fn reads(&self) -> u32 {
        self.primary.reads.load(Ordering::SeqCst)
    }
}

impl JobEngines for Engines {
    fn draft(&mut self) -> Option<&mut dyn DraftReader> {
        // A fresh one each time is fine: it keeps no state.
        Some(Box::leak(Box::new(Draft)))
    }

    fn quality(&mut self) -> Result<(SharedEngine, Option<SharedEngine>), OcrError> {
        self.stopped.store(false, Ordering::SeqCst);
        Ok((Arc::clone(&self.primary) as SharedEngine, None))
    }

    fn idle(&mut self, _since: Instant) {}

    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let (stopped, stops) = (Arc::clone(&self.stopped), Arc::clone(&self.stops));
        Arc::new(move || {
            stopped.store(true, Ordering::SeqCst);
            stops.fetch_add(1, Ordering::SeqCst);
        })
    }
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ready() {
        assert!(started.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn state(runner: &JobRunner, id: JobId) -> JobState {
    runner.status(id).expect("job").state
}

fn file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-ocr-jobs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join(name);
    std::fs::write(&path, b"%PDF-1.7\n").expect("file");
    path
}

#[test]
fn a_job_drafts_every_page_then_replaces_each_with_the_quality_result() {
    let (book, host, engines) =
        (Arc::new(Store::in_memory().expect("store")), Arc::new(Host::default()), Engines::new("نص ٢٠٢٥"));
    let path = file("a.pdf");
    host.open(&path, 4);
    let runner = JobRunner::start(book.clone(), host.clone(), Box::new(engines.clone())).expect("starts");
    let job = runner.submit(&path, "a.pdf", 4, &[2, 0, 1, 9, 0]).expect("submits");
    assert_eq!(job.pages, 3, "pages out of range and repeats dropped");
    wait_for("done", || state(&runner, job.id) == JobState::Done);

    // The drafts went in first as one edit, then one edit per quality page.
    let edits = host.edits();
    assert_eq!(edits[0], [(2, LayerQuality::Draft), (0, LayerQuality::Draft), (1, LayerQuality::Draft)]);
    assert_eq!(
        &edits[1..],
        [vec![(2, LayerQuality::Final)], vec![(0, LayerQuality::Final)], vec![(1, LayerQuality::Final)]]
    );
    assert_eq!(
        host.layers_of(&path),
        [Some(LayerQuality::Final), Some(LayerQuality::Final), Some(LayerQuality::Final), None]
    );
    let status = runner.status(job.id).expect("job");
    assert_eq!((status.drafted, status.finished, status.current), (3, 3, None));

    // Kept with each word's confidence, status and box.
    let pages = book.pages(job.id).expect("pages");
    assert_eq!(pages.len(), 3);
    let page = pages.iter().find(|p| p.page == 2).expect("page 2");
    assert_eq!(page.quality(), LayerQuality::Final);
    assert_eq!(page.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["نص", "٢٠٢٥"]);
    assert!(page.words.iter().all(|w| (w.probability - 0.99).abs() < 1e-6 && !w.status.needs_review()));
    assert!(page.words.iter().all(|w| w.bounds.is_some()));
    assert!(runner.review(&path).expect("review").is_empty(), "nothing to review");
    // The events said where it was.
    let events = host.events.lock().expect("lock").clone();
    assert!(events.iter().any(|e| e.current == Some((0, Pass::Quality))));
    assert_eq!(events.last().map(|e| e.state), Some(JobState::Done));
}

#[test]
fn cancel_stops_the_engines_mid_page_and_keeps_what_was_written() {
    let (book, host, engines) =
        (Arc::new(Store::in_memory().expect("store")), Arc::new(Host::default()), Engines::new("نص"));
    let path = file("b.pdf");
    host.open(&path, 2);
    engines.primary.hold.store(true, Ordering::SeqCst);
    let runner = JobRunner::start(book, host.clone(), Box::new(engines.clone())).expect("starts");
    let job = runner.submit(&path, "b.pdf", 2, &[0, 1]).expect("submits");
    wait_for("the quality pass", || {
        runner.status(job.id).and_then(|s| s.current) == Some((0, Pass::Quality))
    });
    runner.cancel(job.id);
    wait_for("cancelled", || state(&runner, job.id) == JobState::Cancelled);
    assert_eq!(engines.stops.load(Ordering::SeqCst), 1);
    assert_eq!(host.layers_of(&path), [Some(LayerQuality::Draft), Some(LayerQuality::Draft)]);

    // Resumed, it carries on with the quality pass only.
    engines.primary.hold.store(false, Ordering::SeqCst);
    runner.resume(job.id);
    wait_for("done", || state(&runner, job.id) == JobState::Done);
    assert_eq!(host.edits().len(), 3, "drafts once, then two quality pages");
}

#[test]
fn a_job_survives_a_restart_and_restores_what_the_document_lost() {
    let (book, host) = (Arc::new(Store::in_memory().expect("store")), Arc::new(Host::default()));
    let path = file("c.pdf");
    host.open(&path, 3);
    let engines = Engines::new("نص");
    let job = {
        let runner = JobRunner::start(book.clone(), host.clone(), Box::new(engines.clone())).expect("starts");
        // Held on the first quality page, so the pause comes before the job can finish.
        engines.primary.hold.store(true, Ordering::SeqCst);
        let job = runner.submit(&path, "c.pdf", 3, &[0, 1, 2]).expect("submits");
        wait_for("the first quality page", || {
            runner.status(job.id).and_then(|s| s.current) == Some((0, Pass::Quality))
        });
        runner.pause(job.id);
        engines.primary.hold.store(false, Ordering::SeqCst);
        wait_for("paused", || state(&runner, job.id) == JobState::Paused);
        assert_eq!(runner.status(job.id).map(|s| s.finished), Some(1), "stops after the page being read");
        job
        // The app quits here.
    };
    let reads_before = engines.reads();
    let finished_before =
        book.pages(job.id).expect("pages").iter().filter(|p| p.quality() == LayerQuality::Final).count();
    assert!(finished_before >= 1);

    // Reopened without the unsaved layers.
    host.close(&path);
    host.open(&path, 3);
    let runner =
        JobRunner::start(book.clone(), host.clone(), Box::new(engines.clone())).expect("starts again");
    assert_eq!(state(&runner, job.id), JobState::Paused, "a paused job stays paused");
    runner.resume(job.id);
    wait_for("done", || state(&runner, job.id) == JobState::Done);
    assert_eq!(host.layers_of(&path), [Some(LayerQuality::Final); 3]);
    // Only the pages not finished before were read again (one page read each: no targets).
    assert_eq!(engines.reads() - reads_before, 3 - finished_before as u32);
}

#[test]
fn a_job_waits_for_its_document_and_refuses_a_changed_one() {
    let (book, host, engines) =
        (Arc::new(Store::in_memory().expect("store")), Arc::new(Host::default()), Engines::new("نص"));
    let path = file("d.pdf");
    let runner = JobRunner::start(book, host.clone(), Box::new(engines)).expect("starts");
    let job = runner.submit(&path, "d.pdf", 2, &[0, 1]).expect("submits");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(state(&runner, job.id), JobState::Queued, "not open yet");
    host.open(&path, 2);
    runner.wake();
    wait_for("done", || state(&runner, job.id) == JobState::Done);

    // A job made for the 2-page document, when the file now open has 5 pages
    // (changed before submitting: a one-page job would otherwise finish first).
    host.close(&path);
    host.open(&path, 5);
    let other = runner.submit(&path, "d.pdf", 2, &[0]).expect("submits");
    wait_for("failed", || state(&runner, other.id) == JobState::Failed);
    assert_eq!(runner.status(other.id).and_then(|s| s.error).as_deref(), Some("ocr_document_changed"));
    runner.remove(other.id).expect("removes");
    assert_eq!(runner.jobs(&path).len(), 1);
}

#[test]
fn the_review_list_puts_decisions_first() {
    let word = |text: &str, status: WordStatus| OcrWord {
        text: text.into(),
        probability: 0.9,
        status,
        bounds: None,
        placed_by_width: false,
    };
    let page = |page: usize, words: Vec<OcrWord>| PageOcr {
        page,
        layer: PageLayer {
            page,
            quality: LayerQuality::Final,
            lines: vec![],
            placement: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        },
        words,
        truncated: false,
        engine: "x".into(),
        seconds: 1.0,
    };
    let pages = [
        page(0, vec![word("a", WordStatus::Unchecked)]),
        page(1, vec![word("b", WordStatus::Corrected { read: "c".into() })]),
        page(
            2,
            vec![
                word("d", WordStatus::Uncertain { alternatives: vec!["e".into()] }),
                word("f", WordStatus::Confirmed),
            ],
        ),
    ];
    let list = review(&pages);
    assert_eq!(list.iter().map(|p| p.page).collect::<Vec<_>>(), [2, 1]);
    assert!(list[0].needs_decision() && !list[1].needs_decision());
    assert_eq!((list[0].words.len(), list[0].words[0].index), (1, 0));
}
