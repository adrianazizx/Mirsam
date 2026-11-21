//! Safha core engine.
//!
//! UI-independent: nothing in this crate may depend on Tauri, a webview, or any
//! windowing API. The desktop shell, the future CLI, and tests all drive the
//! same types.
//!
//! Milestone 0 surface: bind PDFium, open a document, report page geometry,
//! render a page (or a tile of it) to an RGBA bitmap, and read PDFium's raw
//! text. Everything else in `docs/SPEC.md` section 9 lands in later milestones.

mod bitmap;
mod document;
mod engine;
mod error;

pub use bitmap::Bitmap;
pub use document::{Document, PageSize, TileRequest};
pub use engine::{Engine, PdfiumSource};
pub use error::{Error, Result};
