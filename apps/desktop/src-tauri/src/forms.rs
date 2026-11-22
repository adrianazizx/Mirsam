//! Form filling, Fill & Sign, and the user's signatures (ADR 0045): thin
//! commands over `safha_core::forms`, `safha_core::annotate`, and the
//! state store. Positions come from the UI in displayed-page points and are
//! mapped to page space here; signature images are picked through a native
//! dialog, so the UI never sees a path.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use safha_core::annotate::{
    AnnotationEdit, DrawnSignature, FillText, MAX_SIGNATURE_IMAGE_BYTES, Mark, MarkKind, Signature,
    SignaturePoint, SignatureStamp, TypedSignature,
};
use safha_core::edit::Change;
use safha_core::fonts::FontFamily;
use safha_core::forms::{FormEdit, FormInfo};
use safha_core::store::{Store, StoredSignature};
use safha_core::view::ViewRect;

use crate::editing::{EditOutcome, edit_annotations};
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, lock};

fn store(state: &AppState) -> CommandResult<&Store> {
    state.store().ok_or_else(|| CommandError::new("store_unavailable", "the state store is not open"))
}

fn store_error(err: safha_core::store::StoreError) -> CommandError {
    CommandError::new("store_unavailable", err.to_string())
}

/// The document's form fields.
#[tauri::command]
pub async fn form_fields(state: State<'_, AppState>, document_id: u32) -> CommandResult<FormInfo> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    tauri::async_runtime::spawn_blocking(move || Ok(document.form()?))
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))?
}

/// Fills in a field: one undo step.
#[tauri::command]
pub async fn set_form_field(
    state: State<'_, AppState>,
    document_id: u32,
    field: u32,
    values: Vec<String>,
) -> CommandResult<EditOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (revision, update) = document.with_form_edits(&[FormEdit { field, values }])?;
    history.push_annotations(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(EditOutcome::new(page_count, &history, Change::Annotations))
}

/// Places Fill & Sign text (or a date) with its top-left corner displayed at (`x`, `y`).
#[tauri::command]
pub async fn add_fill_text(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    x: f32,
    y: f32,
    contents: String,
    size: Option<f32>,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |document| {
        let view = document.view_transform(page)?;
        let size = size.unwrap_or(FillText::DEFAULT_SIZE);
        let text = FillText::at(x, y, &contents, size, FillText::DEFAULT_COLOR, &view)?;
        Ok(vec![AnnotationEdit::AddFillText { page, text }])
    })
}

/// Places a tick, cross, or dot `size` points square, its top-left corner displayed at (`x`, `y`).
#[tauri::command]
pub async fn add_fill_mark(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    x: f32,
    y: f32,
    kind: MarkKind,
    size: Option<f32>,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |document| {
        let view = document.view_transform(page)?;
        let size = size.unwrap_or(Mark::DEFAULT_SIZE).clamp(4.0, 200.0);
        Ok(vec![AnnotationEdit::AddMark {
            page,
            mark: Mark::at(x, y, size, kind, Mark::DEFAULT_COLOR, &view),
        }])
    })
}

/// Places a kept signature `width` points wide, its top-left corner displayed at (`x`, `y`).
#[tauri::command]
pub async fn place_signature(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    x: f32,
    y: f32,
    width: f32,
    signature: i64,
) -> CommandResult<EditOutcome> {
    let stored = store(&state)?
        .signature(signature)
        .map_err(store_error)?
        .ok_or_else(|| CommandError::new("invalid_edit", "no such signature"))?;
    edit_annotations(&state, document_id, |document| {
        let view = document.view_transform(page)?;
        let color = match stored.signature {
            Signature::Image(_) => [0.0; 3],
            _ => Signature::DEFAULT_COLOR,
        };
        let stamp = SignatureStamp::at(x, y, width, stored.signature, color, &view)?;
        Ok(vec![AnnotationEdit::AddSignature { page, stamp }])
    })
}

/// Moves or resizes a Fill & Sign item to a box on the displayed page.
#[tauri::command]
pub async fn set_annotation_rect(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    index: usize,
    rect: ViewRect,
) -> CommandResult<EditOutcome> {
    edit_annotations(&state, document_id, |document| {
        let view = document.view_transform(page)?;
        let corners = [(rect.x, rect.y), (rect.x + rect.width, rect.y + rect.height)];
        let [(x0, y0), (x1, y1)] = corners.map(|(x, y)| view.page_point(x, y));
        let rect = [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)];
        Ok(vec![AnnotationEdit::SetRect { page, index, rect }])
    })
}

