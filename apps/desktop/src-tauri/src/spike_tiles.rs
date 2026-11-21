//! Milestone 0 spike: how should rendered tiles travel from the core to the
//! webview? (docs/SPEC.md section 9, "Rendering design"; results in
//! docs/adr/0004-tile-transport.md.)
//!
//! Throwaway measurement code, compiled only with the `devtools` feature. Each
//! command returns the same tile through a different transport so the UI can
//! time them against each other.

use std::time::Instant;

use base64::Engine as _;
use safha_core::TileRequest;
use tauri::ipc::{Channel, InvokeResponseBody, Response};
use tauri::{Manager, State, UriSchemeContext, UriSchemeResponder, http};

use crate::commands::encode_bitmap;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

fn render(state: &AppState, document_id: u32, page: usize, tile: TileRequest) -> CommandResult<Vec<u8>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let started = Instant::now();
    let bitmap = document.render_tile(page, tile)?;
    Ok(encode_bitmap(&bitmap, started.elapsed().as_micros()))
}

/// Transport 1: raw bytes as the command's response body.
#[tauri::command]
pub async fn spike_tile_raw(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    tile: TileRequest,
) -> CommandResult<Response> {
    Ok(Response::new(render(&state, document_id, page, tile)?))
}

/// Transport 2: raw bytes pushed over a Tauri channel.
#[tauri::command]
pub async fn spike_tile_channel(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    tile: TileRequest,
    channel: Channel<InvokeResponseBody>,
) -> CommandResult<()> {
    let bytes = render(&state, document_id, page, tile)?;
    channel
        .send(InvokeResponseBody::Raw(bytes))
        .map_err(|err| CommandError::new("engine_error", err.to_string()))
}

/// Anti-pattern baseline: base64 inside a JSON string, which the spec forbids.
#[tauri::command]
pub async fn spike_tile_base64(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    tile: TileRequest,
) -> CommandResult<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(render(&state, document_id, page, tile)?))
}

/// The startup document's bytes, for the PDFium-in-WebAssembly candidate.
#[tauri::command]
pub fn spike_pdf_bytes() -> CommandResult<Response> {
    let path = std::env::var_os("SAFHA_OPEN")
        .ok_or_else(|| CommandError::new("io_error", "SAFHA_OPEN is not set"))?;
    std::fs::read(path).map(Response::new).map_err(|err| CommandError::new("io_error", err.to_string()))
}

/// Resident set size of the shell process in kilobytes.
#[tauri::command]
pub fn spike_rss_kb() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Transport 3: a custom URI scheme the webview can `fetch()`.
/// `safha-tile://localhost/<document>/<page>?scale=&x=&y=&w=&h=`
pub fn protocol<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = parse(request.uri()).and_then(|(document_id, page, tile)| {
            render(&app.state::<AppState>(), document_id, page, tile).map_err(|err| err.detail)
        });
        let response = match result {
            Ok(bytes) => http::Response::builder()
                .header(http::header::CONTENT_TYPE, "application/octet-stream")
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(http::header::CACHE_CONTROL, "no-store")
                .body(bytes),
            Err(detail) => http::Response::builder()
                .status(http::StatusCode::BAD_REQUEST)
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(detail.into_bytes()),
        };
        if let Ok(response) = response {
            responder.respond(response);
        }
    });
}

fn parse(uri: &http::Uri) -> Result<(u32, usize, TileRequest), String> {
    let mut segments = uri.path().trim_matches('/').split('/');
    let mut next = |what: &str| segments.next().ok_or_else(|| format!("missing {what}"));
    let document_id = next("document")?.parse().map_err(|_| "bad document id")?;
    let page = next("page")?.parse().map_err(|_| "bad page")?;

    let query: std::collections::HashMap<&str, &str> =
        uri.query().unwrap_or_default().split('&').filter_map(|pair| pair.split_once('=')).collect();
    let field = |name: &str| query.get(name).copied().ok_or_else(|| format!("missing {name}"));
    let tile = TileRequest {
        scale: field("scale")?.parse().map_err(|_| "bad scale")?,
        x: field("x")?.parse().map_err(|_| "bad x")?,
        y: field("y")?.parse().map_err(|_| "bad y")?,
        width: field("w")?.parse().map_err(|_| "bad w")?,
        height: field("h")?.parse().map_err(|_| "bad h")?,
    };
    Ok((document_id, page, tile))
}
