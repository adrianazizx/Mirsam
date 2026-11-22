use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use safha_core::Document;
use safha_core::edit::EditHistory;
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
    /// Label of the window whose tab shows it.
    pub window: String,
    /// The document's edits, for undo and redo (ADR 0014). Its lock also
    /// keeps a document's edits in sequence.
    pub history: Arc<Mutex<EditHistory>>,
}

/// One window's tabs, as its UI last reported them (for the session).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowTabs {
    pub ids: Vec<u32>,
    pub active: Option<u32>,
}

/// The label of the first window, from tauri.conf.json.
pub const MAIN_WINDOW: &str = "main";

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
    /// Every window's tabs, in the order the windows opened (main first).
    windows: Mutex<Vec<(String, WindowTabs)>>,
    /// Set once the app is quitting: closing windows then must not drop
    /// their tabs from the saved session.
    exiting: AtomicBool,
    /// Windows the user chose to close although they hold unsaved changes.
    close_confirmed: Mutex<HashSet<String>>,
    /// The user chose to quit although documents hold unsaved changes.
    quit_confirmed: AtomicBool,
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
            windows: Mutex::new(vec![(MAIN_WINDOW.to_string(), WindowTabs::default())]),
            exiting: AtomicBool::new(false),
            close_confirmed: Mutex::new(HashSet::new()),
            quit_confirmed: AtomicBool::new(false),
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

    pub fn insert_document(&self, document: Document, path: PathBuf, window: &str) -> (u32, Arc<Document>) {
        let id = self.next_token();
        let document = Arc::new(document);
        let name = display_name(&path);
        let file = OpenFile {
            document: Arc::clone(&document),
            path,
            name,
            index: Arc::new(IndexCache::new()),
            window: window.to_string(),
            history: Arc::new(Mutex::new(EditHistory::new(document.bytes()))),
        };
        lock(&self.documents).insert(id, file);
        for lane in [Lane::Main, Lane::Thumbnails] {
            self.render.open_document(lane.render_id(id), document.clone());
        }
        (id, document)
    }

    /// The document's edit history; lock it before reading the document to
    /// edit, so edits apply one after another.
    pub fn history(&self, id: u32) -> Option<Arc<Mutex<EditHistory>>> {
        lock(&self.documents).get(&id).map(|file| Arc::clone(&file.history))
    }

    /// Shows another revision of an open document (after an edit, an undo, or
    /// a redo): renders, the search index, and text layers start over.
    pub fn replace_revision(&self, id: u32, document: Document) -> Option<Arc<Document>> {
        let document = Arc::new(document);
        {
            let mut documents = lock(&self.documents);
            let file = documents.get_mut(&id)?;
            file.document = Arc::clone(&document);
            file.index = Arc::new(IndexCache::new());
        }
        for lane in [Lane::Main, Lane::Thumbnails] {
            self.render.replace_document(lane.render_id(id), document.clone());
        }
        Some(document)
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

    /// After Save As: the document now is the file at `path`. Returns its
    /// display name.
    pub fn set_path(&self, id: u32, path: PathBuf) -> Option<String> {
        let mut documents = lock(&self.documents);
        let file = documents.get_mut(&id)?;
        file.name = display_name(&path);
        file.path = path;
        Some(file.name.clone())
    }

    /// The id of a document in `window` showing the same file as `path`, if
    /// any. Another window may have the same file open in its own tab.
    pub fn find_open(&self, path: &Path, window: &str) -> Option<u32> {
        let wanted = std::fs::canonicalize(path).ok()?;
        lock(&self.documents)
            .iter()
            .filter(|(_, file)| file.window == window)
            .find(|(_, file)| std::fs::canonicalize(&file.path).ok().as_ref() == Some(&wanted))
            .map(|(id, _)| *id)
    }

    /// (id, name) of the documents in `window`, in its tab order where known,
    /// then oldest first.
    pub fn open_documents(&self, window: &str) -> Vec<(u32, String)> {
        let order = self.window_tabs(window).ids;
        let mut open: Vec<_> = lock(&self.documents)
            .iter()
            .filter(|(_, file)| file.window == window)
            .map(|(id, file)| (*id, file.name.clone()))
            .collect();
        open.sort_by_key(|(id, _)| (order.iter().position(|known| known == id).unwrap_or(usize::MAX), *id));
        open
    }

    /// Moves a document's tab to another window.
    pub fn move_to_window(&self, id: u32, window: &str) -> bool {
        match lock(&self.documents).get_mut(&id) {
            Some(file) => file.window = window.to_string(),
            None => return false,
        }
        let mut windows = lock(&self.windows);
        for (_, tabs) in windows.iter_mut() {
            tabs.ids.retain(|known| *known != id);
            if tabs.active == Some(id) {
                tabs.active = None;
            }
        }
        if let Some((_, tabs)) = windows.iter_mut().find(|(label, _)| label == window) {
            tabs.ids.push(id);
            tabs.active = Some(id);
        }
        true
    }

    /// A label for a new window, registered (with no tabs yet) in window order.
    pub fn add_window(&self) -> String {
        let label = format!("window-{}", self.next_token());
        lock(&self.windows).push((label.clone(), WindowTabs::default()));
        label
    }

    pub fn set_window_tabs(&self, window: &str, tabs: WindowTabs) {
        let mut windows = lock(&self.windows);
        match windows.iter_mut().find(|(label, _)| label == window) {
            Some((_, known)) => *known = tabs,
            None => windows.push((window.to_string(), tabs)),
        }
    }

    pub fn window_tabs(&self, window: &str) -> WindowTabs {
        lock(&self.windows)
            .iter()
            .find(|(label, _)| label == window)
            .map(|(_, tabs)| tabs.clone())
            .unwrap_or_default()
    }

    /// Every window's tabs as file paths, in window order, for the session.
    pub fn session_windows(&self) -> Vec<(Vec<PathBuf>, Option<usize>)> {
        let windows = lock(&self.windows).clone();
        windows
            .into_iter()
            .map(|(_, tabs)| {
                let files: Vec<(u32, PathBuf)> =
                    tabs.ids.iter().filter_map(|id| Some((*id, self.path_of(*id)?))).collect();
                let active = tabs.active.and_then(|active| files.iter().position(|(id, _)| *id == active));
                (files.into_iter().map(|(_, path)| path).collect::<Vec<_>>(), active)
            })
            .filter(|(files, _)| !files.is_empty())
            .collect()
    }

    /// A window closed: its documents close with it. Returns whether the
    /// session should be rewritten — not while quitting, and not for the
    /// last window, whose tabs are what the next launch restores.
    pub fn close_window(&self, window: &str) -> bool {
        let ids: Vec<u32> = lock(&self.documents)
            .iter()
            .filter(|(_, file)| file.window == window)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.remove_document(id);
        }
        let mut windows = lock(&self.windows);
        let others = windows.iter().filter(|(label, _)| label != window).count();
        if self.exiting.load(Ordering::SeqCst) || others == 0 {
            return false;
        }
        windows.retain(|(label, _)| label != window);
        true
    }

    /// Open documents with unsaved changes, with their windows: in `window`,
    /// or in any window.
    pub fn unsaved(&self, window: Option<&str>) -> Vec<(u32, String)> {
        // Edits lock a history and then the document map; never hold the map
        // while waiting for a history.
        let files: Vec<(u32, String, Arc<Mutex<EditHistory>>)> = lock(&self.documents)
            .iter()
            .filter(|(_, file)| window.is_none_or(|window| file.window == window))
            .map(|(id, file)| (*id, file.window.clone(), Arc::clone(&file.history)))
            .collect();
        let mut unsaved: Vec<(u32, String)> = files
            .into_iter()
            .filter(|(_, _, history)| lock(history).is_modified())
            .map(|(id, window, _)| (id, window))
            .collect();
        unsaved.sort_unstable();
        unsaved
    }

    /// The user confirmed closing `window` without saving.
    pub fn confirm_close(&self, window: &str) {
        lock(&self.close_confirmed).insert(window.to_string());
    }

    /// Whether `window` may close although it holds unsaved changes (once).
    pub fn take_close_confirmed(&self, window: &str) -> bool {
        lock(&self.close_confirmed).remove(window)
    }

    /// The user confirmed quitting without saving.
    pub fn confirm_quit(&self) {
        self.quit_confirmed.store(true, Ordering::SeqCst);
    }

    pub fn quit_confirmed(&self) -> bool {
        self.quit_confirmed.load(Ordering::SeqCst)
    }

    pub fn set_exiting(&self) {
        self.exiting.store(true, Ordering::SeqCst);
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

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new(Instant::now(), None)
    }

    #[test]
    fn closing_a_window_drops_its_tabs_from_the_session_unless_it_is_the_last_or_the_app_quits() {
        let state = state();
        let second = state.add_window();
        state.set_window_tabs(MAIN_WINDOW, WindowTabs { ids: vec![1], active: Some(1) });
        state.set_window_tabs(&second, WindowTabs { ids: vec![2], active: Some(2) });
        // Another window remains: the closed one leaves the session.
        assert!(state.close_window(&second));
        assert_eq!(state.window_tabs(&second), WindowTabs::default());
        // The last window: its tabs stay, to be restored next time.
        assert!(!state.close_window(MAIN_WINDOW));
        assert_eq!(state.window_tabs(MAIN_WINDOW).ids, vec![1]);

        let quitting = AppState::new(Instant::now(), None);
        let other = quitting.add_window();
        quitting.set_window_tabs(&other, WindowTabs { ids: vec![3], active: None });
        quitting.set_exiting();
        assert!(!quitting.close_window(&other));
        assert_eq!(quitting.window_tabs(&other).ids, vec![3]);
    }

    #[test]
    fn a_tab_moved_to_a_new_window_leaves_the_old_one() {
        let state = state();
        state.set_window_tabs(MAIN_WINDOW, WindowTabs { ids: vec![], active: None });
        // No document 7 is open: nothing moves.
        let second = state.add_window();
        assert!(!state.move_to_window(7, &second));
        assert!(second.starts_with("window-"));
    }
}
