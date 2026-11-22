//! The only part of text extraction that talks to PDFium.

use pdfium_render::prelude::{PdfPage, PdfPageObjectCommon, PdfPageObjectsCommon, PdfPageText};

use super::layout::{Glyph, SpaceMark};

/// Every character on the page, in content-stream order.
///
/// PDFium's *generated* characters (the spaces and line breaks it inserts from
/// its own layout guess) are dropped. Space characters the PDF really contains
/// are kept as `' '` glyphs: PDFium reports their *position* unreliably, but
/// their place in the content stream says which two glyphs they separate.
pub(crate) fn collect_glyphs(text: &PdfPageText<'_>) -> Vec<Glyph> {
    let mut glyphs = Vec::new();
    for (index, ch) in text.chars().iter().enumerate() {
        if ch.is_generated().unwrap_or(false) {
            continue;
        }
        let Some(c) = ch.unicode_char() else { continue };
        let c = if c.is_whitespace() { ' ' } else { c };
        if c.is_control() {
            continue;
        }
        let Ok(bounds) = ch.loose_bounds() else { continue };
        let Ok((origin_x, origin_y)) = ch.origin() else { continue };
        let size = ch.scaled_font_size().value;
        glyphs.push(Glyph {
            index,
            ch: c,
            left: bounds.left().value,
            right: bounds.right().value,
            bottom: bounds.bottom().value,
            top: bounds.top().value,
            origin_x: origin_x.value,
            origin_y: origin_y.value,
            // Type 3 fonts and some producers report 0 or 1; fall back to the box height.
            size: if size > 1.5 { size } else { (bounds.top().value - bounds.bottom().value).max(1.0) },
        });
    }
    glyphs
}

/// Where space glyphs are actually drawn.
///
/// A text object that draws nothing visible and maps to no text, or only to
/// whitespace, is a space glyph. Chrome writes one at every word break of
/// Arabic text, exactly where the break is, while PDFium's character list
/// drops most of them; these markers are the most reliable word-break signal
/// there is. Objects nested in form XObjects are not visited yet.
///
/// Takes the page's already-built text page: building one is a full text
/// analysis of the page, which `PdfPageTextObject::text` would redo per object.
pub(crate) fn collect_space_marks(page: &PdfPage<'_>, text: &PdfPageText<'_>) -> Vec<SpaceMark> {
    let mut marks = Vec::new();
    for object in page.objects().iter() {
        let Some(text_object) = object.as_text_object() else { continue };
        let Ok(bounds) = object.bounds() else { continue };
        let size = text_object.scaled_font_size().value.max(1.0);
        let (left, right) = (bounds.left().value, bounds.right().value);
        if right - left > 0.02 * size {
            continue;
        }
        if !text.for_object(text_object).chars().all(char::is_whitespace) {
            continue;
        }
        marks.push(SpaceMark { x: left, y: bounds.bottom().value, size });
    }
    marks
}
