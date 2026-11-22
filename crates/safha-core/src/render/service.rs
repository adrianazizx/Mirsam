//! The render thread and its queue. See the module docs in `render/mod.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use serde::Serialize;

use super::cache::{LruCache, Weighed};
use super::source::{TilePage, TileSource};
use super::{
    DocumentId, KeptRequest, Priority, RenderError, RenderOutcome, RenderRequest, Rendered, TileKey,
};
use crate::document::TileRequest;

/// Tuning knobs for a [`RenderService`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderConfig {
    /// Upper bound on the pixel memory the tile cache keeps, in bytes.
    pub cache_bytes: usize,
    /// How many parsed pages the render thread keeps open between requests.
    pub loaded_pages: usize,
    /// Requests waiting beyond this many are cancelled on arrival, so a runaway
    /// caller cannot grow the queue without bound.
    pub max_pending: usize,
}

impl RenderConfig {
    /// 64 MiB holds about 60 tiles of 512 x 512 RGBA, or 130 previews: nearly
    /// two screens of a full-screen HiDPI viewport (~35 tiles plus nearby
    /// previews), so scrolling back and re-zooming hit the cache. Measured with
    /// `examples/render_bench.rs` on the 1,000-page fixture, resident memory
    /// once the cache churns: 16 MiB -> 62 MB, 64 MiB -> 96 MB, but 128 MiB ->
    /// ~300 MB and 256 MiB -> ~475 MB (the allocator stops reusing freed
    /// bitmaps well beyond 64 MiB). The process budget is 500 MB (spec section 11).
    pub const DEFAULT_CACHE_BYTES: usize = 64 * 1024 * 1024;
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self { cache_bytes: Self::DEFAULT_CACHE_BYTES, loaded_pages: 4, max_pending: 4096 }
    }
}

/// Counters for diagnostics and benchmarks. All counts are since start-up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderStats {
    pub submitted: u64,
    pub cache_hits: u64,
    /// Requests that joined an identical render already queued or running.
    pub coalesced: u64,
    pub rendered: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub render_micros: u64,
    pub page_loads: u64,
    pub cache_bytes: u64,
    pub cache_capacity_bytes: u64,
    pub cache_entries: u64,
    pub cache_evictions: u64,
    pub pending: u64,
}

type Waiter = Box<dyn FnOnce(RenderOutcome) + Send>;

/// Identity of a render: also its cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct JobKey {
    document: DocumentId,
    revision: u64,
    tile: TileKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PageKey {
    document: DocumentId,
    revision: u64,
    page: usize,
}

struct Job {
    page: usize,
    tile: TileRequest,
    priority: Priority,
    generation: u64,
    /// Arrival order, the last tie-breaker.
    seq: u64,
    waiters: Vec<Waiter>,
}

struct DocState {
    source: Arc<dyn TileSource>,
    /// Bumped when the document's content changes; part of every cache key.
    revision: u64,
    /// The newest viewport generation [`RenderService::update_viewport`] has seen.
    generation: u64,
}

#[derive(Clone)]
struct CachedTile {
    bitmap: Arc<crate::Bitmap>,
    render_micros: u32,
}

impl Weighed for CachedTile {
    fn weight(&self) -> usize {
        // Pixels dominate; the constant covers the key, the map slots, and the Arc.
        self.bitmap.as_bgra().len() + 128
    }
}

struct State {
    documents: HashMap<DocumentId, DocState>,
    pending: HashMap<JobKey, Job>,
    /// The job the render thread is working on, and everyone waiting for it.
    running: Option<(JobKey, Vec<Waiter>)>,
    cache: LruCache<JobKey, CachedTile>,
    /// Parsed pages kept open, least recently used first.
    pages: Vec<(PageKey, Box<dyn TilePage>)>,
    next_seq: u64,
    shutdown: bool,
    stats: RenderStats,
    config: RenderConfig,
}

/// Callbacks to run once the lock is released, plus pages to close outside it.
#[derive(Default)]
struct Deferred {
    completions: Vec<(Waiter, RenderOutcome)>,
    closed_pages: Vec<Box<dyn TilePage>>,
}

impl Deferred {
    fn cancel(&mut self, waiters: Vec<Waiter>) {
        self.completions.extend(waiters.into_iter().map(|waiter| (waiter, Err(RenderError::Cancelled))));
    }

