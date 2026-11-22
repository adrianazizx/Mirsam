use std::path::PathBuf;

use pdfium_render::prelude::{PdfiumError, PdfiumInternalError};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The PDFium shared library was not found in any of the searched locations.
    #[error("PDFium library not found (searched: {searched:?})")]
    PdfiumNotFound { searched: Vec<PathBuf> },

    /// The PDFium shared library was found but could not be loaded.
    #[error("failed to load PDFium from {path}: {reason}")]
    PdfiumLoad { path: PathBuf, reason: String },

    /// The document is encrypted and no password, or a wrong one, was supplied.
    #[error("a password is required to open this document")]
    PasswordRequired,

    /// The bytes are not a PDF that PDFium can parse.
    #[error("the file is not a readable PDF")]
    InvalidPdf,

    #[error("page {index} is out of range (document has {count} pages)")]
    PageOutOfRange { index: usize, count: usize },

    #[error("invalid render request: {0}")]
    InvalidRenderRequest(&'static str),

    /// An edit this file's structure does not allow yet (for example, its page
    /// tree sits in an encrypted object stream, which the editor cannot read).
    #[error("this file cannot be edited: {0}")]
    EditUnsupported(&'static str),

    /// An edit that does not fit the document (no pages, a page out of range).
    #[error("invalid edit: {0}")]
    InvalidEdit(&'static str),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PDFium error: {0}")]
    Pdfium(String),
}

impl From<PdfiumError> for Error {
    fn from(err: PdfiumError) -> Self {
        match err {
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
                Error::PasswordRequired
            }
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => Error::InvalidPdf,
            other => Error::Pdfium(format!("{other:?}")),
        }
    }
}
