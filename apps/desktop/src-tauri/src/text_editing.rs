//! Editing a page's text in place (ADR 0024): thin commands over
//! `safha_core::text_edit::BlockEditor`.
//!
//! The edit tool lists a page's blocks ([`text_blocks`]); a click on one
//! begins a session ([`begin_block_edit`]) that holds the block's editor. Each
//! keystroke asks for the text's layout ([`layout_block_edit`]): glyph
//! positions, caret stops, and the outlines of glyphs the UI has not drawn
//! yet, which the UI draws on its own surface (ADR 0022). Committing
//! ([`commit_block_edit`]) writes the text into the page as one undo step.
//!
//! Text offsets are UTF-16 code units, as the UI's text field counts them.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use safha_core::Document;
use safha_core::edit::Change;
use safha_core::text_edit::{BlockEditor, Refusal};
use safha_core::typeset::{
    FontFace, MatchKind, TextLayout, caret_stops, glyph_outline, subset_fonts, units_per_em,
};
use safha_core::view::ViewRect;

use crate::editing::EditOutcome;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, lock};

/// One block of a page's text, as the edit tool shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextBlockInfo {
    /// Its box on the displayed page, in points.
    rect: ViewRect,
    /// Why it cannot be edited in place; absent when it can.
    refusal: Option<Refusal>,
}

#[tauri::command]
pub async fn text_blocks(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
) -> CommandResult<Vec<TextBlockInfo>> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    Ok(document
        .text_blocks(page)?
        .into_iter()
        .map(|(rect, refusal)| TextBlockInfo { rect, refusal })
        .collect())
}

/// A block being edited: its editor, and the revision it was made for.
pub struct BlockEdit {
    pub(crate) document_id: u32,
    revision: Arc<Document>,
    editor: BlockEditor,
}

/// The block an edit began on, and how it is set.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockEditStart {
    token: u32,
    /// The block's text, soft line breaks made spaces.
    text: String,
    font: FontInfo,
    /// The box's left edge and width on the displayed page; the top comes
    /// with each layout, since the first baseline stays put.
    x: f32,
    width: f32,
    /// The original block's box (what the editor covers while open).
    original: ViewRect,
    size: f32,
    /// sRGB, 0–1.
    color: [f32; 3],
    /// Its runs differ in size or colour, which setting it as one loses.
    mixed: bool,
    rtl: bool,
    layout: EditLayout,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontInfo {
    /// The PDF's font, as it names it (empty for an unnamed one).
    original: String,
    /// The font the text is set in.
    used: String,
    /// "exact", "sameFamily", or "substitute".
    kind: &'static str,
}