    fn run(self) {
        for (waiter, outcome) in self.completions {
            waiter(outcome);
        }
        drop(self.closed_pages);
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One render thread with a prioritized, cancellable queue and a tile cache.
///
/// Every method is cheap and non-blocking for the caller: results arrive
/// through the callback passed to [`submit`](Self::submit), on the render
/// thread (or, for cache hits and cancellations, on the calling thread).
/// Callbacks must not block.
pub struct RenderService {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl RenderService {
    pub fn new(config: RenderConfig) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                documents: HashMap::new(),
                pending: HashMap::new(),
                running: None,
                cache: LruCache::new(config.cache_bytes),
                pages: Vec::new(),
                next_seq: 0,
                shutdown: false,
                stats: RenderStats::default(),
                config,
            }),
            wake: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        // If the thread cannot be spawned the service still answers every
        // request (with `Cancelled`) rather than hanging its callers.
        let worker = std::thread::Builder::new()
            .name("safha-render".into())
            .spawn(move || worker_loop(&worker_shared))
            .ok();
        if worker.is_none() {
            shared.lock().shutdown = true;
        }
        Self { shared, worker }
    }

    /// Makes a document renderable under `id`. Re-registering an id replaces
    /// the previous document (and drops everything cached for it).
    pub fn open_document(&self, id: DocumentId, source: Arc<dyn TileSource>) {
        let mut deferred = Deferred::default();
        {
            let mut state = self.shared.lock();
            state.forget(id, &mut deferred);
            state.documents.insert(id, DocState { source, revision: 0, generation: 0 });
        }
        deferred.run();
    }

    /// Cancels the document's queued requests and drops its cached tiles and
    /// open pages. A render already running finishes but is not cached.
    pub fn close_document(&self, id: DocumentId) {
        let mut deferred = Deferred::default();
        self.shared.lock().forget(id, &mut deferred);
        deferred.run();
    }

    /// Replaces a document with another revision of it (after an edit, an
    /// undo, or a redo). Everything rendered, queued, or open for the old
    /// revision is discarded, and a render of it still running is not cached.
    /// Viewport generations carry on, so the viewer's requests stay valid.
    pub fn replace_document(&self, id: DocumentId, source: Arc<dyn TileSource>) {
        let mut deferred = Deferred::default();
        {
            let mut state = self.shared.lock();
            let Some(doc) = state.documents.get_mut(&id) else { return };
            doc.revision += 1;
            doc.source = source;
            let revision = doc.revision;
            state.drop_document_work(id, |old| old < revision, &mut deferred);
        }
        deferred.run();
    }

    /// Declares that the document's content changed (an edit): everything
    /// rendered or queued for the old content is discarded.
    pub fn invalidate_document(&self, id: DocumentId) {
        let mut deferred = Deferred::default();
        {
            let mut state = self.shared.lock();
            let Some(doc) = state.documents.get_mut(&id) else { return };
            doc.revision += 1;
            let revision = doc.revision;
            state.drop_document_work(id, |old| old < revision, &mut deferred);
        }
        deferred.run();
    }

    /// Queues a render. `done` is called exactly once with the bitmap or the
    /// reason there is none.
    pub fn submit(&self, request: RenderRequest, done: impl FnOnce(RenderOutcome) + Send + 'static) {
        let done: Waiter = Box::new(done);
        let mut deferred = Deferred::default();
        {
            let mut state = self.shared.lock();
            state.stats.submitted += 1;
            state.enqueue(request, done, &mut deferred);
        }
        self.shared.wake.notify_one();
        deferred.run();
    }

    /// Moves the document's viewport to `generation`. Queued requests named in
    /// `keep` survive with their new priorities; every other queued request
    /// from an older generation is cancelled. Requests already submitted with
    /// `generation` or newer are untouched. A stale call (older generation than
    /// one already seen) does nothing.
    pub fn update_viewport(&self, id: DocumentId, generation: u64, keep: &[KeptRequest]) {
        let mut deferred = Deferred::default();
        {
            let mut state = self.shared.lock();
            let Some(doc) = state.documents.get_mut(&id) else { return };
            if generation < doc.generation {
                return;
            }
            doc.generation = generation;
            let revision = doc.revision;
            let kept: HashMap<JobKey, Priority> = keep
                .iter()
                .map(|kept| {
                    let key = JobKey { document: id, revision, tile: TileKey::new(kept.page, &kept.tile) };
                    (key, kept.priority)
                })
                .collect();
            let mut doomed = Vec::new();
            for (key, job) in &mut state.pending {
                if key.document != id {
                    continue;
                }
                if let Some(priority) = kept.get(key) {
                    job.priority = *priority;
                    job.generation = job.generation.max(generation);
                } else if job.generation < generation {
                    doomed.push(*key);
                }
            }
            for key in doomed {
                if let Some(job) = state.pending.remove(&key) {
                    state.stats.cancelled += job.waiters.len() as u64;
                    deferred.cancel(job.waiters);
                }
            }
        }
        deferred.run();
    }

