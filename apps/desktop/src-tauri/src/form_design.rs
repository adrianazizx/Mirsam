//! Making forms (ADR 0047): thin commands over `mirsam_core::forms`. Boxes
//! come from the UI in displayed-page points and are mapped to page space
//! here; every change the UI sends at once is one undo step.

use serde::{Deserialize, Serialize};
use tauri::State;

use mirsam_core::edit::Change;
use mirsam_core::forms::{DetectedField, FieldDesign, FieldSpec};
use mirsam_core::view::ViewRect;

use crate::editing::EditOutcome;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, lock};

/// One change to the form, with boxes on the displayed page.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DesignEdit {
    Add { page: usize, rect: ViewRect, spec: FieldSpec },
    AddButton { field: u32, page: usize, rect: ViewRect, value: String },
    Update { field: u32, spec: FieldSpec },
    SetRect { field: u32, widget: usize, page: usize, rect: ViewRect },
    RemoveButton { field: u32, widget: usize },
    Remove { field: u32 },
    TabOrder { page: usize, fields: Vec<u32> },
}

/// What a form change did: the edit, and the field to select after it (the
/// last field added or changed), by its id in the new revision.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignOutcome {
    edit: EditOutcome,
    field: Option<u32>,
}

/// A box on displayed page `page`, in page space.
fn page_rect(document: &mirsam_core::Document, page: usize, rect: ViewRect) -> mirsam_core::Result<[f32; 4]> {
    let view = document.view_transform(page)?;
    let corners = [(rect.x, rect.y), (rect.x + rect.width, rect.y + rect.height)];
    let [(x0, y0), (x1, y1)] = corners.map(|(x, y)| view.page_point(x, y));
    Ok([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

/// Changes the form: `edits` in order, one undo step.
#[tauri::command]
pub async fn design_form(
    state: State<'_, AppState>,
    document_id: u32,
    edits: Vec<DesignEdit>,
) -> CommandResult<DesignOutcome> {
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut designs = Vec::with_capacity(edits.len());
    // The field to select afterwards: by id, or (a new one) by name.
    let mut select: Option<Result<u32, String>> = None;
    for edit in edits {
        designs.push(match edit {
            DesignEdit::Add { page, rect, spec } => {
                select = Some(Err(spec.name.trim().to_string()));
                FieldDesign::Add { page, rect: page_rect(&document, page, rect)?, spec }
            }
            DesignEdit::AddButton { field, page, rect, value } => {
                select = Some(Ok(field));
                FieldDesign::AddButton { field, page, rect: page_rect(&document, page, rect)?, value }
            }
            DesignEdit::Update { field, spec } => {
                select = Some(Ok(field));
                FieldDesign::Update { field, spec }
            }
            DesignEdit::SetRect { field, widget, page, rect } => {
                select = Some(Ok(field));
                FieldDesign::SetRect { field, widget, rect: page_rect(&document, page, rect)? }
            }
            DesignEdit::RemoveButton { field, widget } => {
                select = Some(Ok(field));
                FieldDesign::RemoveButton { field, widget }
            }
            DesignEdit::Remove { field } => {
                select = None;
                FieldDesign::Remove { field }
            }
            DesignEdit::TabOrder { page, fields } => FieldDesign::TabOrder { page, fields },
        });
    }
    let (revision, update) = document.with_form_design(&designs)?;
    let field = match select {
        Some(Ok(id)) => Some(id),
        Some(Err(name)) => revision.form()?.fields.into_iter().find(|f| f.name == name).map(|f| f.id),
        None => None,
    };
    history.push_annotations(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(DesignOutcome { edit: EditOutcome::new(page_count, &history, Change::Annotations), field })
}

/// Fields a flat form seems to ask for on `page`, for review.
#[tauri::command]
pub async fn detect_form_fields(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
) -> CommandResult<Vec<DetectedField>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    tauri::async_runtime::spawn_blocking(move || Ok(document.detect_fields(page)?))
        .await
        .map_err(|err| CommandError::new("engine_error", err.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_arrive_as_the_ui_sends_them() {
        let json = r#"[
            {"op": "add", "page": 0, "rect": {"x": 10, "y": 20, "width": 100, "height": 24},
             "spec": {"kind": "text", "name": "الاسم", "fontSize": 0, "defaultValues": ["محمد"]}},
            {"op": "setRect", "field": 7, "widget": 0, "page": 1, "rect": {"x": 1, "y": 2, "width": 3, "height": 4}},
            {"op": "tabOrder", "page": 0, "fields": [3, 2]},
            {"op": "addButton", "field": 7, "page": 0, "rect": {"x": 1, "y": 2, "width": 12, "height": 12}, "value": "لا"}
        ]"#;
        let edits: Vec<DesignEdit> = serde_json::from_str(json).expect("parses");
        match &edits[0] {
            DesignEdit::Add { spec, .. } => {
                assert_eq!(spec.name, "الاسم");
                assert!(spec.bordered, "bordered unless said otherwise");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(edits[1], DesignEdit::SetRect { field: 7, widget: 0, page: 1, .. }));
        assert!(matches!(&edits[2], DesignEdit::TabOrder { fields, .. } if fields == &[3, 2]));
        assert!(matches!(&edits[3], DesignEdit::AddButton { value, .. } if value == "لا"));
    }
}
