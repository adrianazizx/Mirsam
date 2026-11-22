//! Scheduler behaviour with a fake page source: ordering, generation-based
//! cancellation, coalescing, caching, eviction, page reuse, and shutdown.
//! No PDFium and no webview involved.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::*;
use crate::error::Result;

const TIMEOUT: Duration = Duration::from_secs(5);
const SIDE: u32 = 16;
/// Cache weight of one fake tile (pixels plus the fixed per-entry overhead).
const TILE_WEIGHT: usize = (SIDE * SIDE * 4) as usize + 128;

/// A source whose renders can be held at a gate, that records the order in
/// which tiles were rendered, and that fails on request.
struct Fake {
    gate: Mutex<bool>,
    gate_changed: Condvar,
    started: Mutex<Option<Sender<usize>>>,
    rendered: Mutex<Vec<(usize, u32)>>,
    opens: AtomicUsize,
    failing_page: Option<usize>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Self::with_failing_page(None)
    }

    fn with_failing_page(failing_page: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            gate: Mutex::new(true),
            gate_changed: Condvar::new(),
            started: Mutex::new(None),
            rendered: Mutex::new(Vec::new()),
            opens: AtomicUsize::new(0),
            failing_page,
        })
    }

    /// Closes the gate and returns a receiver told the page of each render as it starts.
    fn hold(&self) -> Receiver<usize> {
        *self.gate.lock().expect("gate") = false;
        let (tx, rx) = channel();
        *self.started.lock().expect("started") = Some(tx);
        rx
    }

    fn release(&self) {
        *self.gate.lock().expect("gate") = true;
        self.gate_changed.notify_all();
    }

    /// (page, x) of every tile rendered so far, in order.
    fn rendered(&self) -> Vec<(usize, u32)> {
        self.rendered.lock().expect("log").clone()
    }
}

struct FakePage {
    source: Arc<Fake>,
    index: usize,
}

impl TileSource for Fake {
    fn open_page(self: Arc<Self>, index: usize) -> Result<Box<dyn TilePage>> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakePage { source: self, index }))
    }
}

impl TilePage for FakePage {
    fn render_tile(&self, tile: TileRequest) -> Result<crate::Bitmap> {
        if let Some(started) = self.source.started.lock().expect("started").as_ref() {
            let _ = started.send(self.index);
        }
        let mut open = self.source.gate.lock().expect("gate");
        while !*open {
            open = self.source.gate_changed.wait(open).expect("gate");
        }
        drop(open);
        self.source.rendered.lock().expect("log").push((self.index, tile.x));
        if self.source.failing_page == Some(self.index) {
            return Err(crate::Error::InvalidRenderRequest("fake failure"));
        }
        let pixels = vec![self.index as u8; (tile.width * tile.height * 4) as usize];
        crate::Bitmap::from_rgba(tile.width, tile.height, pixels)
            .ok_or(crate::Error::InvalidRenderRequest("bad fake bitmap"))
    }
}

const DOC: DocumentId = 1;

fn tile(x: u32) -> TileRequest {
    TileRequest { scale: 1.0, x, y: 0, width: SIDE, height: SIDE }
}

fn request(page: usize, x: u32, generation: u64, urgency: Urgency, distance: f32) -> RenderRequest {
    RenderRequest {
        document: DOC,
        generation,
        page,
        tile: tile(x),
        priority: Priority::new(urgency, distance),
    }
}