    pub fn stats(&self) -> RenderStats {
        let state = self.shared.lock();
        RenderStats {
            cache_bytes: state.cache.used() as u64,
            cache_capacity_bytes: state.cache.capacity() as u64,
            cache_entries: state.cache.len() as u64,
            cache_evictions: state.cache.evictions(),
            pending: state.pending.len() as u64,
            ..state.stats
        }
    }
}

impl Default for RenderService {
    fn default() -> Self {
        Self::new(RenderConfig::default())
    }
}

impl Drop for RenderService {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // Anything still queued (the worker drains on exit; this covers a
        // worker that never started).
        let mut deferred = Deferred::default();
        {
            let mut state = self.shared.lock();
            let jobs: Vec<Job> = state.pending.drain().map(|(_, job)| job).collect();
            for job in jobs {
                deferred.cancel(job.waiters);
            }
            deferred.closed_pages.extend(state.pages.drain(..).map(|(_, page)| page));
        }
        deferred.run();
    }
}

impl std::fmt::Debug for RenderService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderService").field("stats", &self.stats()).finish()
    }
}

impl State {
    fn enqueue(&mut self, request: RenderRequest, done: Waiter, deferred: &mut Deferred) {
        let Some(doc) = self.documents.get(&request.document) else {
            deferred.completions.push((done, Err(RenderError::UnknownDocument(request.document))));
            return;
        };
        if self.shutdown || request.generation < doc.generation {
            self.stats.cancelled += 1;
            deferred.cancel(vec![done]);
            return;
        }
        let key = JobKey {
            document: request.document,
            revision: doc.revision,
            tile: TileKey::new(request.page, &request.tile),
        };

        if let Some(hit) = self.cache.get(&key) {
            self.stats.cache_hits += 1;
            let rendered =
                Rendered { bitmap: hit.bitmap, render_micros: hit.render_micros, from_cache: true };
            deferred.completions.push((done, Ok(rendered)));
            return;
        }
        if let Some((running, waiters)) = &mut self.running
            && *running == key
        {
            self.stats.coalesced += 1;
            waiters.push(done);
            return;
        }
        if let Some(job) = self.pending.get_mut(&key) {
            self.stats.coalesced += 1;
            job.priority = job.priority.max_urgency(request.priority);
            job.generation = job.generation.max(request.generation);
            job.waiters.push(done);
            return;
        }
        if self.pending.len() >= self.config.max_pending {
            self.stats.cancelled += 1;
            deferred.cancel(vec![done]);
            return;
        }
        self.next_seq += 1;
        let job = Job {
            page: request.page,
            tile: request.tile,
            priority: request.priority,
            generation: request.generation,
            seq: self.next_seq,
            waiters: vec![done],
        };
        self.pending.insert(key, job);
    }

    /// The most urgent queued job. A linear scan: the queue holds at most a few
    /// hundred entries in practice, and one render costs far more than the scan.
    fn best_job(&self) -> Option<JobKey> {
        self.pending
            .iter()
            .min_by(|(_, a), (_, b)| a.priority.cmp_key(&b.priority).then(a.seq.cmp(&b.seq)))
            .map(|(key, _)| *key)
    }

    /// Cancels and drops the document's queued work, cached tiles, and open
    /// pages whose revision satisfies `doomed_revision`.
    fn drop_document_work(
        &mut self,
        id: DocumentId,
        doomed_revision: impl Fn(u64) -> bool,
        deferred: &mut Deferred,
    ) {
        let doomed: Vec<JobKey> = self
            .pending
            .keys()
            .filter(|key| key.document == id && doomed_revision(key.revision))
            .copied()
            .collect();
        for key in doomed {
            if let Some(job) = self.pending.remove(&key) {
                self.stats.cancelled += job.waiters.len() as u64;
                deferred.cancel(job.waiters);
            }
        }
        self.cache.retain(|key| !(key.document == id && doomed_revision(key.revision)));
        let (closing, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pages)
            .into_iter()
            .partition(|(page, _)| page.document == id && doomed_revision(page.revision));
        self.pages = open;
        deferred.closed_pages.extend(closing.into_iter().map(|(_, page)| page));
    }

    fn forget(&mut self, id: DocumentId, deferred: &mut Deferred) {
        self.drop_document_work(id, |_| true, deferred);
        self.documents.remove(&id);
    }

