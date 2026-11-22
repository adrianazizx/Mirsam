//! What the render thread renders from: a document that can open pages, and an
//! open page that can render tiles. [`Document`] is the real implementation;
//! the scheduler's tests use a fake.

use std::sync::Arc;

use crate::bitmap::Bitmap;
use crate::document::{Document, LoadedPage, TileRequest};
use crate::error::Result;

/// A document the render thread can open pages of.
pub trait TileSource: Send + Sync {
    /// Opens (parses) one page so several tiles can be rendered from it.
    fn open_page(self: Arc<Self>, index: usize) -> Result<Box<dyn TilePage>>;
}

/// An open page. Dropping it releases the engine's parsed copy.
pub trait TilePage: Send {
    fn render_tile(&self, tile: TileRequest) -> Result<Bitmap>;
}

impl TileSource for Document {
    fn open_page(self: Arc<Self>, index: usize) -> Result<Box<dyn TilePage>> {
        Ok(Box::new(self.load_page(index)?))
    }
}

impl TilePage for LoadedPage {
    fn render_tile(&self, tile: TileRequest) -> Result<Bitmap> {
        LoadedPage::render_tile(self, tile)
    }
}
