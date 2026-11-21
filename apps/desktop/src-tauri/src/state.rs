use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use safha_core::Document;

pub struct AppState {
    /// Process start, for the self-test's cold-start timing.
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    pub started: Instant,
    startup_document: Mutex<Option<PathBuf>>,
    documents: Mutex<HashMap<u32, Arc<Document>>>,
    next_document_id: AtomicU32,
}

impl AppState {
    pub fn new(started: Instant, startup_document: Option<PathBuf>) -> Self {
        Self {
            started,
            startup_document: Mutex::new(startup_document),
            documents: Mutex::new(HashMap::new()),
            next_document_id: AtomicU32::new(1),
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
