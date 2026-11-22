//! Autosave and recovery (spec §9, ADR 0040): while a document has unsaved
//! edits, its current revision is kept as a recovery file in the app data
//! directory, so a crash, a power cut, or a killed process loses nothing.
//!
//! A recovery file is the document's current bytes: the file as opened with
//! each edit's incremental update appended (ADR 0014), so it is a valid PDF
//! by itself. An encrypted document's recovery file is still encrypted and
//! needs the password to open; the password is never written anywhere.
//!
//! What a recovery file belongs to (the original path, when it was written)
//! is kept in the state store (`Store::save_recovery`). Files are named by a
//! key the app chooses: letters, digits, and dashes only.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::{Error, Result};
use crate::save::write_atomically;

/// The directory of recovery files.
#[derive(Debug, Clone)]
pub struct RecoveryFiles {
    dir: PathBuf,
}

impl RecoveryFiles {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        let valid =
            !key.is_empty() && key.len() <= 64 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !valid {
            return Err(Error::InvalidEdit("a recovery key is letters, digits, and dashes"));
        }
        Ok(self.dir.join(format!("{key}.pdf")))
    }

    /// Writes (or replaces) the recovery file for `key`, atomically: a crash
    /// mid-write leaves the previous one.
    pub fn write(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.path(key)?;
        std::fs::create_dir_all(&self.dir)?;
        write_atomically(&path, bytes)
    }

    pub fn read(&self, key: &str) -> Result<Vec<u8>> {
        Ok(std::fs::read(self.path(key)?)?)
    }

    pub fn exists(&self, key: &str) -> bool {
        self.path(key).is_ok_and(|path| path.is_file())
    }

    /// Removes the recovery file for `key`, if there is one.
    pub fn remove(&self, key: &str) {
        if let Ok(path) = self.path(key) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A file's size and modification time: whether it changed since it was
/// opened (by another program, or another copy of this one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    /// Nanoseconds since the Unix epoch.
    pub modified: i64,
}

impl FileStamp {
    /// The stamp of the file at `path`, if it is there and readable.
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let since = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(Self { len: metadata.len(), modified: i64::try_from(since.as_nanos()).unwrap_or(i64::MAX) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "safha-recovery-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_recovery_file_is_written_read_and_removed() {
        let dir = scratch();
        let files = RecoveryFiles::new(dir.join("recovery"));
        assert!(!files.exists("1-2"));
        files.write("1-2", b"%PDF-1.7 first").expect("writes");
        files.write("1-2", b"%PDF-1.7 second").expect("replaces");
        assert_eq!(files.read("1-2").expect("reads"), b"%PDF-1.7 second");
        files.remove("1-2");
        assert!(!files.exists("1-2"));
        files.remove("1-2");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keys_cannot_reach_outside_the_directory() {
        let files = RecoveryFiles::new(scratch());
        for key in ["", "../x", "a/b", "a.b", &"x".repeat(65)] {
            assert!(matches!(files.write(key, b"x"), Err(Error::InvalidEdit(_))), "{key:?}");
        }
    }

    #[test]
    fn a_stamp_changes_when_the_file_does() {
        let dir = scratch();
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("a.pdf");
        std::fs::write(&path, b"one").expect("write");
        let first = FileStamp::of(&path).expect("stamp");
        assert_eq!(FileStamp::of(&path), Some(first));
        std::fs::write(&path, b"three").expect("write");
        assert_ne!(FileStamp::of(&path), Some(first));
        assert_eq!(FileStamp::of(&dir.join("missing.pdf")), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
