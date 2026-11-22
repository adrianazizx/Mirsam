//! Local application state: recent files, per-file view position, settings,
//! the open-tabs session (spec sections 5.1 and 9, "Storage"), and the
//! user's signatures and initials (spec §5.6, ADR 0045).
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

/// The windows and tabs that were open when the app last quit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    /// In window order; the first is the main window.
    pub windows: Vec<SessionWindow>,
}

/// One window's tabs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionWindow {
    pub files: Vec<PathBuf>,
    pub active: Option<usize>,
}

/// A document's unsaved edits, kept as a recovery file (ADR 0040): what
/// the file belongs to. The bytes are in `recovery::RecoveryFiles`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryEntry {
    /// The recovery file's name (see `RecoveryFiles`).
    pub key: String,
    /// The file the document was opened from (and is saved back to).
    pub path: PathBuf,
    pub name: String,
    /// When the recovery file was last written (Unix seconds).
    pub saved_at: i64,
    /// The original's size and modification time when the document was
    /// opened, to tell whether it changed since.
    pub original: Option<crate::recovery::FileStamp>,
}

/// A signature or initials kept for Fill & Sign (ADR 0045).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSignature {
    pub id: i64,
    /// Initials rather than a full signature.
    pub initials: bool,
    /// Unix seconds.
    pub created: i64,
    pub signature: crate::annotate::Signature,
}

/// Bumped whenever `migrate` gains a step.
const SCHEMA_VERSION: i64 = 4;
const MAX_RECENTS: i64 = 200;
/// The most signatures kept; saving another forgets the oldest.
pub const MAX_SIGNATURES: i64 = 24;

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

    /// Records (or updates) a recovery file's entry; `saved_at` is set to now.
    pub fn save_recovery(&self, entry: &RecoveryEntry) -> StoreResult<()> {
        let (len, modified) = match entry.original {
            Some(stamp) => (Some(i64::try_from(stamp.len).unwrap_or(i64::MAX)), Some(stamp.modified)),
            None => (None, None),
        };
        self.with(|db| {
            db.execute(
                "INSERT INTO recovery (key, path, name, saved_at, original_len, original_modified)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(key) DO UPDATE SET path = excluded.path, name = excluded.name,
                   saved_at = excluded.saved_at, original_len = excluded.original_len,
                   original_modified = excluded.original_modified",
                params![entry.key, key_of(&entry.path), entry.name, now(), len, modified],
            )
            .map(drop)
        })
    }

    /// Every recovery entry, most recently written first.
    pub fn recoveries(&self) -> StoreResult<Vec<RecoveryEntry>> {
        self.with(|db| {
            let mut statement = db.prepare(
                "SELECT key, path, name, saved_at, original_len, original_modified FROM recovery
                 ORDER BY saved_at DESC, key",
            )?;
            let rows = statement.query_map([], |row| {
                let len: Option<i64> = row.get(4)?;
                let modified: Option<i64> = row.get(5)?;
                Ok(RecoveryEntry {
                    key: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    name: row.get(2)?,
                    saved_at: row.get(3)?,
                    original: match (len, modified) {
                        (Some(len), Some(modified)) => Some(crate::recovery::FileStamp {
                            len: u64::try_from(len).unwrap_or(0),
                            modified,
                        }),
                        _ => None,
                    },
                })
            })?;
            rows.collect()
        })
    }

    pub fn forget_recovery(&self, key: &str) -> StoreResult<()> {
        self.with(|db| db.execute("DELETE FROM recovery WHERE key = ?1", params![key]).map(drop))
    }

    /// Replaces the saved session atomically.
    pub fn save_session(&self, session: &Session) -> StoreResult<()> {
        let mut connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM session_tabs", [])?;
        let mut position = 0i64;
        for (window, tabs) in session.windows.iter().enumerate() {
            for (index, path) in tabs.files.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO session_tabs (position, path, active, window) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        position,
                        key_of(path),
                        tabs.active == Some(index),
                        i64::try_from(window).unwrap_or(i64::MAX)
                    ],
                )?;
                position += 1;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The saved session, skipping files that no longer exist and windows
    /// left with none.
    pub fn session(&self) -> StoreResult<Session> {
        let rows: Vec<(PathBuf, bool, i64)> = self.with(|db| {
            let mut statement =
                db.prepare("SELECT path, active, window FROM session_tabs ORDER BY window, position")?;
            let rows = statement.query_map([], |row| {
                Ok((PathBuf::from(row.get::<_, String>(0)?), row.get(1)?, row.get(2)?))
            })?;
            rows.collect()
        })?;
        let mut session = Session::default();
        let mut current: Option<i64> = None;
        for (path, active, window) in rows.into_iter().filter(|(path, _, _)| path.is_file()) {
            if current != Some(window) {
                session.windows.push(SessionWindow::default());
                current = Some(window);
            }
            let Some(tabs) = session.windows.last_mut() else { continue };
            if active {
                tabs.active = Some(tabs.files.len());
            }
            tabs.files.push(path);
        }
        Ok(session)
    }
}

