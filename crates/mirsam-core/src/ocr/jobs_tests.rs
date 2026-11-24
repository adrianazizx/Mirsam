//! The job runner with a scripted document host and engines.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::super::check::Verdict;
use super::super::draft::{DraftLine, DraftWord};
use super::super::{Recognition, Timings, WordConfidence};
use super::*;
use crate::store::Store;

/// Open documents: what each page holds of our OCR (with digests, as the
/// shell reads them), and other tools' text for checks.
#[derive(Default)]
struct Host {
    documents: Mutex<HashMap<PathBuf, Vec<LayerState>>>,
    /// Pages written, one entry per edit.
    edits: Mutex<Vec<Vec<(usize, LayerQuality)>>>,
    /// Every layer written, in order.
    written: Mutex<Vec<PageLayer>>,
    events: Mutex<Vec<JobStatus>>,
    texts: Mutex<HashMap<usize, TextInfo>>,
}

impl Host {
    fn open(&self, path: &Path, pages: usize) {
        self.documents.lock().expect("lock").insert(canonical_path(path), vec![LayerState::default(); pages]);
    }

    fn close(&self, path: &Path) {
        self.documents.lock().expect("lock").remove(&canonical_path(path));
    }

    fn layers_of(&self, path: &Path) -> Vec<Option<LayerQuality>> {
        self.states_of(path).iter().map(|s| s.quality).collect()
    }

    fn states_of(&self, path: &Path) -> Vec<LayerState> {
        self.documents.lock().expect("lock").get(&canonical_path(path)).cloned().unwrap_or_default()
    }

    /// Undo: the page's layer is the one with `digest` again.
    fn set_state(&self, path: &Path, page: usize, state: LayerState) {
        if let Some(document) = self.documents.lock().expect("lock").get_mut(&canonical_path(path)) {
            document[page] = state;
        }
    }

    fn edits(&self) -> Vec<Vec<(usize, LayerQuality)>> {
        self.edits.lock().expect("lock").clone()
    }

    fn written(&self) -> Vec<PageLayer> {
        self.written.lock().expect("lock").clone()
    }
}

