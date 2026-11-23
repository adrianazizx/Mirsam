//! Page objects, images and links (ADR 0051): thin commands over
//! `mirsam_core::objects` and `mirsam_core::links`. Images are picked through a
//! native dialog here, so the UI never sees a path; a link's web address is
//! opened here, only after the UI has shown it, and only `http`, `https` and
//! `mailto`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use mirsam_core::edit::Change;
use mirsam_core::links::{LinkEdit, LinkInfo, LinkTarget, NamedDestination};
use mirsam_core::objects::{
    Arrange, MAX_IMAGE_BYTES, NewImage, ObjectChange, ObjectEdit, ObjectRef, PageObject,
};
use mirsam_core::view::ViewRect;

use crate::editing::EditOutcome;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, lock};

/// A change to one object, as the UI sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ObjectEditArg {
    /// `[a b c d e f]` on the displayed page.
    Transform {
        matrix: [f64; 6],
    },
    Delete,
    Arrange {
        to: Arrange,
    },
    Crop {
        rect: Option<ViewRect>,
    },
}

impl From<ObjectEditArg> for ObjectEdit {
    fn from(arg: ObjectEditArg) -> Self {
        match arg {
            ObjectEditArg::Transform { matrix } => ObjectEdit::Transform { matrix },
            ObjectEditArg::Delete => ObjectEdit::Delete,
            ObjectEditArg::Arrange { to } => ObjectEdit::Arrange(to),
            ObjectEditArg::Crop { rect } => ObjectEdit::Crop { rect },
        }
    }
}

/// The document after an object edit, and where the object is now.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectEditOutcome {
    #[serde(flatten)]
    outcome: EditOutcome,
    /// The object's index among the page's objects afterwards; none once removed.
    index: Option<usize>,
}

/// The objects of a page, in drawing order (ADR 0051).
#[tauri::command]
pub async fn page_objects(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
) -> CommandResult<Vec<PageObject>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    Ok(document.page_objects(page)?)
}

fn change(state: &AppState, document_id: u32, change: &ObjectChange) -> CommandResult<ObjectEditOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (revision, update, index) = document.with_object_edit(change)?;
    history.push_content(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(ObjectEditOutcome { outcome: EditOutcome::new(page_count, &history, Change::Content), index })
}

/// Moves, resizes, rotates, crops, removes or reorders an object: one undo step.
#[tauri::command]
pub async fn edit_object(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    object: ObjectRefArg,
    edit: ObjectEditArg,
) -> CommandResult<ObjectEditOutcome> {
    change(&state, document_id, &ObjectChange { page, object: object.into(), edit: edit.into() })
}

/// An object as the UI names it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRefArg {
    index: usize,
    fingerprint: u32,
}

impl From<ObjectRefArg> for ObjectRef {
    fn from(arg: ObjectRefArg) -> Self {
        ObjectRef { index: arg.index, fingerprint: arg.fingerprint }
    }
}

/// Asks for a PNG or JPEG; `None` when the user cancels.
fn pick_image(app: &AppHandle) -> CommandResult<Option<Vec<u8>>> {
    let picked = app.dialog().file().add_filter("Image", &["png", "jpg", "jpeg"]).blocking_pick_file();
    let Some(path) = picked.and_then(|file| file.into_path().ok()) else { return Ok(None) };
    let length =
        std::fs::metadata(&path).map_err(|err| CommandError::new("io_error", err.to_string()))?.len();
    if length > MAX_IMAGE_BYTES as u64 {
        return Err(CommandError::new("image_too_large", "an image file is at most 64 MB"));
    }
    std::fs::read(&path).map(Some).map_err(|err| CommandError::new("io_error", err.to_string()))
}

/// Puts another picked image in an image's place: one undo step; `None` when cancelled.
#[tauri::command]
pub async fn replace_image(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    object: ObjectRefArg,
) -> CommandResult<Option<ObjectEditOutcome>> {
    let Some(data) = pick_image(&app)? else { return Ok(None) };
    change(
        &state,
        document_id,
        &ObjectChange { page, object: object.into(), edit: ObjectEdit::ReplaceImage { data } },
    )
    .map(Some)
}

/// A point on the displayed page.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PointArg {
    x: f32,
    y: f32,
}

/// Where a new image goes: centred on `at` (else the page's centre), at 96
/// pixels an inch, made smaller to fit half the page, and kept on the page.
pub(crate) fn image_rect(pixels: (u32, u32), page: (f32, f32), at: Option<PointArg>) -> ViewRect {
    let (pw, ph) = page;
    let natural = (pixels.0 as f32 * 0.75, pixels.1 as f32 * 0.75);
    let scale = (pw * 0.5 / natural.0).min(ph * 0.5 / natural.1).min(1.0);
    let (w, h) = ((natural.0 * scale).max(1.0), (natural.1 * scale).max(1.0));
    let (cx, cy) = at.map_or((pw / 2.0, ph / 2.0), |p| (p.x, p.y));
    let x = (cx - w / 2.0).clamp(0.0, (pw - w).max(0.0));
    let y = (cy - h / 2.0).clamp(0.0, (ph - h).max(0.0));
    ViewRect { x, y, width: w, height: h }
}