/// A kept signature as the UI lists and previews it. Images are shown from
/// `safha://localhost/signature?id=N`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureWire {
    id: i64,
    initials: bool,
    kind: &'static str,
    /// Width over height, for placing and previewing.
    aspect: f32,
    /// A drawn one's strokes (`[x, y]`, y down) and pen width.
    strokes: Vec<Vec<[f32; 2]>>,
    pen: f32,
    /// A typed one's text and family.
    text: Option<String>,
    family: Option<FontFamily>,
}

fn wire(stored: StoredSignature) -> SignatureWire {
    let aspect = stored.signature.natural_size().map_or(3.0, |(w, h)| w / h.max(0.01));
    let mut out = SignatureWire {
        id: stored.id,
        initials: stored.initials,
        kind: "image",
        aspect,
        strokes: Vec::new(),
        pen: 0.0,
        text: None,
        family: None,
    };
    match stored.signature {
        Signature::Drawn(drawn) => {
            out.kind = "drawn";
            out.strokes = drawn.strokes.iter().map(|s| s.iter().map(|p| [p.x, p.y]).collect()).collect();
            out.pen = drawn.width;
        }
        Signature::Typed(typed) => {
            out.kind = "typed";
            out.text = Some(typed.text);
            out.family = Some(typed.family);
        }
        Signature::Image(_) => {}
    }
    out
}

/// The kept signatures and initials, newest first.
#[tauri::command]
pub async fn signatures(state: State<'_, AppState>) -> CommandResult<Vec<SignatureWire>> {
    Ok(store(&state)?.signatures().map_err(store_error)?.into_iter().map(wire).collect())
}

fn keep(state: &AppState, initials: bool, signature: Signature) -> CommandResult<SignatureWire> {
    signature.check()?;
    let store = store(state)?;
    let id = store.save_signature(initials, &signature).map_err(store_error)?;
    let stored = store
        .signature(id)
        .map_err(store_error)?
        .ok_or_else(|| CommandError::new("store_unavailable", "not kept"))?;
    Ok(wire(stored))
}

/// A point of a drawn signature, in the drawing pad's pixels (y down).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PadPoint {
    x: f32,
    y: f32,
    pressure: f32,
}

/// Keeps a drawn signature.
#[tauri::command]
pub async fn save_drawn_signature(
    state: State<'_, AppState>,
    initials: bool,
    strokes: Vec<Vec<PadPoint>>,
    width: f32,
) -> CommandResult<SignatureWire> {
    let strokes = strokes
        .into_iter()
        .map(|stroke| {
            stroke.into_iter().map(|p| SignaturePoint { x: p.x, y: p.y, pressure: p.pressure }).collect()
        })
        .collect();
    keep(&state, initials, Signature::Drawn(DrawnSignature { strokes, width }))
}

/// Keeps a typed signature.
#[tauri::command]
pub async fn save_typed_signature(
    state: State<'_, AppState>,
    initials: bool,
    text: String,
    family: FontFamily,
) -> CommandResult<SignatureWire> {
    keep(&state, initials, Signature::Typed(TypedSignature { text: text.trim().to_string(), family }))
}

/// Asks for a PNG or JPEG and keeps it as a signature; `None` when the user cancels.
#[tauri::command]
pub async fn import_signature_image(
    app: AppHandle,
    state: State<'_, AppState>,
    initials: bool,
) -> CommandResult<Option<SignatureWire>> {
    let picked = app.dialog().file().add_filter("Image", &["png", "jpg", "jpeg"]).blocking_pick_file();
    let Some(path) = picked.and_then(|file| file.into_path().ok()) else { return Ok(None) };
    let too_large = || CommandError::new("invalid_edit", "a signature image is at most 8 MB");
    let length =
        std::fs::metadata(&path).map_err(|err| CommandError::new("io_error", err.to_string()))?.len();
    if length > MAX_SIGNATURE_IMAGE_BYTES as u64 {
        return Err(too_large());
    }
    let bytes = std::fs::read(&path).map_err(|err| CommandError::new("io_error", err.to_string()))?;
    keep(&state, initials, Signature::Image(bytes)).map(Some)
}

/// Forgets a kept signature.
#[tauri::command]
pub async fn delete_signature(state: State<'_, AppState>, id: i64) -> CommandResult<()> {
    store(&state)?.delete_signature(id).map_err(store_error)
}

/// `safha://localhost/signature?id=N`: a kept image signature's own bytes,
/// which the webview decodes itself (ADR 0011).
pub fn signature_image(state: &AppState, uri: &tauri::http::Uri) -> Option<(&'static str, Vec<u8>)> {
    let id = uri.query()?.split('&').find_map(|pair| pair.strip_prefix("id="))?.parse::<i64>().ok()?;
    let stored = state.store()?.signature(id).ok()??;
    let Signature::Image(bytes) = stored.signature else { return None };
    let kind = if bytes.starts_with(b"\x89PNG") { "image/png" } else { "image/jpeg" };
    Some((kind, bytes))
}
