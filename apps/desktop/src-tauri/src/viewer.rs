//! Viewer commands and the tile protocol, over the core's render service.
//!
//! The scheduling itself (priorities, cancellation, cache) lives in
//! `safha_core::render`; this module only adapts it. Tiles are served by the
//! `safha` URI scheme as plain BMP images (ADR 0011, superseding the transport
//! of ADR 0004): the webview fetches them and decodes them natively, so the
//! pixels never become JavaScript buffers. That keeps WebKit's memory flat
//! where raw command responses grew it by hundreds of megabytes.

use std::collections::HashMap;

use safha_core::render::{KeptRequest, Priority, RenderError, RenderOutcome, RenderRequest, Urgency};
use safha_core::{PageSize, TileRequest};
use serde::Serialize;
use tauri::{Manager, State, UriSchemeContext, UriSchemeResponder, http};

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, Lane};

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

/// The tile protocol: `safha://localhost/tile?doc=&lane=&gen=&page=&scale=&x=&y=&w=&h=&u=&d=`
/// (on Windows `http://safha.localhost/tile?…`).
///
/// Submits the tile to the render queue exactly as a command would and answers
/// when it is rendered: `200` with an `image/bmp` body and the headers
/// `X-Render-Micros` and `X-From-Cache`; otherwise an error status whose
/// `X-Error` header is the error code (`cancelled` when the viewport moved on
/// before the render started, which the UI drops silently).
pub fn tile_protocol<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    if request.uri().path().trim_matches('/') == "document" {
        document_response(ctx.app_handle(), request.uri(), responder);
        return;
    }
    if request.uri().path().trim_matches('/') == "signature" {
        let response =
            match crate::forms::signature_image(&ctx.app_handle().state::<AppState>(), request.uri()) {
                Some((kind, bytes)) => http::Response::builder()
                    .header(http::header::CONTENT_TYPE, kind)
                    .header(http::header::CACHE_CONTROL, "no-store")
                    .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(bytes)
                    .unwrap_or_else(|err| {
                        error_response(
                            http::StatusCode::INTERNAL_SERVER_ERROR,
                            "engine_error",
                            &err.to_string(),
                        )
                    }),
                None => error_response(
                    http::StatusCode::NOT_FOUND,
                    "unknown_signature",
                    "no such signature image",
                ),
            };
        responder.respond(response);
        return;
    }
    let request = match parse_tile_request(request.uri()) {
        Ok(request) => request,
        Err(detail) => {
            responder.respond(error_response(
                http::StatusCode::BAD_REQUEST,
                "invalid_render_request",
                &detail,
            ));
            return;
        }
    };
    let app = ctx.app_handle().clone();
    app.state::<AppState>().render.submit(request, move |outcome| {
        // Encoding copies a megabyte; keep it off the render thread.
        tauri::async_runtime::spawn_blocking(move || responder.respond(tile_response(outcome)));
    });
}

/// `safha://localhost/document?doc=N`: the document as PDF bytes, for a
/// window that shows it with the platform's own PDF viewer (printing).
fn document_response<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    uri: &http::Uri,
    responder: UriSchemeResponder,
) {
    let id = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .find_map(|pair| pair.strip_prefix("doc="))
        .and_then(|id| id.parse::<u32>().ok());
    let Some(document) = id.and_then(|id| app.state::<AppState>().document(id)) else {
        responder.respond(error_response(
            http::StatusCode::NOT_FOUND,
            "unknown_document",
            "no such document",
        ));
        return;
    };
    tauri::async_runtime::spawn_blocking(move || {
        let response = match document.to_pdf_bytes() {
            Ok(bytes) => http::Response::builder()
                .header(http::header::CONTENT_TYPE, "application/pdf")
                .header(http::header::CACHE_CONTROL, "no-store")
                .body(bytes)
                .unwrap_or_else(|err| {
                    error_response(http::StatusCode::INTERNAL_SERVER_ERROR, "engine_error", &err.to_string())
                }),
            Err(err) => {
                error_response(http::StatusCode::UNPROCESSABLE_ENTITY, "engine_error", &err.to_string())
            }
        };
        responder.respond(response);
    });
}

fn tile_response(outcome: RenderOutcome) -> http::Response<Vec<u8>> {
    match outcome {
        Ok(rendered) => http::Response::builder()
            .header(http::header::CONTENT_TYPE, "image/bmp")
            .header(http::header::CACHE_CONTROL, "no-store")
            .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(http::header::ACCESS_CONTROL_EXPOSE_HEADERS, "X-Render-Micros, X-From-Cache, X-Error")
            .header("X-Render-Micros", rendered.render_micros.to_string())
            .header("X-From-Cache", if rendered.from_cache { "1" } else { "0" })
            .body(rendered.bitmap.encode_bmp())
            .unwrap_or_else(|err| {
                error_response(http::StatusCode::INTERNAL_SERVER_ERROR, "engine_error", &err.to_string())
            }),
        Err(err) => {
            let error = CommandError::from(err.clone());
            let status = match err {
                RenderError::Cancelled => http::StatusCode::CONFLICT,
                RenderError::UnknownDocument(_) => http::StatusCode::NOT_FOUND,
                RenderError::Engine(_) => http::StatusCode::UNPROCESSABLE_ENTITY,
            };
            error_response(status, error.code, &error.detail)
        }
    }
}

