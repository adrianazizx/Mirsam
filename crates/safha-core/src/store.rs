//! Local application state: recent files, per-file view position, settings,
//! and the open-tabs session (spec sections 5.1 and 9, "Storage").
//!
//! One SQLite file in the app data directory. Everything stays on the device
//! (spec section 1, "Private by default"). Paths are stored here and never
//! handed to the UI, which sees only file names (ADR 0001).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not create the state directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("state database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("state database is newer than this version of the app (schema {0})")]
    TooNew(i64),
}

pub type StoreResult<T> = std::result::Result<T, StoreError>;

/// Where a document was left, restored the next time it opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewState {
    pub page: usize,
    /// "fit-width", "fit-page", or "custom"; the UI owns the vocabulary.
    pub zoom_mode: String,
    pub zoom: f32,
    /// Scroll position within the page, 0.0 (top) to 1.0 (bottom).
    pub page_offset: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFile {
    pub path: PathBuf,
    pub name: String,
    /// Unix seconds.
    pub last_opened: i64,
    /// Whether the file is still there.
    pub exists: bool,
}

/// The tabs that were open when the app last quit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    pub files: Vec<PathBuf>,
    pub active: Option<usize>,
}

/// Bumped whenever `migrate` gains a step.
const SCHEMA_VERSION: i64 = 1;
const MAX_RECENTS: i64 = 200;

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    /// Opens (creating if needed) the store at `path`.
    pub fn open(path: &Path) -> StoreResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::init(Connection::open(path)?)
    }

    /// A throwaway store, for tests.
    pub fn in_memory() -> StoreResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(connection: Connection) -> StoreResult<Self> {
        // WAL keeps a crash mid-write from corrupting the file; it is state, not documents.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&connection)?;
        Ok(Self { connection: Mutex::new(connection) })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> StoreResult<T> {
        let connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(f(&connection)?)
    }

    /// Records that `path` was opened now. Keeps the list bounded.
    pub fn record_open(&self, path: &Path) -> StoreResult<()> {
        let key = key_of(path);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.with(|db| {
            db.execute(
                "INSERT INTO recent_files (path, name, last_opened, seq)
                 VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(seq), 0) + 1 FROM recent_files))
                 ON CONFLICT(path) DO UPDATE SET name = excluded.name, last_opened = excluded.last_opened,
                   seq = excluded.seq",
                params![key, name, now()],
            )?;
            db.execute(
                "DELETE FROM recent_files WHERE path NOT IN
                 (SELECT path FROM recent_files ORDER BY seq DESC LIMIT ?1)",
                params![MAX_RECENTS],
            )?;
            Ok(())
        })
    }

    /// Most recently opened first.
    pub fn recents(&self, limit: usize) -> StoreResult<Vec<RecentFile>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.with(|db| {
            let mut statement =
                db.prepare("SELECT path, name, last_opened FROM recent_files ORDER BY seq DESC LIMIT ?1")?;
            let rows = statement.query_map(params![limit], |row| {
                let path = PathBuf::from(row.get::<_, String>(0)?);
                Ok(RecentFile { exists: path.is_file(), path, name: row.get(1)?, last_opened: row.get(2)? })
            })?;
            rows.collect()
        })
    }

    pub fn forget_recent(&self, path: &Path) -> StoreResult<()> {
        self.with(|db| {
            db.execute("DELETE FROM recent_files WHERE path = ?1", params![key_of(path)]).map(drop)
        })
    }

    pub fn clear_recents(&self) -> StoreResult<()> {
        self.with(|db| db.execute("DELETE FROM recent_files", []).map(drop))
    }

    pub fn save_view_state(&self, path: &Path, state: &ViewState) -> StoreResult<()> {
        let page = i64::try_from(state.page).unwrap_or(i64::MAX);
        self.with(|db| {
            db.execute(
                "INSERT INTO view_state (path, page, zoom_mode, zoom, page_offset, saved) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET page = excluded.page, zoom_mode = excluded.zoom_mode,
                   zoom = excluded.zoom, page_offset = excluded.page_offset, saved = excluded.saved",
                params![key_of(path), page, state.zoom_mode, f64::from(state.zoom), f64::from(state.page_offset), now()],
            )
            .map(drop)
        })
    }

    pub fn view_state(&self, path: &Path) -> StoreResult<Option<ViewState>> {
        self.with(|db| {
            db.query_row(
                "SELECT page, zoom_mode, zoom, page_offset FROM view_state WHERE path = ?1",
                params![key_of(path)],
                |row| {
                    Ok(ViewState {
                        page: usize::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                        zoom_mode: row.get(1)?,
                        zoom: row.get::<_, f64>(2)? as f32,
                        page_offset: (row.get::<_, f64>(3)? as f32).clamp(0.0, 1.0),
                    })
                },
            )
            .optional()
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> StoreResult<()> {
        self.with(|db| {
            db.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(drop)
        })
    }

    pub fn setting(&self, key: &str) -> StoreResult<Option<String>> {
        self.with(|db| {
            db.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
                .optional()
        })
    }

    /// Replaces the saved session atomically.
    pub fn save_session(&self, session: &Session) -> StoreResult<()> {
        let mut connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM session_tabs", [])?;
        for (position, path) in session.files.iter().enumerate() {
            let active = session.active == Some(position);
            transaction.execute(
                "INSERT INTO session_tabs (position, path, active) VALUES (?1, ?2, ?3)",
                params![i64::try_from(position).unwrap_or(i64::MAX), key_of(path), active],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// The saved session, skipping files that no longer exist.
    pub fn session(&self) -> StoreResult<Session> {
        let rows: Vec<(PathBuf, bool)> = self.with(|db| {
            let mut statement = db.prepare("SELECT path, active FROM session_tabs ORDER BY position")?;
            let rows =
                statement.query_map([], |row| Ok((PathBuf::from(row.get::<_, String>(0)?), row.get(1)?)))?;
            rows.collect()
        })?;
        let mut session = Session::default();
        for (path, active) in rows.into_iter().filter(|(path, _)| path.is_file()) {
            if active {
                session.active = Some(session.files.len());
            }
            session.files.push(path);
        }
        Ok(session)
    }
}

/// The same file opened through different relative paths or symlinks is one entry.
/// A file that no longer exists cannot be canonicalized, so its folder is
/// (on macOS `/var` is itself a symlink to `/private/var`).
fn key_of(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).ok().or_else(|| {
        let parent = std::fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    });
    canonical.unwrap_or_else(|| path.to_path_buf()).to_string_lossy().into_owned()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn migrate(db: &Connection) -> StoreResult<()> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::TooNew(version));
    }
    if version < 1 {
        db.execute_batch(
            "BEGIN;
             CREATE TABLE recent_files (
                 path TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 last_opened INTEGER NOT NULL,
                 -- Opening order; timestamps tie within a second.
                 seq INTEGER NOT NULL
             );
             CREATE TABLE view_state (
                 path TEXT PRIMARY KEY,
                 page INTEGER NOT NULL,
                 zoom_mode TEXT NOT NULL,
                 zoom REAL NOT NULL,
                 page_offset REAL NOT NULL,
                 saved INTEGER NOT NULL
             );
             CREATE TABLE settings (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE session_tabs (
                 position INTEGER PRIMARY KEY,
                 path TEXT NOT NULL,
                 active INTEGER NOT NULL
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"%PDF-1.7\n").expect("writes");
        path
    }

    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("safha-store-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates");
        dir
    }

    #[test]
    fn recents_are_most_recent_first_and_deduplicated() {
        let dir = scratch();
        let store = Store::in_memory().expect("opens");
        let a = temp_file(&dir, "عقد.pdf");
        let b = temp_file(&dir, "b.pdf");
        store.record_open(&a).expect("records");
        store.record_open(&b).expect("records");
        store.record_open(&a).expect("records");
        let recents = store.recents(10).expect("lists");
        let names: Vec<&str> = recents.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["عقد.pdf", "b.pdf"]);
        assert!(recents.iter().all(|r| r.exists));

        std::fs::remove_file(&b).expect("removes");
        assert!(!store.recents(10).expect("lists")[1].exists);
        store.forget_recent(&b).expect("forgets");
        assert_eq!(store.recents(10).expect("lists").len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn view_state_round_trips_per_file() {
        let dir = scratch();
        let store = Store::in_memory().expect("opens");
        let file = temp_file(&dir, "a.pdf");
        assert_eq!(store.view_state(&file).expect("reads"), None);
        let state = ViewState { page: 41, zoom_mode: "custom".into(), zoom: 1.5, page_offset: 0.25 };
        store.save_view_state(&file, &state).expect("saves");
        assert_eq!(store.view_state(&file).expect("reads"), Some(state));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_skips_files_that_are_gone() {
        let dir = scratch();
        let store = Store::in_memory().expect("opens");
        let a = temp_file(&dir, "a.pdf");
        let b = temp_file(&dir, "b.pdf");
        let c = temp_file(&dir, "c.pdf");
        store
            .save_session(&Session { files: vec![a.clone(), b.clone(), c.clone()], active: Some(2) })
            .expect("saves");
        std::fs::remove_file(&b).expect("removes");
        let session = store.session().expect("reads");
        assert_eq!(session.files.len(), 2);
        assert_eq!(session.active, Some(1));
        assert!(session.files[1].ends_with("c.pdf"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_and_reopening_a_file_store() {
        let dir = scratch();
        let path = dir.join("nested").join("state.sqlite");
        {
            let store = Store::open(&path).expect("creates");
            store.set_setting("locale", "ar").expect("sets");
            store.set_setting("locale", "en").expect("overwrites");
        }
        let store = Store::open(&path).expect("reopens");
        assert_eq!(store.setting("locale").expect("reads").as_deref(), Some("en"));
        assert_eq!(store.setting("missing").expect("reads"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_a_database_from_a_newer_app() {
        let connection = Connection::open_in_memory().expect("opens");
        connection.pragma_update(None, "user_version", SCHEMA_VERSION + 1).expect("sets");
        assert!(matches!(Store::init(connection), Err(StoreError::TooNew(_))));
    }
}