impl Store {
    /// Keeps a signature (or initials); returns its id. Only the newest
    /// [`MAX_SIGNATURES`] are kept.
    pub fn save_signature(&self, initials: bool, signature: &crate::annotate::Signature) -> StoreResult<i64> {
        use crate::annotate::Signature;
        let (kind, data) = match signature {
            Signature::Drawn(drawn) => ("drawn", serde_json::to_vec(drawn).unwrap_or_default()),
            Signature::Typed(typed) => ("typed", serde_json::to_vec(typed).unwrap_or_default()),
            Signature::Image(bytes) => ("image", bytes.clone()),
        };
        self.with(|db| {
            db.execute(
                "INSERT INTO signatures (kind, initials, data, created) VALUES (?1, ?2, ?3, ?4)",
                params![kind, initials, data, now()],
            )?;
            let id = db.last_insert_rowid();
            db.execute(
                "DELETE FROM signatures WHERE id NOT IN (SELECT id FROM signatures ORDER BY id DESC LIMIT ?1)",
                params![MAX_SIGNATURES],
            )?;
            Ok(id)
        })
    }

    /// Every kept signature, newest first. One that cannot be read is left out.
    pub fn signatures(&self) -> StoreResult<Vec<StoredSignature>> {
        self.with(|db| {
            let mut statement =
                db.prepare("SELECT id, kind, initials, data, created FROM signatures ORDER BY id DESC")?;
            let rows = statement.query_map([], |row| {
                let kind: String = row.get(1)?;
                let data: Vec<u8> = row.get(3)?;
                Ok((row.get::<_, i64>(0)?, kind, row.get::<_, bool>(2)?, data, row.get::<_, i64>(4)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, kind, initials, data, created) = row?;
                if let Some(signature) = decode_signature(&kind, data) {
                    out.push(StoredSignature { id, initials, created, signature });
                }
            }
            Ok(out)
        })
    }

    /// The signature kept as `id`.
    pub fn signature(&self, id: i64) -> StoreResult<Option<StoredSignature>> {
        Ok(self.signatures()?.into_iter().find(|stored| stored.id == id))
    }

    pub fn delete_signature(&self, id: i64) -> StoreResult<()> {
        self.with(|db| db.execute("DELETE FROM signatures WHERE id = ?1", params![id]).map(|_| ()))
    }
}

fn decode_signature(kind: &str, data: Vec<u8>) -> Option<crate::annotate::Signature> {
    use crate::annotate::Signature;
    Some(match kind {
        "drawn" => Signature::Drawn(serde_json::from_slice(&data).ok()?),
        "typed" => Signature::Typed(serde_json::from_slice(&data).ok()?),
        "image" => Signature::Image(data),
        _ => return None,
    })
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
    if version < 2 {
        // Several windows, each with its own tabs.
        db.execute_batch(
            "BEGIN;
             ALTER TABLE session_tabs ADD COLUMN window INTEGER NOT NULL DEFAULT 0;
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
    }
    if version < 3 {
        // Recovery files of documents with unsaved edits (ADR 0040).
        db.execute_batch(
            "BEGIN;
             CREATE TABLE recovery (
                 key TEXT PRIMARY KEY,
                 path TEXT NOT NULL,
                 name TEXT NOT NULL,
                 saved_at INTEGER NOT NULL,
                 original_len INTEGER,
                 original_modified INTEGER
             );
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
    }
    if version < 4 {
        // Signatures and initials for Fill & Sign (ADR 0045).
        db.execute_batch(
            "BEGIN;
             CREATE TABLE signatures (
                 id INTEGER PRIMARY KEY,
                 kind TEXT NOT NULL,
                 initials INTEGER NOT NULL,
                 data BLOB NOT NULL,
                 created INTEGER NOT NULL
             );
             PRAGMA user_version = 4;
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
    fn recovery_entries_are_kept_until_forgotten() {
        let dir = scratch();
        let store = Store::in_memory().expect("opens");
        let path = temp_file(&dir, "عقد.pdf");
        let stamp = crate::recovery::FileStamp::of(&path);
        let entry = RecoveryEntry {
            key: "1-7".into(),
            path: path.clone(),
            name: "عقد.pdf".into(),
            saved_at: 0,
            original: stamp,
        };
        store.save_recovery(&entry).expect("saves");
        store.save_recovery(&entry).expect("updates");
        let listed = store.recoveries().expect("lists");
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].key.as_str(), listed[0].name.as_str()), ("1-7", "عقد.pdf"));
        assert_eq!(listed[0].original, stamp);
        assert!(listed[0].saved_at > 0);
        store.forget_recovery("1-7").expect("forgets");
        assert!(store.recoveries().expect("lists").is_empty());
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
        let window = SessionWindow { files: vec![a.clone(), b.clone(), c.clone()], active: Some(2) };
        store.save_session(&Session { windows: vec![window] }).expect("saves");
        std::fs::remove_file(&b).expect("removes");
        let session = store.session().expect("reads");
        assert_eq!(session.windows.len(), 1);
        assert_eq!(session.windows[0].files.len(), 2);
        assert_eq!(session.windows[0].active, Some(1));
        assert!(session.windows[0].files[1].ends_with("c.pdf"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_keeps_each_window_and_drops_emptied_ones() {
        let dir = scratch();
        let store = Store::in_memory().expect("opens");
        let a = temp_file(&dir, "a.pdf");
        let b = temp_file(&dir, "b.pdf");
        let gone = temp_file(&dir, "gone.pdf");
        let windows = vec![
            SessionWindow { files: vec![a.clone()], active: Some(0) },
            SessionWindow { files: vec![gone.clone()], active: Some(0) },
            SessionWindow { files: vec![b.clone()], active: None },
        ];
        store.save_session(&Session { windows }).expect("saves");
        std::fs::remove_file(&gone).expect("removes");
        let session = store.session().expect("reads");
        assert_eq!(session.windows.len(), 2);
        assert!(session.windows[1].files[0].ends_with("b.pdf"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_version_1_database_gains_windows_and_keeps_its_session() {
        let dir = scratch();
        let path = dir.join("v1.sqlite3");
        let file = temp_file(&dir, "kept.pdf");
        {
            let db = Connection::open(&path).expect("opens");
            db.execute_batch(
                "CREATE TABLE recent_files (path TEXT PRIMARY KEY, name TEXT NOT NULL, last_opened INTEGER NOT NULL, seq INTEGER NOT NULL);
                 CREATE TABLE view_state (path TEXT PRIMARY KEY, page INTEGER NOT NULL, zoom_mode TEXT NOT NULL, zoom REAL NOT NULL, page_offset REAL NOT NULL, saved INTEGER NOT NULL);
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE session_tabs (position INTEGER PRIMARY KEY, path TEXT NOT NULL, active INTEGER NOT NULL);
                 PRAGMA user_version = 1;",
            )
            .expect("creates a version 1 schema");
            db.execute(
                "INSERT INTO session_tabs (position, path, active) VALUES (0, ?1, 1)",
                params![key_of(&file)],
            )
            .expect("inserts");
        }
        let store = Store::open(&path).expect("migrates");
        let session = store.session().expect("reads");
        assert_eq!(session.windows.len(), 1);
        assert_eq!(session.windows[0].active, Some(0));
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
    fn signatures_are_kept_newest_first_and_bounded() {
        use crate::annotate::{DrawnSignature, Signature, SignaturePoint, TypedSignature};
        use crate::fonts::FontFamily;
        let store = Store::in_memory().expect("opens");
        let typed = Signature::Typed(TypedSignature { text: "سارة".into(), family: FontFamily::Naskh });
        let drawn = Signature::Drawn(DrawnSignature {
            strokes: vec![vec![SignaturePoint { x: 1.0, y: 2.0, pressure: 0.5 }]],
            width: 3.0,
        });
        let first = store.save_signature(false, &typed).expect("saves");
        let second = store.save_signature(true, &drawn).expect("saves");
        let third = store.save_signature(false, &Signature::Image(vec![1, 2, 3])).expect("saves");
        let kept = store.signatures().expect("lists");
        assert_eq!(kept.iter().map(|s| s.id).collect::<Vec<_>>(), [third, second, first]);
        assert_eq!(kept[1].signature, drawn);
        assert!(kept[1].initials && !kept[2].initials);
        assert_eq!(store.signature(first).expect("reads").map(|s| s.signature), Some(typed));
        store.delete_signature(second).expect("deletes");
        assert_eq!(store.signatures().expect("lists").len(), 2);
        for _ in 0..MAX_SIGNATURES {
            store.save_signature(false, &drawn).expect("saves");
        }
        assert_eq!(store.signatures().expect("lists").len() as i64, MAX_SIGNATURES);
    }

    #[test]
    fn refuses_a_database_from_a_newer_app() {
        let connection = Connection::open_in_memory().expect("opens");
        connection.pragma_update(None, "user_version", SCHEMA_VERSION + 1).expect("sets");
        assert!(matches!(Store::init(connection), Err(StoreError::TooNew(_))));
    }
}
