//! Viewer commands: page geometry and tiles through the core's render service.
//!
//! The scheduling itself (priorities, cancellation, cache) lives in
//! `safha_core::render`; these commands only adapt it to IPC. Tiles travel as
//! raw bytes (ADR 0004): the [`BITMAP_HEADER_LEN`]-byte header from
//! `commands.rs`, whose last word carries [`FLAG_FROM_CACHE`], then RGBA.

use safha_core::render::{KeptRequest, Priority, RenderRequest, Rendered};
use safha_core::{PageSize, TileRequest};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Response;

use crate::commands::encode_bitmap_with_flags;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, Lane};

/// Header flag: the tile came from the cache rather than a fresh render.
pub const FLAG_FROM_CACHE: u32 = 1;

/// A page size on the wire. The engine's f32 values are widened to f64, which
/// is exact: JSON then carries the f32 value itself, so the UI's f32 page-pixel
/// arithmetic (`viewer/geometry.ts`) matches the engine's to the pixel. Sent as
/// shortest f32 decimals instead, a value parsed as f64 and rounded back to f32
/// could land one f32 step away.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PageSizeWire {
    width: f64,
    height: f64,
}

impl From<PageSize> for PageSizeWire {
    fn from(size: PageSize) -> Self {
        Self { width: f64::from(size.width), height: f64::from(size.height) }
    }
}

/// Sizes of every page in points, so the UI can lay out the whole document
/// before anything renders.
#[tauri::command]
pub async fn page_sizes(state: State<'_, AppState>, document_id: u32) -> CommandResult<Vec<PageSizeWire>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let sizes = tauri::async_runtime::spawn_blocking(move || document.page_sizes())
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))??;
    Ok(sizes.into_iter().map(PageSizeWire::from).collect())
}

/// Renders one rectangle of a page through the render queue. Rejects with
/// code `cancelled` when the viewport moved on before the render started.
/// `lane` keeps the thumbnail panel's requests apart from the page view's.
#[tauri::command]
pub async fn render_tile(
    state: State<'_, AppState>,
    document_id: u32,
    generation: u64,
    page: usize,
    tile: TileRequest,
    priority: Priority,
    lane: Option<Lane>,
) -> CommandResult<Response> {
    let (tx, mut rx) = tauri::async_runtime::channel(1);
    let document = lane.unwrap_or_default().render_id(document_id);
    let request = RenderRequest { document, generation, page, tile, priority };
    state.render.submit(request, move |outcome| {
        // Capacity 1 and exactly one send: this cannot fail for lack of room.
        let _ = tx.try_send(outcome);
    });
    let rendered =
        rx.recv().await.ok_or_else(|| CommandError::new("cancelled", "render service stopped"))??;
    Ok(Response::new(encode_rendered(&rendered)))
}

/// Moves the lane's viewport to `generation`: queued tiles not in `keep` are
/// cancelled, kept ones take their new priorities.
#[tauri::command]
pub async fn update_viewport(
    state: State<'_, AppState>,
    document_id: u32,
    generation: u64,
    keep: Vec<KeptRequest>,
    lane: Option<Lane>,
) -> CommandResult<()> {
    state.render.update_viewport(lane.unwrap_or_default().render_id(document_id), generation, &keep);
    Ok(())
}

pub fn encode_rendered(rendered: &Rendered) -> Vec<u8> {
    let flags = if rendered.from_cache { FLAG_FROM_CACHE } else { 0 };
    encode_bitmap_with_flags(&rendered.bitmap, u128::from(rendered.render_micros), flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_sizes_reach_json_as_exact_f32_values() {
        // 594.96 is the A4 width PDFium reports for the Chrome fixtures.
        let width = 594.96_f32;
        let json = serde_json::to_string(&PageSizeWire::from(PageSize { width, height: 841.92 }))
            .expect("serializes");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parses");
        let sent = parsed["width"].as_f64().expect("a number");
        // What the UI does: read an f64, round to f32 (Math.fround).
        assert_eq!(sent as f32, width);
        assert_eq!(sent, f64::from(width));
    }
}