    fn take_page(&mut self, key: PageKey) -> Option<Box<dyn TilePage>> {
        let index = self.pages.iter().position(|(page, _)| *page == key)?;
        Some(self.pages.remove(index).1)
    }

    /// Returns an open page to the pool as the most recently used.
    fn return_page(&mut self, key: PageKey, page: Box<dyn TilePage>, deferred: &mut Deferred) {
        let current = self.documents.get(&key.document).is_some_and(|doc| doc.revision == key.revision);
        if !current || self.config.loaded_pages == 0 {
            deferred.closed_pages.push(page);
            return;
        }
        self.pages.push((key, page));
        while self.pages.len() > self.config.loaded_pages {
            let (_, oldest) = self.pages.remove(0);
            deferred.closed_pages.push(oldest);
        }
    }
}

/// Work taken off the queue, to be rendered without holding the lock.
struct Claimed {
    key: JobKey,
    page_key: PageKey,
    tile: TileRequest,
    open_page: Option<Box<dyn TilePage>>,
    source: Arc<dyn TileSource>,
}

fn claim_next(state: &mut State, deferred: &mut Deferred) -> Option<Claimed> {
    loop {
        let key = state.best_job()?;
        let job = state.pending.remove(&key)?;
        let Some(doc) = state.documents.get(&key.document) else {
            // Unreachable in practice (closing a document drops its jobs), but
            // every caller is still owed an answer.
            state.stats.cancelled += job.waiters.len() as u64;
            deferred.cancel(job.waiters);
            continue;
        };
        let source = Arc::clone(&doc.source);
        let page_key = PageKey { document: key.document, revision: key.revision, page: job.page };
        let open_page = state.take_page(page_key);
        state.running = Some((key, job.waiters));
        return Some(Claimed { key, page_key, tile: job.tile, open_page, source });
    }
}

fn worker_loop(shared: &Shared) {
    loop {
        let mut orphans = Deferred::default();
        let claimed = {
            let mut state = shared.lock();
            loop {
                if state.shutdown {
                    drain_on_shutdown(state);
                    orphans.run();
                    return;
                }
                if let Some(claimed) = claim_next(&mut state, &mut orphans) {
                    break claimed;
                }
                state = shared.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
        };
        orphans.run();

        let started = Instant::now();
        let loaded_fresh = claimed.open_page.is_none();
        let page = match claimed.open_page {
            Some(page) => Ok(page),
            None => Arc::clone(&claimed.source).open_page(claimed.page_key.page),
        };
        let result = page.and_then(|page| page.render_tile(claimed.tile).map(|bitmap| (page, bitmap)));
        let render_micros = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);

        let mut deferred = Deferred::default();
        {
            let mut state = shared.lock();
            let waiters = state.running.take().map(|(_, waiters)| waiters).unwrap_or_default();
            if loaded_fresh {
                state.stats.page_loads += 1;
            }
            state.stats.render_micros += u64::from(render_micros);
            match result {
                Ok((page, bitmap)) => {
                    state.stats.rendered += 1;
                    state.return_page(claimed.page_key, page, &mut deferred);
                    let tile = CachedTile { bitmap: Arc::new(bitmap), render_micros };
                    let current = state
                        .documents
                        .get(&claimed.key.document)
                        .is_some_and(|doc| doc.revision == claimed.key.revision);
                    if current {
                        state.cache.insert(claimed.key, tile.clone());
                    }
                    let rendered = Rendered {
                        bitmap: tile.bitmap,
                        render_micros: tile.render_micros,
                        from_cache: false,
                    };
                    deferred
                        .completions
                        .extend(waiters.into_iter().map(|waiter| (waiter, Ok(rendered.clone()))));
                }
                Err(error) => {
                    state.stats.failed += 1;
                    let error = Arc::new(error);
                    deferred.completions.extend(
                        waiters
                            .into_iter()
                            .map(|waiter| (waiter, Err(RenderError::Engine(Arc::clone(&error))))),
                    );
                }
            }
        }
        deferred.run();
    }
}

fn drain_on_shutdown(mut state: MutexGuard<'_, State>) {
    let mut deferred = Deferred::default();
    let jobs: Vec<Job> = state.pending.drain().map(|(_, job)| job).collect();
    for job in jobs {
        deferred.cancel(job.waiters);
    }
    deferred.closed_pages.extend(state.pages.drain(..).map(|(_, page)| page));
    drop(state);
    deferred.run();
}

#[cfg(test)]
impl RenderService {
    /// Number of cached parsed pages, for tests.
    pub(crate) fn open_pages(&self) -> usize {
        self.shared.lock().pages.len()
    }
}
