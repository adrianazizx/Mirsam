//! Safha core engine.
//!
//! UI-independent: nothing in this crate may depend on Tauri, a webview, or any
//! windowing API. The desktop shell, the future CLI, and tests all drive the
//! same types.
//!
//! Surface so far: bind PDFium, open documents, report page geometry, render
//! through a prioritized queue ([`render`]), extract text in logical order
//! ([`text`]), search it ([`search`]), read the outline and page labels, map
//! page space to the displayed page ([`view`]), and keep local app state
//! ([`store`]).

mod bitmap;
pub mod catalog;
mod document;
pub mod edit;
mod engine;
mod error;
pub mod outline;
mod pdf;
pub mod render;
pub mod search;
pub mod store;
pub mod text;
pub mod view;

pub use bitmap::{BMP_HEADER_LEN, Bitmap};
pub use document::LoadedPage;
pub use document::{Document, PageSize, TileRequest};
pub use engine::{Engine, PdfiumSource};
pub use error::{Error, Result};
pub use render::RenderService;