fn error_response(status: http::StatusCode, code: &str, detail: &str) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(detail.as_bytes().to_vec());
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, http::HeaderValue::from_static("*"));
    headers.insert(http::header::ACCESS_CONTROL_EXPOSE_HEADERS, http::HeaderValue::from_static("X-Error"));
    headers.insert(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store"));
    if let Ok(value) = http::HeaderValue::from_str(code) {
        headers.insert("X-Error", value);
    }
    response
}

fn parse_tile_request(uri: &http::Uri) -> Result<RenderRequest, String> {
    if uri.path().trim_matches('/') != "tile" {
        return Err(format!("unknown path {}", uri.path()));
    }
    let query: HashMap<&str, &str> =
        uri.query().unwrap_or_default().split('&').filter_map(|pair| pair.split_once('=')).collect();
    fn field<T: std::str::FromStr>(query: &HashMap<&str, &str>, name: &str) -> Result<T, String> {
        query.get(name).ok_or_else(|| format!("missing {name}"))?.parse().map_err(|_| format!("bad {name}"))
    }
    let lane = match query.get("lane").copied().unwrap_or("main") {
        "main" => Lane::Main,
        "thumbnails" => Lane::Thumbnails,
        other => return Err(format!("unknown lane {other}")),
    };
    let urgency = match query.get("u").copied().unwrap_or("visible") {
        "placeholder" => Urgency::Placeholder,
        "visible" => Urgency::Visible,
        "thumbnail" => Urgency::Thumbnail,
        "prefetch" => Urgency::Prefetch,
        other => return Err(format!("unknown urgency {other}")),
    };
    // The scale travels as the shortest decimal of an f32 value; parsing it as
    // f64 and narrowing gives back exactly that f32 (the cache key compares bits).
    let scale = field::<f64>(&query, "scale")? as f32;
    Ok(RenderRequest {
        document: lane.render_id(field(&query, "doc")?),
        generation: field(&query, "gen")?,
        page: field(&query, "page")?,
        tile: TileRequest {
            scale,
            x: field(&query, "x")?,
            y: field(&query, "y")?,
            width: field(&query, "w")?,
            height: field(&query, "h")?,
        },
        priority: Priority::new(urgency, field::<f32>(&query, "d").unwrap_or(0.0)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_urls_parse_into_render_requests() {
        // As the UI builds them (backend/tauri.ts): an f32 scale's shortest decimal.
        let scale = 2.229_f32;
        let uri: http::Uri =
            format!("safha://localhost/tile?doc=3&lane=thumbnails&gen=1795000000000000&page=7&scale={scale}&x=512&y=0&w=512&h=300&u=prefetch&d=12.5")
                .parse()
                .expect("uri");
        let request = parse_tile_request(&uri).expect("parses");
        assert_eq!(request.document, Lane::Thumbnails.render_id(3));
        assert_eq!(request.generation, 1_795_000_000_000_000);
        assert_eq!(request.page, 7);
        assert_eq!(request.tile.scale.to_bits(), scale.to_bits());
        assert_eq!(
            (request.tile.x, request.tile.y, request.tile.width, request.tile.height),
            (512, 0, 512, 300)
        );
        assert_eq!(request.priority, Priority::new(Urgency::Prefetch, 12.5));
        assert!(parse_tile_request(&"safha://localhost/tile?doc=1".parse().expect("uri")).is_err());
        assert!(parse_tile_request(&"safha://localhost/other?doc=1".parse().expect("uri")).is_err());
    }

    /// A one-page PDF with a 3 x 2 pt blank page.
    fn blank_pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] >>",
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
        pdf
    }

    /// The response is a contract with `fetchTile` in `apps/desktop/src/backend/tauri.ts`.
    #[test]
    fn a_rendered_tile_is_a_bmp_with_its_timing_in_headers() {
        let engine =
            safha_core::Engine::global(&safha_core::PdfiumSource::Auto).expect("PDFium is available");
        let bitmap =
            engine.open_bytes(blank_pdf(), None).expect("opens").render_page(0, 1.0).expect("renders");
        let rendered = safha_core::render::Rendered {
            bitmap: std::sync::Arc::new(bitmap),
            render_micros: 1234,
            from_cache: true,
        };
        let response = tile_response(Ok(rendered));
        assert_eq!(response.status(), http::StatusCode::OK);
        let header =
            |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        assert_eq!(header("content-type").as_deref(), Some("image/bmp"));
        assert_eq!(header("X-Render-Micros").as_deref(), Some("1234"));
        assert_eq!(header("X-From-Cache").as_deref(), Some("1"));
        let body = response.body();
        assert_eq!(&body[..2], b"BM");
        assert_eq!(body.len(), safha_core::BMP_HEADER_LEN + 3 * 2 * 4);
        // A blank page is opaque white.
        assert!(body[safha_core::BMP_HEADER_LEN..].iter().all(|byte| *byte == 255));
    }

    #[test]
    fn a_cancelled_tile_is_a_409_with_its_code() {
        let response = tile_response(Err(RenderError::Cancelled));
        assert_eq!(response.status(), http::StatusCode::CONFLICT);
        assert_eq!(response.headers().get("X-Error").and_then(|v| v.to_str().ok()), Some("cancelled"));
    }

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