/// Outcomes arrive tagged with a label so tests can tell requests apart.
struct Outcomes {
    tx: Sender<(&'static str, RenderOutcome)>,
    rx: Receiver<(&'static str, RenderOutcome)>,
}

impl Outcomes {
    fn new() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }

    fn callback(&self, label: &'static str) -> impl FnOnce(RenderOutcome) + Send + 'static {
        let tx = self.tx.clone();
        move |outcome| {
            let _ = tx.send((label, outcome));
        }
    }

    fn next(&self) -> (&'static str, RenderOutcome) {
        self.rx.recv_timeout(TIMEOUT).expect("an outcome arrives")
    }

    /// Collects `n` outcomes keyed by label.
    fn take(&self, n: usize) -> HashMap<&'static str, RenderOutcome> {
        (0..n).map(|_| self.next()).collect()
    }

    fn none_pending(&self) -> bool {
        self.rx.recv_timeout(Duration::from_millis(50)).is_err()
    }
}

fn service_with(fake: &Arc<Fake>, config: RenderConfig) -> RenderService {
    let service = RenderService::new(config);
    service.open_document(DOC, Arc::clone(fake) as Arc<dyn TileSource>);
    service
}

fn service(fake: &Arc<Fake>) -> RenderService {
    service_with(fake, RenderConfig::default())
}

/// Submits a request that occupies the render thread until `fake.release()`.
fn block_worker(service: &RenderService, fake: &Fake, outcomes: &Outcomes) {
    let started = fake.hold();
    service.submit(request(99, 0, 0, Urgency::Visible, 0.0), outcomes.callback("blocker"));
    assert_eq!(started.recv_timeout(TIMEOUT).expect("blocker starts"), 99);
}

fn is_ok(outcome: &RenderOutcome) -> bool {
    outcome.is_ok()
}

fn is_cancelled(outcome: &RenderOutcome) -> bool {
    matches!(outcome, Err(RenderError::Cancelled))
}

#[test]
fn renders_placeholders_then_visible_then_prefetch_nearest_first() {
    let fake = Fake::new();
    let service = service(&fake);
    let outcomes = Outcomes::new();
    block_worker(&service, &fake, &outcomes);

    service.submit(request(1, 0, 0, Urgency::Prefetch, 0.0), outcomes.callback("prefetch"));
    service.submit(request(2, 0, 0, Urgency::Visible, 50.0), outcomes.callback("visible far"));
    service.submit(request(3, 0, 0, Urgency::Visible, 10.0), outcomes.callback("visible near"));
    service.submit(request(4, 0, 0, Urgency::Placeholder, 500.0), outcomes.callback("placeholder"));
    // Same priority as "visible near": arrival order breaks the tie.
    service.submit(request(5, 0, 0, Urgency::Visible, 10.0), outcomes.callback("visible near, later"));
    fake.release();

    let all = outcomes.take(6);
    assert!(all.values().all(is_ok));
    let pages: Vec<usize> = fake.rendered().iter().map(|(page, _)| *page).collect();
    assert_eq!(pages, vec![99, 4, 3, 5, 2, 1]);
}

#[test]
fn a_viewport_change_drops_unstarted_requests_it_does_not_keep() {
    let fake = Fake::new();
    let service = service(&fake);
    let outcomes = Outcomes::new();
    block_worker(&service, &fake, &outcomes);

    service.submit(request(1, 0, 1, Urgency::Visible, 0.0), outcomes.callback("a"));
    service.submit(request(2, 0, 1, Urgency::Visible, 0.0), outcomes.callback("b"));
    service.submit(request(3, 0, 1, Urgency::Visible, 0.0), outcomes.callback("c"));
    let keep_b = KeptRequest { page: 2, tile: tile(0), priority: Priority::new(Urgency::Prefetch, 0.0) };
    service.update_viewport(DOC, 2, &[keep_b]);

    // Dropped requests are answered at once, before anything else renders.
    let dropped = outcomes.take(2);
    assert!(is_cancelled(&dropped["a"]) && is_cancelled(&dropped["c"]), "{dropped:?}");

    // A request from the superseded generation is cancelled on arrival.
    service.submit(request(4, 0, 1, Urgency::Visible, 0.0), outcomes.callback("late"));
    assert!(is_cancelled(&outcomes.next().1));

    service.submit(request(5, 0, 2, Urgency::Visible, 0.0), outcomes.callback("d"));
    fake.release();
    let finished = outcomes.take(3);
    // The blocker had started before the viewport moved, so it completes.
    assert!(is_ok(&finished["blocker"]) && is_ok(&finished["b"]) && is_ok(&finished["d"]));
    // `b` was demoted to prefetch by the update, so the visible `d` goes first.
    let pages: Vec<usize> = fake.rendered().iter().map(|(page, _)| *page).collect();
    assert_eq!(pages, vec![99, 5, 2]);
    assert_eq!(service.stats().cancelled, 3);
}

#[test]
fn viewport_updates_ignore_stale_generations_and_spare_newer_requests() {
    let fake = Fake::new();
    let service = service(&fake);
    let outcomes = Outcomes::new();
    block_worker(&service, &fake, &outcomes);

    // Submitted with generation 5 before the update for generation 3 arrived
    // (IPC calls may be handled out of order): it must survive that update.
    service.submit(request(1, 0, 5, Urgency::Visible, 0.0), outcomes.callback("newer"));
    service.update_viewport(DOC, 3, &[]);
    service.update_viewport(DOC, 2, &[]);
    assert!(outcomes.none_pending());
    service.update_viewport(DOC, 6, &[]);
    assert!(is_cancelled(&outcomes.next().1));
    fake.release();
    assert!(is_ok(&outcomes.next().1));
}

#[test]
fn identical_requests_share_one_render_and_later_ones_hit_the_cache() {
    let fake = Fake::new();
    let service = service(&fake);
    let outcomes = Outcomes::new();
    block_worker(&service, &fake, &outcomes);

    service.submit(request(1, 0, 0, Urgency::Prefetch, 9.0), outcomes.callback("first"));
    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("second"));
    // Joins the render already running.
    service.submit(request(99, 0, 0, Urgency::Visible, 0.0), outcomes.callback("blocker again"));
    fake.release();
    let done = outcomes.take(4);
    for label in ["first", "second", "blocker", "blocker again"] {
        let rendered = done[label].as_ref().expect("rendered");
        assert!(!rendered.from_cache, "{label}");
    }
    assert_eq!(fake.rendered(), vec![(99, 0), (1, 0)]);

    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("again"));
    let (_, again) = outcomes.next();
    let again = again.expect("cached");
    assert!(again.from_cache);
    assert_eq!(again.bitmap.pixel(0, 0), Some([1, 1, 1, 1]));
    let stats = service.stats();
    assert_eq!((stats.rendered, stats.coalesced, stats.cache_hits), (2, 2, 1));
    assert_eq!(stats.cache_entries, 2);
}

