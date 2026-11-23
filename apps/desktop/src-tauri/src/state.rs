use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use safha_core::Document;
use safha_core::edit::EditHistory;
use safha_core::recovery::{FileStamp, RecoveryFiles};
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
    /// Names its recovery file while it has unsaved changes (ADR 0040).
    pub recovery_key: String,
    /// The original file's size and time when it was opened.
    pub original: Option<FileStamp>,
    /// Restored from a recovery file whose original changed since: Save
    /// would overwrite someone else's version, so the first save is Save As.
    pub save_as_first: bool,
}

/// What autosave needs of a document.
pub struct AutosaveTarget {
    pub history: Arc<Mutex<EditHistory>>,
    pub key: String,
    pub path: PathBuf,
    pub name: String,
    pub original: Option<FileStamp>,
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
    /// Documents to open in the main window at startup: from the command
    /// line, the platform ("Open with"), or later copies of the app that
    /// started before the UI asked (ADR 0046). `None` once taken.
    launch_documents: Mutex<Option<Vec<PathBuf>>>,
    started_up: AtomicBool,
    /// The window the user was last in: documents another copy hands over
    /// open there.
    last_focused: Mutex<Option<String>>,
    documents: Mutex<HashMap<u32, OpenFile>>,
    /// Files waiting for a password, by token.
    locked: Mutex<HashMap<u32, PathBuf>>,
    /// Recovery files waiting for a password, by token (their keys).
    locked_recoveries: Mutex<HashMap<u32, String>>,
    /// Inserts from files waiting for a password (ADR 0016), by token.
    pending_inserts: Mutex<HashMap<u32, crate::editing::PendingInsert>>,
    /// Recovery files from the last listing, by token (their keys).
    recoveries: Mutex<HashMap<u32, String>>,
    /// Bumped by each change to a document: an autosave writes only for the
    /// latest (ADR 0040).
    autosave: Mutex<HashMap<u32, u64>>,
    /// Where documents with unsaved changes are kept (ADR 0040).
    recovery: OnceLock<RecoveryFiles>,
    /// The running app, for autosaves scheduled after a change; absent in tests.
    app: OnceLock<tauri::AppHandle>,
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
    pub fn new(started: Instant, launch_documents: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            started,
            launch_documents: Mutex::new(Some(launch_documents.into_iter().collect())),
            started_up: AtomicBool::new(false),
            last_focused: Mutex::new(None),
            documents: Mutex::new(HashMap::new()),
            locked: Mutex::new(HashMap::new()),
            locked_recoveries: Mutex::new(HashMap::new()),
            pending_inserts: Mutex::new(HashMap::new()),
            recoveries: Mutex::new(HashMap::new()),
            autosave: Mutex::new(HashMap::new()),
            recovery: OnceLock::new(),
            app: OnceLock::new(),
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

    pub fn set_recovery(&self, files: RecoveryFiles) {
        let _ = self.recovery.set(files);
    }

    pub fn set_app(&self, app: tauri::AppHandle) {
        let _ = self.app.set(app);
    }

    pub fn recovery(&self) -> Option<&RecoveryFiles> {
        self.recovery.get()
    }

    /// Marks a change to a document; the number an autosave waits on.
    pub fn bump_autosave(&self, id: u32) -> u64 {
        let mut generations = lock(&self.autosave);
        let generation = generations.entry(id).or_insert(0);
        *generation += 1;
        *generation
    }

    pub fn autosave_generation(&self, id: u32) -> Option<u64> {
        lock(&self.autosave).get(&id).copied()
    }

    pub fn autosave_target(&self, id: u32) -> Option<AutosaveTarget> {
        lock(&self.documents).get(&id).map(|file| AutosaveTarget {
            history: Arc::clone(&file.history),
            key: file.recovery_key.clone(),
            path: file.path.clone(),
            name: file.name.clone(),
            original: file.original,
        })
    }

    /// The recovery keys of open documents.
    pub fn recovery_keys(&self) -> HashSet<String> {
        lock(&self.documents).values().map(|file| file.recovery_key.clone()).collect()
    }

    /// A token for a listed recovery file (the UI never sees its path).
    pub fn remember_recovery(&self, key: String) -> u32 {
        let token = self.next_token();
        lock(&self.recoveries).insert(token, key);
        token
    }

    pub fn recovery_key(&self, token: u32) -> Option<String> {
        lock(&self.recoveries).get(&token).cloned()
    }

    pub fn lock_pending_recovery(&self, key: String) -> u32 {
        let token = self.next_token();
        lock(&self.locked_recoveries).insert(token, key);
        token
    }

    pub fn pending_recovery(&self, token: u32) -> Option<String> {
        lock(&self.locked_recoveries).get(&token).cloned()
    }

    /// Whether the document must be saved somewhere new before anywhere else.
    pub fn save_as_first(&self, id: u32) -> bool {
        lock(&self.documents).get(&id).is_some_and(|file| file.save_as_first)
    }

    /// True on the first call only: the UI's first startup request restores the
    /// session; a reload of the UI re-attaches to what is already open.
    pub fn begin_startup(&self) -> bool {
        !self.started_up.swap(true, Ordering::SeqCst)
    }

    /// Returns the launch documents once; later calls get none.
    pub fn take_launch_documents(&self) -> Vec<PathBuf> {
        lock(&self.launch_documents).take().unwrap_or_default()
    }

    /// Documents arriving from elsewhere (another copy, the platform): they
    /// join the launch documents while the UI has not taken those yet.
    /// Otherwise they are returned, to be opened now.
    pub fn add_launch_documents(&self, documents: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
        match lock(&self.launch_documents).as_mut() {
            Some(waiting) => {
                waiting.extend(documents);
                None
            }
            None => Some(documents),
        }
    }

    /// A window got the focus.
    pub fn focused(&self, window: &str) {
        *lock(&self.last_focused) = Some(window.to_string());
    }

    /// Where documents from elsewhere go: the document window the user was
    /// last in, or the first one still open.
    pub fn arrival_window(&self) -> Option<String> {
        let windows = lock(&self.windows);
        let last = lock(&self.last_focused).clone();
        last.filter(|label| windows.iter().any(|(known, _)| known == label))
            .or_else(|| windows.first().map(|(label, _)| label.clone()))
    }

    pub fn next_token(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn insert_document(&self, document: Document, path: PathBuf, window: &str) -> (u32, Arc<Document>) {
        let history = EditHistory::new(document.bytes());
        let original = FileStamp::of(&path);
        self.insert_with(document, path, window, history, None, original, false)
    }

    /// Opens a document restored from a recovery file (ADR 0040), keeping the
    /// file's key.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_restored(
        &self,
        document: Document,
        path: PathBuf,
        window: &str,
        history: EditHistory,
        key: String,
        original: Option<FileStamp>,
        save_as_first: bool,
    ) -> (u32, Arc<Document>) {
        self.insert_with(document, path, window, history, Some(key), original, save_as_first)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_with(
        &self,
        document: Document,
        path: PathBuf,
        window: &str,
        history: EditHistory,
        key: Option<String>,
        original: Option<FileStamp>,
        save_as_first: bool,
    ) -> (u32, Arc<Document>) {
        let id = self.next_token();
        let document = Arc::new(document);
        let name = display_name(&path);
        // Unique across runs: a crashed run's recovery files keep their names.
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis());
        let file = OpenFile {
            document: Arc::clone(&document),
            path,
            name,
            index: Arc::new(IndexCache::new()),
            window: window.to_string(),
            history: Arc::new(Mutex::new(history)),
            recovery_key: key.unwrap_or_else(|| format!("{millis}-{}-{id}", std::process::id())),
            original,
            save_as_first,
        };
        lock(&self.documents).insert(id, file);
        for lane in [Lane::Main, Lane::Thumbnails] {
            self.render.open_document(lane.render_id(id), document.clone());
        }
        self.documents_changed();
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
        // Every edit, undo, and redo ends here: keep the recovery file current.
        if let Some(app) = self.app.get() {
            crate::recovery::changed(app, id);
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
    /// The document was saved to `path`: from now on it is that file.
    pub fn set_path(&self, id: u32, path: PathBuf) -> Option<String> {
        let mut documents = lock(&self.documents);
        let file = documents.get_mut(&id)?;
        file.name = display_name(&path);
        file.original = FileStamp::of(&path);
        file.save_as_first = false;
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

    /// The open documents showing the file at `path` (canonical, as the state
    /// store names files), oldest first.
    pub fn documents_of(&self, path: &Path) -> Vec<u32> {
        let mut ids: Vec<u32> = lock(&self.documents)
            .iter()
            .filter(|(_, file)| safha_core::store::canonical_path(&file.path) == path)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// The label of the window whose tab shows document `id`.
    pub fn window_of(&self, id: u32) -> Option<String> {
        lock(&self.documents).get(&id).map(|file| file.window.clone())
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

    pub fn exiting(&self) -> bool {
        self.exiting.load(Ordering::SeqCst)
    }

    pub fn set_exiting(&self) {
        self.exiting.store(true, Ordering::SeqCst);
    }

    /// Closes a document: queued renders cancelled, cached tiles dropped, a
    /// search index build in progress stopped.
    pub fn remove_document(&self, id: u32) -> bool {
        let Some(file) = lock(&self.documents).remove(&id) else { return false };
        file.index.cancel();
        // Closed: saved, or its changes discarded. Either way it has nothing to recover.
        lock(&self.autosave).remove(&id);
        if let Some(files) = self.recovery() {
            files.remove(&file.recovery_key);
        }
        if let Some(store) = self.store()
            && let Err(err) = store.forget_recovery(&file.recovery_key)
        {
            eprintln!("safha: could not forget a recovery file: {err}");
        }
        for lane in [Lane::Main, Lane::Thumbnails] {
            self.render.close_document(lane.render_id(id));
        }
        self.documents_changed();
        true
    }

    /// A document opened or closed: an OCR job waiting for it may run (ADR 0035).
    fn documents_changed(&self) {
        use tauri::Manager as _;
        if let Some(app) = self.app.get()
            && let Some(ocr) = app.try_state::<crate::ocr::OcrState>()
        {
            ocr.wake();
        }
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
        lock(&self.locked_recoveries).remove(&token);
        lock(&self.pending_inserts).remove(&token);
    }

    /// Keeps an insert that waits for a file's password; the token asks for it.
    pub fn hold_insert(&self, pending: crate::editing::PendingInsert) -> u32 {
        let token = self.next_token();
        lock(&self.pending_inserts).insert(token, pending);
        token
    }

    pub fn take_insert(&self, token: u32) -> Option<crate::editing::PendingInsert> {
        lock(&self.pending_inserts).remove(&token)
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

    #[test]
    fn documents_from_elsewhere_wait_for_the_ui_and_then_open_directly() {
        let state = AppState::new(Instant::now(), [PathBuf::from("/a.pdf")]);
        assert_eq!(state.add_launch_documents(vec![PathBuf::from("/b.pdf")]), None);
        assert_eq!(state.take_launch_documents(), [PathBuf::from("/a.pdf"), PathBuf::from("/b.pdf")]);
        // The UI has taken them: later arrivals are the caller's to open.
        assert_eq!(
            state.add_launch_documents(vec![PathBuf::from("/c.pdf")]),
            Some(vec![PathBuf::from("/c.pdf")])
        );
        assert!(state.take_launch_documents().is_empty());
    }

    #[test]
    fn documents_from_elsewhere_go_to_the_window_last_used() {
        let state = state();
        assert_eq!(state.arrival_window().as_deref(), Some(MAIN_WINDOW));
        let second = state.add_window();
        state.focused(&second);
        assert_eq!(state.arrival_window(), Some(second.clone()));
        // A print window is no document window.
        state.focused("print-7");
        assert_eq!(state.arrival_window().as_deref(), Some(MAIN_WINDOW));
        state.focused(&second);
        state.set_window_tabs(&second, WindowTabs { ids: vec![1], active: Some(1) });
        assert!(state.close_window(&second));
        assert_eq!(state.arrival_window().as_deref(), Some(MAIN_WINDOW));
    }
}
