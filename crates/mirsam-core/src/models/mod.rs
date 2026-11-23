//! OCR models and the runtime that runs them: what can be downloaded, what is
//! installed, and installing it (spec §8: models are downloaded on demand, not
//! bundled; the download is resumable and its size explained. ADR 0034).
//!
//! - [`Catalog`]: the pinned [`Package`]s for this platform. Each file has its
//!   URL, size and SHA-256; each package its licence and [`Role`].
//! - [`ModelStore`]: the install folder in the app's data directory. It reports
//!   what is installed, installs a package (resumable download, checksum,
//!   disk-space check, cancel, atomic move into place) and deletes one.
//! - [`Downloader`]: HTTPS with resume (`Range`), retries and timeouts.
//! - [`Hardware`]: the accelerators llama.cpp could use here, and the memory.
//!
//! The OCR pipeline finds its files through [`ModelStore::installed`]: the 4B's
//! folder holds exactly the files `ocr::ServerConfig::for_model` expects.
//!
//! The store's folder:
//!
//! ```text
//! <root>/<package id>/              an installed package and its receipt
//! <root>/.downloads/<sha256>.part   a download in progress, kept to resume
//! <root>/.staging/<package id>-<n>/ a package being put together
//! <root>/.trash/                    removed packages, deleted next
//! ```

mod archive;
mod catalog;
mod download;
mod hardware;
mod store;
#[cfg(test)]
mod test_server;

use std::io;

pub use catalog::{
    Arch, Asset, Catalog, DRAFT, ENSEMBLE, Os, PRIMARY, Package, Platform, RUNTIME, Role, Unpack,
    draft_package, runtime_package,
};
pub use download::{DownloadConfig, Downloader};
pub use hardware::{Accelerator, Hardware};
pub use store::{InstalledPackage, ModelStore, PackageState, PackageStatus, Phase, Progress};

/// Why a model operation failed. [`ModelError::code`] is the stable code the UI
/// maps to a localized message.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// No package with this id in the catalog.
    #[error("no OCR model called {0:?}")]
    UnknownPackage(String),

    /// The download stopped: offline, refused, timed out, or the server failed.
    /// What was received is kept, so trying again resumes.
    #[error("the download failed: {0}")]
    Network(String),

    /// The server answered with an HTTP error.
    #[error("the server answered {status} for {url}")]
    Http { status: u16, url: String },

    /// What arrived is not the pinned file (size or SHA-256 differ). It is
    /// deleted; trying again starts over.
    #[error("{name} does not match its checksum")]
    ChecksumMismatch { name: String },

    /// Not enough free space for the download and the install.
    #[error("{needed} bytes are needed, {available} are free")]
    DiskFull { needed: u64, available: u64 },

    /// The user cancelled. What was received is kept.
    #[error("the download was cancelled")]
    Cancelled,

    /// A runtime archive could not be unpacked, or holds something unsafe.
    #[error("the archive could not be unpacked: {0}")]
    Archive(String),

    /// Reading or writing the install folder failed.
    #[error("the model folder: {0}")]
    Io(#[source] io::Error),
}

impl ModelError {
    pub fn code(&self) -> &'static str {
        match self {
            ModelError::UnknownPackage(_) => "model_unknown",
            ModelError::Network(_) | ModelError::Http { .. } => "model_download_failed",
            ModelError::ChecksumMismatch { .. } => "model_checksum_mismatch",
            ModelError::DiskFull { .. } => "model_disk_full",
            ModelError::Cancelled => "cancelled",
            ModelError::Archive(_) => "model_archive_invalid",
            ModelError::Io(_) => "model_install_failed",
        }
    }
}

impl From<io::Error> for ModelError {
    /// A full disk is its own error: the UI says how much space is needed.
    fn from(err: io::Error) -> Self {
        if err.kind() == io::ErrorKind::StorageFull {
            ModelError::DiskFull { needed: 0, available: 0 }
        } else {
            ModelError::Io(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        let cases = [
            (ModelError::UnknownPackage("x".into()), "model_unknown"),
            (ModelError::Network("offline".into()), "model_download_failed"),
            (ModelError::Http { status: 404, url: "u".into() }, "model_download_failed"),
            (ModelError::ChecksumMismatch { name: "f".into() }, "model_checksum_mismatch"),
            (ModelError::DiskFull { needed: 2, available: 1 }, "model_disk_full"),
            (ModelError::Cancelled, "cancelled"),
            (ModelError::Archive("bad".into()), "model_archive_invalid"),
            (ModelError::Io(io::Error::other("x")), "model_install_failed"),
        ];
        for (err, code) in cases {
            assert_eq!(err.code(), code, "{err}");
        }
        let full: ModelError = io::Error::from(io::ErrorKind::StorageFull).into();
        assert_eq!(full.code(), "model_disk_full");
    }
}
