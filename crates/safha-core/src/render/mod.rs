//! Render scheduling (docs/SPEC.md section 9, "Rendering design").
//!
//! PDFium is single-threaded per process (ADR 0002), so a pool of render
//! threads over one PDFium would only queue on its mutex. Instead there is one
//! render thread fed by one prioritized queue:
//!
//! - **Priority.** Placeholders (low-resolution stand-ins for pages that are on
//!   screen with nothing drawn) first, then visible tiles, then prefetch; within
//!   a class, nearest to the viewport centre first; ties in arrival order.
//! - **Cancellation by generation.** Each document has a viewport generation.
//!   [`RenderService::update_viewport`] advances it and names the requests that
//!   are still wanted; every other request from an older generation that has not
//!   started is dropped and its caller told [`RenderError::Cancelled`]. A
//!   request that arrives with a generation older than the document's current
//!   one is cancelled on arrival. A render that has already started always runs
//!   to completion and is cached: its pixels are still correct.
//! - **Cache.** Finished tiles go into an LRU cache keyed by (document,
//!   document revision, page, scale, tile rectangle) with a byte cap. Identical
//!   requests in flight are coalesced into one render.
//! - **Loaded pages.** Parsing a page costs more than rasterizing a 512 px tile
//!   of it, so the render thread keeps the last few pages it used open.
//!
//! Nothing here knows about tiles grids, zoom, or the UI: callers name the pixel
//! rectangle they want, as in [`TileRequest`].

mod cache;
mod service;
mod source;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::document::TileRequest;
use crate::error::Error;

pub use service::{RenderConfig, RenderService, RenderStats};
pub use source::{TilePage, TileSource};

/// Caller-chosen identifier of a document registered with a [`RenderService`].
pub type DocumentId = u64;

/// How urgently a request is needed. Lower sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Urgency {
    /// A cheap low-resolution render of a page that is on screen and would
    /// otherwise be blank.
    Placeholder,
    /// A full-resolution tile that intersects the viewport.
    Visible,
    /// Near the viewport: wanted soon if the user keeps scrolling.
    Prefetch,
}

/// Scheduling priority of a request: urgency first, then distance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Priority {
    pub urgency: Urgency,
    /// Distance from the centre of the viewport, in any unit consistent across
    /// one document's requests (the UI uses CSS pixels). Nearer renders first.
    pub distance: f32,
}

impl Priority {
    pub const fn new(urgency: Urgency, distance: f32) -> Self {
        Self { urgency, distance }
    }

    /// Total order: urgency, then distance (NaN sorts last).
    fn cmp_key(&self, other: &Self) -> std::cmp::Ordering {
        self.urgency.cmp(&other.urgency).then_with(|| {
            let a = if self.distance.is_nan() { f32::INFINITY } else { self.distance };
            let b = if other.distance.is_nan() { f32::INFINITY } else { other.distance };
            a.total_cmp(&b)
        })
    }

    /// The more urgent of two priorities.
    fn max_urgency(self, other: Self) -> Self {
        if other.cmp_key(&self).is_lt() { other } else { self }
    }
}

/// One rectangle of one page, as the cache and the queue identify it. The scale
/// is compared bit for bit, so callers should derive it the same way each time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub page: usize,
    pub scale_bits: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl TileKey {
    pub fn new(page: usize, tile: &TileRequest) -> Self {
        Self {
            page,
            scale_bits: tile.scale.to_bits(),
            x: tile.x,
            y: tile.y,
            width: tile.width,
            height: tile.height,
        }
    }
}

/// A request for one rendered rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    pub document: DocumentId,
    /// The caller's viewport generation when it asked (see the module docs).
    pub generation: u64,
    pub page: usize,
    pub tile: TileRequest,
    pub priority: Priority,
}

/// A request that is still wanted after a viewport change, with its new priority.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct KeptRequest {
    pub page: usize,
    pub tile: TileRequest,
    pub priority: Priority,
}

/// A finished render.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub bitmap: Arc<Bitmap>,
    /// Time PDFium spent producing this bitmap (when it was first rendered).
    pub render_micros: u32,
    /// True when this response was served from the cache without rendering.
    pub from_cache: bool,
}

/// Why a request produced no bitmap.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RenderError {
    /// The request was dropped before it started because the viewport moved
    /// on, the document was closed or changed, or the service shut down.
    #[error("the render request was cancelled")]
    Cancelled,
    /// No document with this id is registered with the service.
    #[error("no document {0} is registered for rendering")]
    UnknownDocument(DocumentId),
    /// The engine rejected or failed the render. Shared by every caller that
    /// was waiting on the same render.
    #[error(transparent)]
    Engine(Arc<Error>),
}

pub type RenderOutcome = std::result::Result<Rendered, RenderError>;