/// The text laid out as it will be written.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditLayout {
    /// The box's top on the displayed page; its height.
    top: f32,
    height: f32,
    lines: Vec<EditLine>,
    glyphs: Vec<EditGlyph>,
    /// Each cluster's extent, for selections.
    clusters: Vec<EditCluster>,
    carets: Vec<EditCaret>,
    /// Outlines of glyphs not among those the request said it holds.
    outlines: Vec<EditOutline>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditLine {
    /// From the box's top.
    top: f32,
    baseline: f32,
    height: f32,
    start: usize,
    end: usize,
    rtl: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditGlyph {
    /// Index into the session's fonts.
    font: usize,
    id: u32,
    /// From the box's left edge; the baseline from its top, y down.
    x: f32,
    y: f32,
    size: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditCluster {
    line: usize,
    start: usize,
    end: usize,
    left: f32,
    right: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditCaret {
    offset: usize,
    line: usize,
    x: f32,
    split: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOutline {
    font: usize,
    id: u32,
    units_per_em: u16,
    /// SVG path data in font units, y up.
    path: String,
}

fn kind_name(kind: MatchKind) -> &'static str {
    match kind {
        MatchKind::Exact => "exact",
        MatchKind::SameFamily => "sameFamily",
        MatchKind::Substitute => "substitute",
    }
}

/// `text` laid out by `editor`, with outlines for glyphs not in `known`.
fn edit_layout(editor: &BlockEditor, text: &str, known: &[(usize, u32)]) -> CommandResult<EditLayout> {
    let (laid, top) = editor.layout(text)?;
    let faces: Vec<FontFace> = editor.faces().into_iter().map(|(face, _)| face).collect();
    describe(text, &laid, top, &faces, known)
}

fn describe(
    text: &str,
    laid: &TextLayout,
    top: f32,
    faces: &[FontFace],
    known: &[(usize, u32)],
) -> CommandResult<EditLayout> {
    let utf16 = |byte: usize| text.get(..byte).map_or(0, |prefix| prefix.encode_utf16().count());

    // Outlines come from the subsets the writer would embed: the same glyphs,
    // at the same variation.
    let missing =
        laid.runs().any(|run| run.glyphs.iter().any(|glyph| !known.contains(&(run.font, glyph.id))));
    let mut outlines = Vec::new();
    if missing {
        let subsets =
            subset_fonts(text, laid, faces).map_err(|err| CommandError::new("text_failed", err.code()))?;
        for subset in &subsets {
            let upem = units_per_em(&subset.data).unwrap_or(1000);
            for glyph in &subset.glyphs {
                if known.contains(&(subset.font, glyph.original)) {
                    continue;
                }
                if let Some(path) = glyph_outline(&subset.data, u32::from(glyph.cid)) {
                    outlines.push(EditOutline {
                        font: subset.font,
                        id: glyph.original,
                        units_per_em: upem,
                        path,
                    });
                }
            }
        }
    }

    let (mut lines, mut glyphs, mut clusters) = (Vec::new(), Vec::new(), Vec::new());
    for (index, line) in laid.lines.iter().enumerate() {
        lines.push(EditLine {
            top: line.top,
            baseline: line.baseline,
            height: line.height,
            start: utf16(line.text_range.start),
            end: utf16(line.text_range.end),
            rtl: line.rtl,
        });
        for run in &line.runs {
            for glyph in &run.glyphs {
                glyphs.push(EditGlyph {
                    font: run.font,
                    id: glyph.id,
                    x: glyph.x,
                    y: line.baseline - glyph.y_offset,
                    size: run.size,
                });
            }
            for cluster in &run.clusters {
                let drawn = &run.glyphs[cluster.glyphs.clone()];
                let Some(first) = drawn.first() else { continue };
                let width: f32 = drawn.iter().map(|glyph| glyph.advance).sum();
                clusters.push(EditCluster {
                    line: index,
                    start: utf16(cluster.text.start),
                    end: utf16(cluster.text.end),
                    left: first.x,
                    right: first.x + width,
                });
            }
        }
    }
    let carets = caret_stops(text, laid)
        .into_iter()
        .map(|stop| EditCaret { offset: utf16(stop.offset), line: stop.line, x: stop.x, split: stop.split })
        .collect();
    Ok(EditLayout { top, height: laid.height, lines, glyphs, clusters, carets, outlines })
}

/// Begins editing block `block` of page `page`, or says why it cannot be
/// edited in place (`edit_unsupported`).
#[tauri::command]
pub async fn begin_block_edit(
    state: State<'_, AppState>,
    document_id: u32,
    page: usize,
    block: usize,
) -> CommandResult<BlockEditStart> {
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    // Installed fonts stay out until their embedding permissions are read
    // (ADR 0024): only the bundled faces are written.
    let editor = document.block_editor(page, block, false)?;
    let original = document.text_blocks(page)?.get(block).map(|(rect, _)| *rect).unwrap_or(ViewRect {
        x: editor.x,
        y: editor.first_baseline,
        width: editor.width,
        height: editor.height,
    });
    let layout = edit_layout(&editor, &editor.text, &[])?;
    let start = BlockEditStart {
        token: 0,
        text: editor.text.clone(),
        font: FontInfo {
            original: editor.font.original.clone(),
            used: editor.font.used.clone(),
            kind: kind_name(editor.font.kind),
        },
        x: editor.x,
        width: editor.width,
        original,
        size: editor.size,
        color: editor.color,
        mixed: editor.mixed,
        rtl: editor.direction == safha_core::typeset::BaseDirection::Rtl,
        layout,
    };
    let token = state.hold_block_edit(BlockEdit { document_id, revision: document, editor });
    Ok(BlockEditStart { token, ..start })
}

/// `text` laid out in the block being edited. `known`: the glyphs (font,
/// id) whose outlines the UI already holds.
#[tauri::command]
pub async fn layout_block_edit(
    state: State<'_, AppState>,
    token: u32,
    text: String,
    known: Vec<(usize, u32)>,
) -> CommandResult<EditLayout> {
    state
        .with_block_edit(token, |edit| edit_layout(&edit.editor, &text, &known))
        .ok_or_else(|| CommandError::new("stale_edit", "no such text edit"))?
}

/// Writes `text` in place of the block: one undo step. Nothing changes (and
/// nothing is returned) when the text is what it was.
#[tauri::command]
pub async fn commit_block_edit(
    state: State<'_, AppState>,
    token: u32,
    text: String,
) -> CommandResult<Option<EditOutcome>> {
    let edit =
        state.take_block_edit(token).ok_or_else(|| CommandError::new("stale_edit", "no such text edit"))?;
    if text == edit.editor.text {
        return Ok(None);
    }
    let document_id = edit.document_id;
    let history = state.history(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    let mut history = lock(&history);
    let document = state.document(document_id).ok_or_else(|| CommandError::unknown_document(document_id))?;
    // An undo, a redo, or another edit since the session began: the block's
    // operations are not where they were.
    if !Arc::ptr_eq(&document, &edit.revision) {
        return Err(CommandError::new("stale_edit", "the document changed while its text was edited"));
    }
    let (revision, update) = document.with_replaced_block(&edit.editor, &text)?;
    history.push_content(update);
    let page_count = revision.page_count();
    state.replace_revision(document_id, revision);
    Ok(Some(EditOutcome::new(page_count, &history, Change::Content)))
}

/// Ends a session without writing anything.
#[tauri::command]
pub async fn cancel_block_edit(state: State<'_, AppState>, token: u32) -> CommandResult<()> {
    state.take_block_edit(token);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use safha_core::typeset::{Align, BaseDirection, LineHeight, ParagraphStyle, Span, layout};

    fn faces() -> Vec<FontFace> {
        safha_core::fonts::faces_for(safha_core::fonts::FontFamily::Sans, "مرحبا hello")
            .expect("bundled")
            .into_iter()
            .map(|(face, _)| face)
            .collect()
    }

    fn laid(text: &str, faces: &[FontFace]) -> TextLayout {
        let style = ParagraphStyle {
            direction: BaseDirection::Auto,
            align: Align::Start,
            width: Some(200.0),
            line_height: LineHeight::Normal,
        };
        layout(text, &[Span::new(0..text.len(), 0, 12.0)], faces, &style).expect("lays out")
    }

    #[test]
    fn offsets_are_utf16_and_known_outlines_are_not_sent_again() {
        let faces = faces();
        // "𝐀" is two UTF-16 units: the caret after it stands at 2, not 4 (bytes).
        let text = "مرحبا 𝐀 hello";
        let laid = laid(text, &faces);
        let first = describe(text, &laid, 10.0, &faces, &[]).expect("describes");
        assert_eq!(first.top, 10.0);
        let units = text.encode_utf16().count();
        assert!(first.carets.iter().all(|caret| caret.offset <= units));
        assert!(first.carets.iter().any(|caret| caret.offset == units));
        assert_eq!(first.lines.last().map(|line| line.end), Some(units));
        assert!(!first.outlines.is_empty());
        // Every cluster covers text and has width.
        assert!(first.clusters.iter().all(|c| c.end > c.start && c.right > c.left));

        let known: Vec<(usize, u32)> = first.outlines.iter().map(|o| (o.font, o.id)).collect();
        let again = describe(text, &laid, 10.0, &faces, &known).expect("describes");
        assert!(again.outlines.is_empty(), "outlines already held are not sent");
        assert_eq!(again.glyphs.len(), first.glyphs.len());
    }
}