/// Places a picked image on a page: one undo step; `None` when cancelled.
#[tauri::command]
pub async fn insert_image(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    at: Option<PointArg>,
) -> CommandResult<Option<ObjectEditOutcome>> {
    let Some(data) = pick_image(&app)? else { return Ok(None) };
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let size = document.page_size(page)?;
    let pixels = mirsam_core::objects::image_pixels(&data)?;
    let rect = image_rect(pixels, (size.width, size.height), at);
    let (revision, update, index) = document.with_new_image(&NewImage { page, rect, data })?;
    history.push_content(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(Some(ObjectEditOutcome { outcome: EditOutcome::new(page_count, &history, Change::Content), index }))
}

/// A page's links and where they go.
#[tauri::command]
pub async fn page_links(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
) -> CommandResult<Vec<LinkInfo>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    Ok(document.page_links(page)?)
}

/// The document's named destinations.
#[tauri::command]
pub async fn named_destinations(
    state: State<'_, AppState>,
    document_id: u32,
) -> CommandResult<Vec<NamedDestination>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    Ok(document.named_destinations()?)
}

/// A link change, as the UI sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LinkEditArg {
    Add { page: usize, rects: Vec<ViewRect>, target: LinkTarget },
    SetTarget { page: usize, index: usize, target: LinkTarget },
    SetRect { page: usize, index: usize, rect: ViewRect },
    Remove { page: usize, index: usize },
}

impl From<LinkEditArg> for LinkEdit {
    fn from(arg: LinkEditArg) -> Self {
        match arg {
            LinkEditArg::Add { page, rects, target } => LinkEdit::Add { page, rects, target },
            LinkEditArg::SetTarget { page, index, target } => LinkEdit::SetTarget { page, index, target },
            LinkEditArg::SetRect { page, index, rect } => LinkEdit::SetRect { page, index, rect },
            LinkEditArg::Remove { page, index } => LinkEdit::Remove { page, index },
        }
    }
}

/// Adds, points elsewhere, moves or removes a link: one undo step.
#[tauri::command]
pub async fn edit_link(
    state: State<'_, AppState>,
    document_id: u32,
    edit: LinkEditArg,
) -> CommandResult<EditOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let (revision, update) = document.with_link_edit(&edit.into())?;
    history.push_annotations(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(EditOutcome::new(page_count, &history, Change::Annotations))
}

/// Opens a link's web or mail address in the system's browser or mail app,
/// after the UI has shown it to the user.
#[tauri::command]
pub async fn open_link(url: String) -> CommandResult<()> {
    let Some(url) = mirsam_core::links::openable(&url) else {
        return Err(CommandError::new("link_refused", "only web and e-mail addresses are opened"));
    };
    open::that_detached(&url).map_err(|err| CommandError::new("io_error", err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_arrive_in_the_shape_the_ui_sends() {
        let edit: ObjectEditArg =
            serde_json::from_str(r#"{"kind":"transform","matrix":[1,0,0,1,10,-5]}"#).expect("parses");
        assert!(matches!(edit, ObjectEditArg::Transform { matrix } if matrix[4] == 10.0));
        let edit: ObjectEditArg =
            serde_json::from_str(r#"{"kind":"arrange","to":"backward"}"#).expect("parses");
        assert!(matches!(edit, ObjectEditArg::Arrange { to: Arrange::Backward }));
        let edit: ObjectEditArg = serde_json::from_str(r#"{"kind":"crop","rect":null}"#).expect("parses");
        assert!(matches!(edit, ObjectEditArg::Crop { rect: None }));
        let link: LinkEditArg = serde_json::from_str(
            r#"{"kind":"add","page":2,"rects":[{"x":1,"y":2,"width":30,"height":4}],"target":{"kind":"named","name":"intro"}}"#,
        )
        .expect("parses");
        assert!(matches!(link, LinkEditArg::Add { page: 2, target: LinkTarget::Named { .. }, .. }));
        let link: LinkEditArg = serde_json::from_str(
            r#"{"kind":"setTarget","page":0,"index":3,"target":{"kind":"page","page":4}}"#,
        )
        .expect("parses");
        assert!(matches!(
            link,
            LinkEditArg::SetTarget { index: 3, target: LinkTarget::Page { page: 4 }, .. }
        ));
    }

    #[test]
    fn a_new_image_fits_half_the_page_where_it_was_asked_for() {
        // 1200 × 600 px is 900 × 450 pt: halved to fit half a 600 × 800 page's width.
        let rect = image_rect((1200, 600), (600.0, 800.0), None);
        assert_eq!((rect.width, rect.height), (300.0, 150.0));
        assert_eq!((rect.x, rect.y), (150.0, 325.0));
        // A small image keeps its size, centred on the point, kept on the page.
        let rect = image_rect((40, 40), (600.0, 800.0), Some(PointArg { x: 5.0, y: 790.0 }));
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (0.0, 770.0, 30.0, 30.0));
    }

    #[test]
    fn only_web_and_mail_addresses_are_opened() {
        let runtime = |url: &str| mirsam_core::links::may_open(url);
        assert!(runtime("https://example.com/a?b=c&d=e"));
        assert!(!runtime("file:///etc/passwd"));
        assert!(!runtime("https://example.com/\"&calc"));
    }
}
