use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use safha_core::Document;
use safha_core::render::{RenderConfig, RenderService};

pub struct AppState {
    /// Process start, for the self-test's cold-start timing.
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    pub started: Instant,
    startup_document: Mutex<Option<PathBuf>>,
    documents: Mutex<HashMap<u32, Arc<Document>>>,
    next_document_id: AtomicU32,
    /// The one render thread and tile cache for every open document.
    pub render: RenderService,
}

impl AppState {
    pub fn new(started: Instant, startup_document: Option<PathBuf>) -> Self {
        Self {
            started,
            startup_document: Mutex::new(startup_document),
            documents: Mutex::new(HashMap::new()),
            next_document_id: AtomicU32::new(1),
            render: RenderService::new(render_config()),
        }
    }

    /// Returns the launch document path once; later calls get `None`.
    pub fn take_startup_document(&self) -> Option<PathBuf> {
        self.startup_document.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    pub fn insert_document(&self, document: Document) -> (u32, Arc<Document>) {
        let id = self.next_document_id.fetch_add(1, Ordering::Relaxed);
        let document = Arc::new(document);
        self.documents.lock().unwrap_or_else(PoisonError::into_inner).insert(id, Arc::clone(&document));
        (id, document)
    }

    pub fn document(&self, id: u32) -> Option<Arc<Document>> {
        self.documents.lock().unwrap_or_else(PoisonError::into_inner).get(&id).cloned()
    }

    pub fn remove_document(&self, id: u32) -> bool {
        self.documents.lock().unwrap_or_else(PoisonError::into_inner).remove(&id).is_some()
    }
}

/// Render settings. The tile cache size can be overridden with
/// `SAFHA_TILE_CACHE_MB` until the settings store exists (Milestone 1).
fn render_config() -> RenderConfig {
    let mut config = RenderConfig::default();
    if let Some(megabytes) =
        std::env::var("SAFHA_TILE_CACHE_MB").ok().and_then(|value| value.parse::<usize>().ok())
    {
        config.cache_bytes = megabytes.saturating_mul(1024 * 1024);
    }
    config
}
