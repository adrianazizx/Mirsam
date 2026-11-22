use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use safha_core::Document;
use safha_core::render::{DocumentId, RenderConfig, RenderService};
use safha_core::search::IndexCache;
use safha_core::store::Store;
use serde::Deserialize;

/// The render-queue lane a request belongs to. Each lane is registered with
/// the render service as its own document, so it has its own viewport
/// generation: scrolling the thumbnail panel never cancels the page view's
/// tiles, and the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Lane {
    #[default]
    Main,
    Thumbnails,
}

impl Lane {
    pub fn render_id(self, document: u32) -> DocumentId {
        match self {
            Lane::Main => DocumentId::from(document),
            Lane::Thumbnails => (1 << 32) | DocumentId::from(document),
        }
    }
}

/// A document open in a tab.
pub struct OpenFile {
    pub document: Arc<Document>,
    /// Never sent to the UI (ADR 0001); used for recents and per-file state.
    pub path: PathBuf,
    pub name: String,
    /// The search index, built by the first search.
    pub index: Arc<IndexCache>,
}

pub struct AppState {
    /// Process start, for the self-test's cold-start timing.
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    pub started: Instant,
    startup_document: Mutex<Option<PathBuf>>,
    started_up: AtomicBool,
    documents: Mutex<HashMap<u32, OpenFile>>,
    /// Files waiting for a password, by token.
    locked: Mutex<HashMap<u32, PathBuf>>,
    /// Recent files from the last listing, by token: the UI names a recent
    /// file by token, never by path.
    recents: Mutex<HashMap<u32, PathBuf>>,
    next_id: AtomicU32,
    /// The one render thread and tile cache for every open document.
    pub render: RenderService,
    /// Local state (recents, positions, settings, session). Set once at
    /// startup; absent if it could not be opened, and the app then runs
    /// without remembering anything rather than not at all.
    store: OnceLock<Store>,
}

impl AppState {
    pub fn new(started: Instant, startup_document: Option<PathBuf>) -> Self {
        Self {
            started,
            startup_document: Mutex::new(startup_document),
            started_up: AtomicBool::new(false),
            documents: Mutex::new(HashMap::new()),
            locked: Mutex::new(HashMap::new()),
            recents: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
            render: RenderService::new(render_config()),
            store: OnceLock::new(),
        }
    }

    pub fn set_store(&self, store: Store) {
        let _ = self.store.set(store);
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.get()
    }

    /// True on the first call only: the UI's first startup request restores the
    /// session; a reload of the UI re-attaches to what is already open.
    pub fn begin_startup(&self) -> bool {
        !self.started_up.swap(true, Ordering::SeqCst)
    }

    /// Returns the launch document path once; later calls get `None`.
    pub fn take_startup_document(&self) -> Option<PathBuf> {
        lock(&self.startup_document).take()
    }

    pub fn next_token(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn insert_document(&self, document: Document, path: PathBuf) -> (u32, Arc<Document>) {
        let id = self.next_token();
        let document = Arc::new(document);
        let name = display_name(&path);
        let file =
            OpenFile { document: Arc::clone(&document), path, name, index: Arc::new(IndexCache::new()) };
        lock(&self.documents).insert(id, file);
        for lane in [Lane::Main, Lane::Thumbnails] {
            self.render.open_document(lane.render_id(id), document.clone());
        }
        (id, document)
    }

    pub fn document(&self, id: u32) -> Option<Arc<Document>> {
        lock(&self.documents).get(&id).map(|file| Arc::clone(&file.document))
    }

    pub fn index(&self, id: u32) -> Option<(Arc<Document>, Arc<IndexCache>)> {
        lock(&self.documents).get(&id).map(|file| (Arc::clone(&file.document), Arc::clone(&file.index)))
    }

    pub fn path_of(&self, id: u32) -> Option<PathBuf> {
        lock(&self.documents).get(&id).map(|file| file.path.clone())
    }

    /// The id of an open document showing the same file as `path`, if any.
    pub fn find_open(&self, path: &Path) -> Option<u32> {
        let wanted = std::fs::canonicalize(path).ok()?;
        lock(&self.documents)
            .iter()
            .find(|(_, file)| std::fs::canonicalize(&file.path).ok().as_ref() == Some(&wanted))
            .map(|(id, _)| *id)
    }

    /// (id, name, page count) of every open document, oldest first.
    pub fn open_documents(&self) -> Vec<(u32, String, usize)> {
        let mut open: Vec<_> = lock(&self.documents)
            .iter()
            .map(|(id, file)| (*id, file.name.clone(), file.document.page_count()))
            .collect();
        open.sort_by_key(|(id, _, _)| *id);
        open
    }

    /// Closes a document: queued renders cancelled, cached tiles dropped, a
    /// search index build in progress stopped.
    pub fn remove_document(&self, id: u32) -> bool {
        let Some(file) = lock(&self.documents).remove(&id) else { return false };
        file.index.cancel();
        for lane in [Lane::Main, Lane::Thumbnails] {
            self.render.close_document(lane.render_id(id));
        }
        true
    }

    pub fn lock_pending(&self, path: PathBuf) -> u32 {
        let token = self.next_token();
        lock(&self.locked).insert(token, path);
        token
    }

    pub fn pending_path(&self, token: u32) -> Option<PathBuf> {
        lock(&self.locked).get(&token).cloned()
    }

    pub fn forget_pending(&self, token: u32) {
        lock(&self.locked).remove(&token);
    }

    /// Replaces the recent-file tokens with a fresh listing.
    pub fn remember_recents(&self, paths: Vec<PathBuf>) -> Vec<u32> {
        let mut recents = lock(&self.recents);
        recents.clear();
        paths
            .into_iter()
            .map(|path| {
                let token = self.next_token();
                recents.insert(token, path);
                token
            })
            .collect()
    }

    pub fn recent_path(&self, token: u32) -> Option<PathBuf> {
        lock(&self.recents).get(&token).cloned()
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Render settings. The tile cache size can be overridden with
/// `SAFHA_TILE_CACHE_MB`.
fn render_config() -> RenderConfig {
    let mut config = RenderConfig::default();
    if let Some(megabytes) =
        std::env::var("SAFHA_TILE_CACHE_MB").ok().and_then(|value| value.parse::<usize>().ok())
    {
        config.cache_bytes = megabytes.saturating_mul(1024 * 1024);
    }
    config
}