#[test]
fn the_cache_evicts_least_recently_used_tiles_at_its_byte_cap() {
    let fake = Fake::new();
    let config = RenderConfig { cache_bytes: 2 * TILE_WEIGHT, ..RenderConfig::default() };
    let service = service_with(&fake, config);
    let outcomes = Outcomes::new();
    let render = |x: u32| {
        service.submit(request(1, x, 0, Urgency::Visible, 0.0), outcomes.callback("tile"));
        outcomes.next().1.expect("renders")
    };

    for x in [0, 16, 32] {
        assert!(!render(x).from_cache);
    }
    // Capacity is two tiles: the oldest (x = 0) went to make room for x = 32.
    assert!(render(16).from_cache);
    // x = 16 was just touched, so making room for x = 0 evicted x = 32.
    assert!(!render(0).from_cache);
    assert!(render(16).from_cache);
    assert!(!render(32).from_cache);
    let stats = service.stats();
    assert_eq!(stats.cache_evictions, 3);
    assert!(stats.cache_bytes <= stats.cache_capacity_bytes);
}

#[test]
fn keeps_recent_pages_open_between_tiles() {
    let fake = Fake::new();
    let config = RenderConfig { loaded_pages: 2, ..RenderConfig::default() };
    let service = service_with(&fake, config);
    let outcomes = Outcomes::new();
    let render = |page: usize, x: u32| {
        service.submit(request(page, x, 0, Urgency::Visible, 0.0), outcomes.callback("tile"));
        outcomes.next().1.expect("renders");
    };

    for x in [0, 16, 32, 48] {
        render(1, x);
    }
    assert_eq!(fake.opens.load(Ordering::SeqCst), 1);
    render(2, 0);
    render(1, 64);
    assert_eq!(fake.opens.load(Ordering::SeqCst), 2);
    // A third page pushes out the least recently used one (page 2).
    render(3, 0);
    render(2, 16);
    assert_eq!(fake.opens.load(Ordering::SeqCst), 4);
    assert_eq!(service.open_pages(), 2);
    assert_eq!(service.stats().page_loads, 4);
}