/// A page 72 × 48 points with two dark text lines, rendered at any scale.
fn page_image(scale: f32) -> Result<OcrImage, String> {
    let (w, h) = ((72.0 * scale).round() as usize, (48.0 * scale).round() as usize);
    let mut rgb = vec![255u8; w * h * 3];
    for top in [12.0f32, 28.8] {
        let (y0, y1) = ((top * scale) as usize, ((top + 3.4) * scale) as usize);
        for y in y0..y1.min(h) {
            for x in (7.2 * scale) as usize..(64.8 * scale) as usize {
                if (x as f32 / scale) % 9.6 < 8.2 {
                    rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
    }
    OcrImage::from_rgb(w as u32, h as u32, rgb).map_err(|e| e.to_string())
}

impl JobHost for Host {
    fn page_count(&self, path: &Path) -> Option<usize> {
        self.documents.lock().expect("lock").get(path).map(Vec::len)
    }

    fn render(&self, _path: &Path, _page: usize, scale: f32) -> Result<RenderedPage, String> {
        Ok(RenderedPage { image: page_image(scale)?, scale, placement: [1.0, 0.0, 0.0, 1.0, 0.0, 842.0] })
    }

    fn layers(&self, path: &Path) -> Option<Vec<Option<LayerQuality>>> {
        Some(self.documents.lock().expect("lock").get(path)?.iter().map(|s| s.quality).collect())
    }

    fn states(&self, path: &Path) -> Option<Vec<LayerState>> {
        self.documents.lock().expect("lock").get(path).cloned()
    }

    fn apply(&self, path: &Path, layers: &[PageLayer]) -> Result<bool, String> {
        let mut documents = self.documents.lock().expect("lock");
        let Some(document) = documents.get_mut(path) else { return Ok(false) };
        for layer in layers {
            document[layer.page] =
                LayerState { quality: Some(layer.quality), digest: Some(layer.digest()), converted: None };
        }
        self.edits.lock().expect("lock").push(layers.iter().map(|l| (l.page, l.quality)).collect());
        self.written.lock().expect("lock").extend(layers.iter().cloned());
        Ok(true)
    }

    fn text_info(&self, _path: &Path, page: usize) -> Result<TextInfo, String> {
        Ok(self.texts.lock().expect("lock").get(&page).cloned().unwrap_or_default())
    }

    fn changed(&self, status: &JobStatus) {
        self.events.lock().expect("lock").push(status.clone());
    }
}

/// Answers every image with `text`; counts reads; can be held until stopped.
struct Scripted {
    name: &'static str,
    text: Mutex<String>,
    /// The probability of every word.
    probability: Mutex<f32>,
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
        let text = self.text.lock().expect("lock").clone();
        let probability = *self.probability.lock().expect("lock");
        let words = text.split_whitespace().map(|w| WordConfidence { word: w.into(), probability }).collect();
        Ok(Recognition {
            text: text.clone(),
            raw: text,
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
                text: Mutex::new(text.into()),
                probability: Mutex::new(0.99),
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

    fn answer(&self, text: &str, probability: f32) {
        *self.primary.text.lock().expect("lock") = text.into();
        *self.primary.probability.lock().expect("lock") = probability;
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
    let dir = std::env::temp_dir().join(format!("mirsam-ocr-jobs-{}", std::process::id()));
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
    let job = runner.submit(&path, "a.pdf", 4, &[2, 0, 1, 9, 0], JobOptions::default()).expect("submits");
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
    let states = host.states_of(&path);
    assert!(
        runner.review(&path, &states, ReviewFilter::default()).expect("review").is_empty(),
        "nothing to review"
    );
    let numbers = runner.review(&path, &states, ReviewFilter { numbers: true }).expect("review");
    assert_eq!(numbers.iter().map(|p| p.page).collect::<Vec<_>>(), [0, 1, 2], "every number, on request");
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
    let job = runner.submit(&path, "b.pdf", 2, &[0, 1], JobOptions::default()).expect("submits");
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
        let job = runner.submit(&path, "c.pdf", 3, &[0, 1, 2], JobOptions::default()).expect("submits");
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
    let job = runner.submit(&path, "d.pdf", 2, &[0, 1], JobOptions::default()).expect("submits");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(state(&runner, job.id), JobState::Queued, "not open yet");
    host.open(&path, 2);
    runner.wake();
    wait_for("done", || state(&runner, job.id) == JobState::Done);

    // A job made for the 2-page document, when the file now open has 5 pages
    // (changed before submitting: a one-page job would otherwise finish first).
    host.close(&path);
    host.open(&path, 5);
    let other = runner.submit(&path, "d.pdf", 2, &[0], JobOptions::default()).expect("submits");
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
        at: None,
        review: WordReview::Open,
    };
    let page = |page: usize, words: Vec<OcrWord>| PageOcr {
        page,
        layer: PageLayer {
            page,
            quality: LayerQuality::Final,
            lines: vec![],
            placement: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            clear: Clear::None,
        },
        words,
        truncated: false,
        engine: "x".into(),
        seconds: 1.0,
        checked: false,
        decisions: Vec::new(),
        digest: None,
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
    let mut list: Vec<ReviewPage> = pages
        .iter()
        .filter_map(|p| {
            let resolved = super::super::review::Resolved {
                job: 1,
                node: None,
                view: p.view(None),
                converted: false,
                stale: false,
            };
            review_page(&resolved, ReviewFilter::default())
        })
        .collect();
    sort_review(&mut list);
    assert_eq!(list.iter().map(|p| p.page).collect::<Vec<_>>(), [2, 1]);
    assert!(list[0].needs_decision() && !list[1].needs_decision());
    assert_eq!((list[0].words.len(), list[0].words[0].index), (1, 0));
}

/// Runs a recognize job over a one-page document and waits for it.
fn recognized(
    name: &str,
    text: &str,
    probability: f32,
) -> (Arc<Store>, Arc<Host>, Engines, JobRunner, PathBuf) {
    let (book, host, engines) =
        (Arc::new(Store::in_memory().expect("store")), Arc::new(Host::default()), Engines::new(text));
    engines.answer(text, probability);
    let path = file(name);
    host.open(&path, 1);
    let runner = JobRunner::start(book.clone(), host.clone(), Box::new(engines.clone())).expect("starts");
    let job = runner.submit(&path, name, 1, &[0], JobOptions::default()).expect("submits");
    wait_for("done", || state(&runner, job.id) == JobState::Done);
    (book, host, engines, runner, path)
}

#[test]
fn review_decisions_write_the_layer_and_follow_undo() {
    let (_book, host, _engines, runner, path) = recognized("e.pdf", "نص الفاتورة Invoice", 0.7);
    let states = host.states_of(&path);
    let recognized = states[0];
    let list = runner.review(&path, &states, ReviewFilter::default()).expect("review");
    assert_eq!(list.len(), 1);
    let flagged: Vec<(&str, &[super::super::review::Reason])> =
        list[0].words.iter().map(|w| (w.text.as_str(), &w.reasons[..])).collect();
    assert_eq!(flagged.len(), 3, "every word under 0.8: {flagged:?}");
    let invoice = list[0].words.iter().find(|w| w.text == "Invoice").expect("listed").index;

    // A correction comes back as a layer to write; the shell writes it.
    let decided = runner
        .decide(&path, &states, 0, Some(invoice), Action::Edit { text: "Invoice:".into() })
        .expect("edit");
    let layer = decided.layer.expect("the text changed");
    assert!(layer.lines.iter().flat_map(|l| &l.words).any(|w| w.text == "Invoice:"));
    assert!(host.apply(&canonical_path(&path), &[layer]).expect("applied"));
    let states = host.states_of(&path);
    let page = runner.review(&path, &states, ReviewFilter::default()).expect("review").remove(0);
    let word = page.words.iter().find(|w| w.index == invoice).expect("still listed");
    assert_eq!(
        (word.text.as_str(), &word.review),
        ("Invoice:", &WordReview::Edited { read: "Invoice".into() })
    );
    assert_eq!(page.open, 2);

    // Undo puts the recognised layer back: the word is open again.
    host.set_state(&path, 0, recognized);
    let page = runner.review(&path, &[recognized], ReviewFilter::default()).expect("review").remove(0);
    let word = page.words.iter().find(|w| w.index == invoice).expect("listed");
    assert_eq!((word.text.as_str(), &word.review), ("Invoice", &WordReview::Open));
    // Accept and check change no text.
    let decided = runner.decide(&path, &[recognized], 0, Some(invoice), Action::Accept).expect("accept");
    assert!(decided.layer.is_none());
    let decided = runner.decide(&path, &[recognized], 0, None, Action::CheckPage).expect("page");
    assert!(decided.view.checked && decided.view.words.iter().all(|w| !w.review.is_open()));
    assert_eq!(
        runner.decide(&path, &[recognized], 0, Some(99), Action::Accept).expect_err("no word"),
        "ocr_review_no_word"
    );
    // A layer the review has no record of is not overwritten.
    let unknown = LayerState { quality: Some(LayerQuality::Final), digest: Some(7), converted: None };
    assert_eq!(
        runner.decide(&path, &[unknown], 0, Some(0), Action::Edit { text: "x".into() }).expect_err("stale"),
        "ocr_review_stale"
    );
}

#[test]
fn an_area_read_again_replaces_its_words_only() {
    let (book, host, engines, runner, path) = recognized("f.pdf", "رقم ١٣٣٤", 0.99);
    let states = host.states_of(&path);
    let before = runner.page_view(&path, &states, 0).expect("view").expect("recognized");
    let number = before.view.words.iter().find(|w| w.text == "١٣٣٤").expect("read").clone();
    let b = number.bounds.expect("placed");
    let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    let rect = [cx - 1.0, cy - 1.0, cx + 1.0, cy + 1.0];

    engines.answer("I234", 0.99);
    let options = JobOptions {
        kind: JobKind::Region { rect, language: Language::Auto, numbers: true },
        replace: false,
    };
    let job = runner.submit(&path, "f.pdf", 1, &[0], options).expect("submits");
    wait_for("done", || {
        state(&runner, job.id) != JobState::Queued && state(&runner, job.id) != JobState::Running
    });
    assert_eq!(state(&runner, job.id), JobState::Done, "{:?}", runner.status(job.id));

    let written = host.written();
    let last = written.last().expect("written");
    assert_eq!(last.clear, Clear::Region { rect });
    let texts: Vec<&str> = last.lines.iter().flat_map(|l| l.words.iter().map(|w| w.text.as_str())).collect();
    assert!(texts.contains(&"رقم") && texts.contains(&"I234") && !texts.contains(&"١٣٣٤"), "{texts:?}");
    // "Numbers only": a reading with a letter in it is flagged, not guessed.
    let states = host.states_of(&path);
    let page = runner.review(&path, &states, ReviewFilter::default()).expect("review").remove(0);
    let flagged = page.words.iter().find(|w| w.text == "I234").expect("flagged");
    assert!(matches!(flagged.status, WordStatus::Uncertain { .. }));
    assert!(page.needs_decision());
    assert!(book.page(job.id, 0).expect("kept").is_some());

    // Undo the area: the page as recognised is what the review shows.
    host.set_state(&path, 0, LayerState { digest: Some(before.view.layer.digest()), ..states[0] });
    let back = runner.page_view(&path, &host.states_of(&path), 0).expect("view").expect("known");
    assert!(back.view.words.iter().any(|w| w.text == "١٣٣٤") && !back.stale);
}

#[test]
fn a_check_finds_a_poor_layer_and_a_replacing_job_clears_it() {
    let (book, host, engines) =
        (Arc::new(Store::in_memory().expect("store")), Arc::new(Host::default()), Engines::new("نص عربي"));
    let path = file("g.pdf");
    host.open(&path, 3);
    let poor = TextInfo {
        text: "ÚÞÏ ÇáÅíÌÇÑ Èíä ÇáØÑÝíä Ýí ãÏíäÉ ÇáÑíÇÖ".into(),
        invisible: 30,
        ..TextInfo::default()
    };
    host.texts.lock().expect("lock").insert(0, poor);
    host.texts
        .lock()
        .expect("lock")
        .insert(1, TextInfo { ours: true, text: "نص".into(), ..TextInfo::default() });
    let runner = JobRunner::start(book, host.clone(), Box::new(engines)).expect("starts");
    let check = JobOptions { kind: JobKind::Check, replace: false };
    let job = runner.submit(&path, "g.pdf", 3, &[0, 1, 2], check).expect("submits");
    wait_for("checked", || state(&runner, job.id) == JobState::Done);
    assert!(host.edits().is_empty(), "a check writes nothing");
    let checks = runner.checks(&path).expect("checks");
    let verdicts: Vec<(usize, Verdict)> = checks.iter().map(|c| (c.page, c.verdict)).collect();
    assert_eq!(verdicts, [(0, Verdict::Poor), (1, Verdict::NotChecked), (2, Verdict::NotChecked)]);
    assert!(checks[0].problems.contains(&super::super::check::Problem::Mojibake));
    assert_eq!(runner.status(job.id).map(|s| (s.finished, s.options.kind)), Some((3, JobKind::Check)));
    // A check job's verdicts are not review results.
    assert!(
        runner
            .review(&path, &host.states_of(&path), ReviewFilter { numbers: true })
            .expect("review")
            .is_empty()
    );

    let replace = JobOptions { kind: JobKind::Recognize, replace: true };
    let job = runner.submit(&path, "g.pdf", 3, &[0], replace).expect("submits");
    wait_for("replaced", || state(&runner, job.id) == JobState::Done);
    let written = host.written();
    assert_eq!(
        written.iter().map(|l| (l.quality, l.clear)).collect::<Vec<_>>(),
        [(LayerQuality::Draft, Clear::Page), (LayerQuality::Final, Clear::Page)]
    );
}
