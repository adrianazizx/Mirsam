use std::path::Path;

use safha_core::{Engine, PdfiumSource};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::CommandResult;
use crate::state::AppState;

/// Length of the little-endian header that precedes the RGBA bytes in a
/// bitmap response (`render_tile`): width, height, render time in
/// microseconds, flags (`viewer::FLAG_*`).
pub const BITMAP_HEADER_LEN: usize = 16;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    devtools: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    id: u32,
    /// File name for display. The UI never receives the full path.
    name: String,
    page_count: usize,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo { version: env!("CARGO_PKG_VERSION"), devtools: cfg!(feature = "devtools") }
}

/// Shows the native file picker and opens the chosen PDF.
/// Returns `None` when the user cancels.
#[tauri::command]
pub async fn open_document_dialog(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<DocumentInfo>> {
    let picked = app.dialog().file().add_filter("PDF", &["pdf"]).blocking_pick_file();
    let Some(path) = picked.and_then(|file| file.into_path().ok()) else {
        return Ok(None);
    };
    open_path(&state, &path).map(Some)
}

/// Opens the document the app was launched with, if any. Only the first call
/// returns it, so a UI reload does not open a second copy.
#[tauri::command]
pub async fn take_startup_document(state: State<'_, AppState>) -> CommandResult<Option<DocumentInfo>> {
    match state.take_startup_document() {
        Some(path) => open_path(&state, &path).map(Some),
        None => Ok(None),
    }
}

/// Closes a document: its queued renders are cancelled and its cached tiles dropped.
#[tauri::command]
pub fn close_document(state: State<'_, AppState>, document_id: u32) -> bool {
    state.render.close_document(document_id.into());
    state.remove_document(document_id)
}

/// A bitmap as raw bytes: a [`BITMAP_HEADER_LEN`]-byte header followed by
/// straight RGBA. Binary all the way to the webview — never base64 or JSON
/// (spec section 9, "Rendering design").
#[cfg(any(test, feature = "devtools"))]
pub fn encode_bitmap(bitmap: &safha_core::Bitmap, render_micros: u128) -> Vec<u8> {
    encode_bitmap_with_flags(bitmap, render_micros, 0)
}

pub fn encode_bitmap_with_flags(bitmap: &safha_core::Bitmap, render_micros: u128, flags: u32) -> Vec<u8> {
    let rgba = bitmap.as_rgba();
    let mut out = Vec::with_capacity(BITMAP_HEADER_LEN + rgba.len());
    out.extend_from_slice(&bitmap.width().to_le_bytes());
    out.extend_from_slice(&bitmap.height().to_le_bytes());
    out.extend_from_slice(&u32::try_from(render_micros).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(rgba);
    out
}

pub fn open_path(state: &AppState, path: &Path) -> CommandResult<DocumentInfo> {
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let document = engine.open_file(path, None)?;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let (id, document) = state.insert_document(document);
    state.render.open_document(id.into(), document.clone());
    Ok(DocumentInfo { id, name, page_count: document.page_count() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire format is a contract with `apps/desktop/src/backend/bitmap.ts`.
    #[test]
    fn bitmap_payload_is_a_16_byte_little_endian_header_then_rgba() {
        // A one-page PDF with a 3x2 pt page, rendered at 1 px/pt.
        let body = "<< /Type /Catalog /Pages 2 0 R >>";
        let objects = [
            body.to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );

        let engine = Engine::global(&PdfiumSource::Auto).expect("PDFium is available");
        let bitmap = engine.open_bytes(pdf, None).expect("opens").render_page(0, 1.0).expect("renders");
        let payload = encode_bitmap(&bitmap, 1234);

        assert_eq!(payload.len(), BITMAP_HEADER_LEN + 3 * 2 * 4);
        assert_eq!(payload[0..4], 3u32.to_le_bytes());
        assert_eq!(payload[4..8], 2u32.to_le_bytes());
        assert_eq!(payload[8..12], 1234u32.to_le_bytes());
        assert_eq!(payload[12..16], [0; 4]);
        // A blank page is opaque white.
        assert!(payload[BITMAP_HEADER_LEN..].iter().all(|byte| *byte == 255));
        // The last header word carries flags (bit 0: served from the tile cache).
        let flagged = encode_bitmap_with_flags(&bitmap, 0, crate::viewer::FLAG_FROM_CACHE);
        assert_eq!(flagged[12..16], 1u32.to_le_bytes());
    }
}
