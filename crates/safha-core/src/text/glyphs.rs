//! The only part of text extraction that talks to PDFium.

use pdfium_render::prelude::{
    PdfMatrix, PdfPage, PdfPageObject, PdfPageObjectCommon, PdfPageObjectsCommon, PdfPageText, PdfPoints,
};

use super::layout::{Glyph, SpaceMark, turn_point, unturn_point};

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
        let turn = ch.angle_radians().map_or(0, quarter_turns);
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
            turn,
        });
    }
    glyphs
}

/// The quarter turn (0–3, counter-clockwise) a baseline is set along, from
/// PDFium's angle for it: `FPDFText_GetCharAngle` is `atan2(c, a)` of the
/// character's matrix, which turns clockwise. 0 for a baseline that is not
/// near a quarter turn (slanted text is read as it always was).
fn quarter_turns(angle: f32) -> u8 {
    quarter_of(-angle)
}

/// The quarter turn (0–3) nearest to a counter-clockwise `angle` in
/// radians, or 0 when it is not near one.
fn quarter_of(angle: f32) -> u8 {
    let quarters = angle / std::f32::consts::FRAC_PI_2;
    let nearest = quarters.round();
    if (quarters - nearest).abs() > 0.2 {
        return 0;
    }
    (nearest as i64).rem_euclid(4) as u8
}

/// Where space glyphs are actually drawn.
///
/// A text object that draws nothing visible and maps to no text, or only to
/// whitespace, is a space glyph. Chrome writes one at every word break of
/// Arabic text, exactly where the break is, while PDFium's character list
/// drops most of them; these markers are the most reliable word-break signal
/// there is. Objects nested in form XObjects are visited too (content some
/// producers wrap in forms, flattened annotations), their bounds mapped
/// through each form's matrix to page space.
///
/// Takes the page's already-built text page: building one is a full text
/// analysis of the page, which `PdfPageTextObject::text` would redo per object.
pub(crate) fn collect_space_marks(page: &PdfPage<'_>, text: &PdfPageText<'_>) -> Vec<SpaceMark> {
    let mut marks = Vec::new();
    for object in page.objects().iter() {
        visit(&object, &mut Vec::new(), text, &mut marks);
    }
    marks
}

/// Forms nested deeper than this are not looked into.
const MAX_FORM_DEPTH: usize = 8;

/// Collects the space marks of `object`, which lies inside the forms whose
/// matrices are `forms` (outermost first).
fn visit(
    object: &PdfPageObject<'_>,
    forms: &mut Vec<PdfMatrix>,
    text: &PdfPageText<'_>,
    marks: &mut Vec<SpaceMark>,
) {
    if let Some(form) = object.as_x_object_form_object() {
        let Ok(matrix) = object.matrix() else { return };
        if forms.len() >= MAX_FORM_DEPTH {
            return;
        }
        forms.push(matrix);
        for child in form.iter() {
            visit(&child, forms, text, marks);
        }
        forms.pop();
        return;
    }
    let Some(text_object) = object.as_text_object() else { return };
    let Ok(bounds) = object.bounds() else { return };
    // To page space: through the innermost form first.
    let to_page = |x: f32, y: f32| {
        forms.iter().rev().fold((x, y), |(x, y), matrix| {
            let (x, y) = matrix.apply_to_points(PdfPoints::new(x), PdfPoints::new(y));
            (x.value, y.value)
        })
    };
    // The text's own frame: where its baseline runs left to right.
    let turn = object.matrix().map_or(0, |m| quarter_of(m.b().atan2(m.a())));
    let (x0, y0) = turn_point(to_page(bounds.left().value, bounds.bottom().value), turn);
    let (x1, y1) = turn_point(to_page(bounds.right().value, bounds.top().value), turn);
    // A form's scale enlarges the font as it does the box.
    let scale = forms.iter().fold(1.0, |scale, m| scale * (m.a() * m.d() - m.b() * m.c()).abs().sqrt());
    let size = (text_object.scaled_font_size().value * scale).max(1.0);
    let (left, right) = (x0.min(x1), x0.max(x1));
    if right - left > 0.02 * size {
        return;
    }
    if !text.for_object(text_object).chars().all(char::is_whitespace) {
        return;
    }
    let (x, y) = unturn_point((left, y0.min(y1)), turn);
    marks.push(SpaceMark { x, y, size, turn });
}

#[cfg(test)]
mod tests {
    use super::quarter_turns;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn baselines_snap_to_the_nearest_quarter_turn() {
        // PDFium's angles turn clockwise: a quarter clockwise is three counter-clockwise.
        assert_eq!(quarter_turns(0.0), 0);
        assert_eq!(quarter_turns(FRAC_PI_2 + 0.01), 3);
        assert_eq!(quarter_turns(PI), 2);
        assert_eq!(quarter_turns(3.0 * FRAC_PI_2 - 0.02), 1);
        assert_eq!(quarter_turns(2.0 * PI - 0.01), 0);
        // A watermark at 45° is read as before.
        assert_eq!(quarter_turns(PI / 4.0), 0);
    }
}
