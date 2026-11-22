//! Saving (spec section 9, ADR 0015): nothing is ever written in place.
//!
//! 1. **What to write.**
//!    - A signed document: its bytes as they are, the file as opened plus each
//!      edit's incremental update. Its signatures stay valid.
//!    - Any other document: rewritten in full by PDFium. That drops what edits
//!      left behind in earlier revisions, such as the content of a deleted
//!      page, which must not survive in the file.
//! 2. **Validate** the bytes by opening them again (same password) and
//!    checking the page count.
//! 3. **Write** them to a temporary file beside the target, flush it to disk,
//!    and rename it over the target. A crash or a full disk leaves the old
//!    file intact.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::document::Document;
use crate::error::{Error, Result};

/// How a document was saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    /// Rewritten in full, cleaned of earlier revisions.
    Rewritten,
    /// Edits appended to the original bytes, keeping its signatures valid.
    Incremental,
}

/// Saves `document` to `path`.
pub fn save(document: &Document, path: &Path) -> Result<SaveMode> {
    let (bytes, mode) = if document.is_signed() {
        (document.bytes().to_vec(), SaveMode::Incremental)
    } else {
        (document.to_pdf_bytes()?, SaveMode::Rewritten)
    };
    let reopened = document
        .revision(Arc::from(bytes.as_slice()))
        .map_err(|_| Error::SaveRejected("the saved file would not open"))?;
    if reopened.page_count() != document.page_count() {
        return Err(Error::SaveRejected("the saved file would have a different number of pages"));
    }
    write_atomically(path, &bytes)?;
    Ok(mode)
}

/// Saves `document` to `path` with its protection changed (ADR 0043): a
/// password set, changed or removed, or its permissions. Returns the document
/// as the saved file reads, opened with the new password, for the caller to
/// show from now on.
///
/// The saved file is opened again with the new password (and must have the
/// same pages) before anything is written; with a wrong cipher or a bad
/// password nothing is.
pub fn save_secured(
    document: &Document,
    path: &Path,
    change: &crate::security::SecurityChange,
) -> Result<Document> {
    use crate::security::SecurityChange;
    let bytes: Arc<[u8]> = Arc::from(document.to_pdf_bytes_secured(change)?);
    let password = match change {
        SecurityChange::Keep => None,
        SecurityChange::Remove => None,
        SecurityChange::Set(protection) => protection.opening_password(),
    };
    // Kept: the password the document is open with.
    let password = match change {
        SecurityChange::Keep => document.password_in_use(),
        _ => password,
    };
    let reopened = document
        .reopen(Arc::clone(&bytes), password.as_deref())
        .map_err(|_| Error::SaveRejected("the saved file would not open with its password"))?;
    if reopened.page_count() != document.page_count() {
        return Err(Error::SaveRejected("the saved file would have a different number of pages"));
    }
    write_atomically(path, &bytes)?;
    Ok(reopened)
}

/// Writes `bytes` to `path` through a temporary file in the same directory
/// (a rename is atomic only within one file system), keeping an existing
/// file's permissions.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = temporary_path(path)?;
    let written = (|| -> std::io::Result<()> {
        let mut file = fs::File::create_new(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(existing) = fs::metadata(path) {
            fs::set_permissions(&temp, existing.permissions())?;
        }
        fs::rename(&temp, path)
    })();
    if let Err(err) = written {
        let _ = fs::remove_file(&temp);
        return Err(err.into());
    }
    // Make the rename itself durable.
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(dir) = fs::File::open(if dir.as_os_str().is_empty() { Path::new(".") } else { dir })
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn temporary_path(path: &Path) -> Result<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let name = path.file_name().ok_or(Error::SaveRejected("the path names no file"))?;
    let mut temp = std::ffi::OsString::from(".");
    temp.push(name);
    temp.push(format!(".safha-{}-{}.tmp", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    Ok(path.with_file_name(temp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_replace_the_file_and_leave_no_temporary_behind() {
        let dir = std::env::temp_dir().join(format!("safha-save-unit-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("a file.pdf");
        fs::write(&path, b"old").expect("write");
        write_atomically(&path, b"new contents").expect("saves");
        assert_eq!(fs::read(&path).expect("read"), b"new contents");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .expect("list")
            .filter_map(|entry| entry.ok().map(|e| e.file_name()))
            .filter(|name| name.to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // A directory that does not exist fails cleanly.
        assert!(write_atomically(&dir.join("missing/x.pdf"), b"x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