#[test]
fn closing_a_document_cancels_its_queue_and_drops_its_tiles_and_pages() {
    let fake = Fake::new();
    let service = service(&fake);
    let outcomes = Outcomes::new();
    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("cached"));
    outcomes.next().1.expect("renders");
    block_worker(&service, &fake, &outcomes);
    service.submit(request(2, 0, 0, Urgency::Visible, 0.0), outcomes.callback("queued"));

    service.close_document(DOC);
    assert!(is_cancelled(&outcomes.next().1));
    assert_eq!(service.stats().cache_entries, 0);
    fake.release();
    // The running render still answers its caller, but is not cached.
    assert!(is_ok(&outcomes.next().1));
    assert_eq!(service.stats().cache_entries, 0);
    assert_eq!(service.open_pages(), 0);

    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("after close"));
    assert!(matches!(outcomes.next().1, Err(RenderError::UnknownDocument(DOC))));
}

#[test]
fn invalidating_a_document_rerenders_instead_of_serving_stale_tiles() {
    let fake = Fake::new();
    let service = service(&fake);
    let outcomes = Outcomes::new();
    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("before"));
    outcomes.next().1.expect("renders");

    block_worker(&service, &fake, &outcomes);
    service.submit(request(2, 0, 0, Urgency::Visible, 0.0), outcomes.callback("queued"));
    service.invalidate_document(DOC);
    assert!(is_cancelled(&outcomes.next().1));
    fake.release();
    assert!(is_ok(&outcomes.next().1));

    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("after"));
    assert!(!outcomes.next().1.expect("renders").from_cache);
    assert_eq!(fake.rendered().iter().filter(|(page, _)| *page == 1).count(), 2);
}

#[test]
fn engine_errors_reach_every_waiter_and_are_not_cached() {
    let fake = Fake::with_failing_page(Some(7));
    let service = service(&fake);
    let outcomes = Outcomes::new();
    block_worker(&service, &fake, &outcomes);
    service.submit(request(7, 0, 0, Urgency::Visible, 0.0), outcomes.callback("one"));
    service.submit(request(7, 0, 0, Urgency::Visible, 0.0), outcomes.callback("two"));
    fake.release();
    let done = outcomes.take(3);
    assert!(matches!(done["one"], Err(RenderError::Engine(_))));
    assert!(matches!(done["two"], Err(RenderError::Engine(_))));

    service.submit(request(7, 0, 0, Urgency::Visible, 0.0), outcomes.callback("retry"));
    assert!(matches!(outcomes.next().1, Err(RenderError::Engine(_))));
    assert_eq!(service.stats().failed, 2);
}

#[test]
fn the_queue_is_bounded() {
    let fake = Fake::new();
    let config = RenderConfig { max_pending: 2, ..RenderConfig::default() };
    let service = service_with(&fake, config);
    let outcomes = Outcomes::new();
    block_worker(&service, &fake, &outcomes);
    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("a"));
    service.submit(request(2, 0, 0, Urgency::Visible, 0.0), outcomes.callback("b"));
    service.submit(request(3, 0, 0, Urgency::Visible, 0.0), outcomes.callback("over"));
    let (label, outcome) = outcomes.next();
    assert_eq!(label, "over");
    assert!(is_cancelled(&outcome));
    fake.release();
    assert!(outcomes.take(3).values().all(is_ok));
}

#[test]
fn dropping_the_service_cancels_queued_work_and_finishes_the_running_render() {
    let fake = Fake::new();
    let service = service(&fake);
    let outcomes = Outcomes::new();
    block_worker(&service, &fake, &outcomes);
    service.submit(request(1, 0, 0, Urgency::Visible, 0.0), outcomes.callback("queued"));

    let releaser = {
        let fake = Arc::clone(&fake);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            fake.release();
        })
    };
    drop(service);
    releaser.join().expect("releaser");
    let done = outcomes.take(2);
    assert!(is_ok(&done["blocker"]));
    assert!(is_cancelled(&done["queued"]));
}

#[test]
fn unknown_documents_are_reported() {
    let service = RenderService::default();
    let outcomes = Outcomes::new();
    service.submit(request(0, 0, 0, Urgency::Visible, 0.0), outcomes.callback("x"));
    assert!(matches!(outcomes.next().1, Err(RenderError::UnknownDocument(DOC))));
}
