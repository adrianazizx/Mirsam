//! Text boxes (`/FreeText` annotations, ADR 0019): text drawn on the page,
//! shaped by the text engine (`typeset`, ADR 0020) and written with embedded
//! font subsets (`pdf::text`), so Arabic is joined and ordered correctly in
//! every viewer (spec §6.4: text boxes go through the same shaping pipeline).
//!
//! The box's width is chosen; its height always fits the text. Its text is
//! drawn upright as the page is displayed: on a turned page the appearance
//! is turned back by the page's `/Rotate`, as a note's icon is.

use super::{Appearance, check_color, check_contents, in_range, num, text_string};
use crate::error::{Error, Result};
use crate::pdf::text::{NamedFont, draw_text};
use crate::pdf::{Dict, Object};
use crate::typeset::{Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, layout};
use crate::view::ViewTransform;

/// Space between the box's edge and its text, in points.
pub const TEXT_BOX_PADDING: f32 = 2.0;

/// A text box to add.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    /// The box in page space (left, bottom, right, top).
    pub rect: [f32; 4],
    pub contents: String,
    /// Font size in points.
    pub size: f32,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
}

/// Whether the text's first strong character is right to left (UAX #9 P2).
fn starts_rtl(text: &str) -> bool {
    crate::fonts::starts_rtl(text)
}

/// The fonts for a text: IBM Plex, its own script's face first (ADR 0019).
fn fonts_for(text: &str) -> Result<Vec<NamedFont>> {
    crate::fonts::for_text(crate::fonts::FontFamily::Sans, text)
}

fn check(contents: &str, size: f32, width: f32) -> Result<()> {
    check_contents(contents)?;
    if contents.trim().is_empty() {
        return Err(Error::InvalidEdit("a text box needs text"));
    }
    if !(1.0..=400.0).contains(&size) {
        return Err(Error::InvalidEdit("text is 1 to 400 points in size"));
    }
    if !(width > 2.0 * TEXT_BOX_PADDING + 1.0 && width <= 14_400.0) {
        return Err(Error::InvalidEdit("a text box needs room for its text"));
    }
    Ok(())
}

impl TextBox {
    pub const DEFAULT_SIZE: f32 = 12.0;
    /// A new box's width, in points, when it fits on the page.
    pub const DEFAULT_WIDTH: f32 = 240.0;
    pub const DEFAULT_COLOR: [f32; 3] = [0.0, 0.0, 0.0];

    /// The height a box `width` points wide needs for `contents`, padding included.
    pub fn height_for(contents: &str, width: f32, size: f32) -> Result<f32> {
        check(contents, size, width)?;
        let fonts = fonts_for(contents)?;
        let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
        let laid = layout(contents, &[Span::new(0..contents.len(), 0, size)], &faces, &style(width))
            .map_err(|err| Error::Typeset(err.code()))?;
        Ok(laid.height + 2.0 * TEXT_BOX_PADDING)
    }

    /// The page-space box displayed with its top-left corner at (`x`, `y`),
    /// `width` × `height` points.
    pub fn rect_at(x: f32, y: f32, width: f32, height: f32, view: &ViewTransform) -> [f32; 4] {
        let corners = [(x, y), (x + width, y), (x, y + height), (x + width, y + height)];
        super::bounds(corners.into_iter().map(|(x, y)| view.page_point(x, y)))
    }

    /// `/Contents`; `/DA` for viewers that redraw it themselves; `/Q` (the
    /// text's alignment: right for right-to-left text); no border.
    pub(crate) fn entries(&self) -> Vec<(Vec<u8>, Object)> {
        let [r, g, b] = self.color;
        let da = format!("/Helv {} Tf {} {} {} rg", num(self.size), num(r), num(g), num(b));
        let border = Dict(vec![(b"W".to_vec(), Object::Int(0))]);
        vec![
            (b"Contents".to_vec(), Object::Str(text_string(&self.contents))),
            (b"DA".to_vec(), Object::Str(da.into_bytes())),
            (b"Q".to_vec(), Object::Int(if starts_rtl(&self.contents) { 2 } else { 0 })),
            (b"BS".to_vec(), Object::Dict(border)),
        ]
    }

    /// The box's size as displayed on a page turned by `rotate` degrees.
    fn displayed_size(&self, rotate: i64) -> (f32, f32) {
        let [left, bottom, right, top] = self.rect;
        let (w, h) = (right - left, top - bottom);
        if rotate.rem_euclid(180) == 90 { (h, w) } else { (w, h) }
    }

    /// The text, laid out to the box's displayed width and drawn from its
    /// top-left corner, upright as displayed.
    pub(crate) fn appearance(&self, rotate: i64) -> Result<Appearance> {
        check_color(self.color)?;
        let [left, bottom, right, top] = self.rect;
        if !in_range((left, bottom)) || !in_range((right, top)) || right <= left || top <= bottom {
            return Err(Error::InvalidEdit("a text box needs a place on the page"));
        }
        let (width, height) = self.displayed_size(rotate);
        check(&self.contents, self.size, width)?;
        let fonts = fonts_for(&self.contents)?;
        let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
        let laid =
            layout(&self.contents, &[Span::new(0..self.contents.len(), 0, self.size)], &faces, &style(width))
                .map_err(|err| Error::Typeset(err.code()))?;
        let drawing = draw_text(
            &self.contents,
            &laid,
            &fonts,
            (TEXT_BOX_PADDING, height - TEXT_BOX_PADDING),
            self.color,
            "F",
        )?;

        let turn = (rotate.rem_euclid(360) as f32).to_radians();
        let (sin, cos) = (turn.sin().round() + 0.0, turn.cos().round() + 0.0);
        Ok(Appearance {
            rect: self.rect,
            bbox: [0.0, 0.0, width, height],
            matrix: (rotate.rem_euclid(360) != 0).then_some([cos, sin, -sin + 0.0, cos, 0.0, 0.0]),
            resources: Dict::default(),
            content: drawing.content,
            fonts: drawing.fonts,
            images: Vec::new(),
        })
    }
}

/// Wrapped to the box's width inside its padding, aligned to the start of
/// the text's own direction.
fn style(width: f32) -> ParagraphStyle {
    ParagraphStyle {
        direction: BaseDirection::Auto,
        align: Align::Start,
        width: Some(width - 2.0 * TEXT_BOX_PADDING),
        line_height: LineHeight::Normal,
    }
}

/// `rect` resized to `height` as displayed on a page turned by `rotate`
/// degrees, its displayed top edge kept where it is.
pub(crate) fn resized(rect: [f32; 4], rotate: i64, height: f32) -> [f32; 4] {
    let [left, bottom, right, top] = rect;
    match rotate.rem_euclid(360) {
        // The displayed top is the page's left edge; down is +x.
        90 => [left, bottom, left + height, top],
        // The displayed top is the page's bottom edge; down is +y.
        180 => [left, bottom, right, bottom + height],
        // The displayed top is the page's right edge; down is -x.
        270 => [right - height, bottom, right, top],
        _ => [left, top - height, right, top],
    }
}

/// Font size and colour from a `/DA` string (`/Helv 12 Tf 0 0 0 rg`), with
/// this module's defaults for what it lacks.
pub(crate) fn parse_da(da: &[u8]) -> (f32, [f32; 3]) {
    let text = String::from_utf8_lossy(da);
    let words: Vec<&str> = text.split_whitespace().collect();
    let number = |i: usize| words.get(i).and_then(|w| w.parse::<f32>().ok());
    let mut size = TextBox::DEFAULT_SIZE;
    let mut color = TextBox::DEFAULT_COLOR;
    for (i, word) in words.iter().enumerate() {
        match *word {
            "Tf" => size = i.checked_sub(1).and_then(number).filter(|s| *s > 0.0).unwrap_or(size),
            "rg" => {
                if let (Some(r), Some(g), Some(b)) = (
                    i.checked_sub(3).and_then(number),
                    i.checked_sub(2).and_then(number),
                    i.checked_sub(1).and_then(number),
                ) {
                    color = [r, g, b];
                }
            }
            "g" => {
                if let Some(gray) = i.checked_sub(1).and_then(number) {
                    color = [gray; 3];
                }
            }
            _ => {}
        }
    }
    (size, color.map(|c| c.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_box(contents: &str, rect: [f32; 4]) -> TextBox {
        TextBox { rect, contents: contents.into(), size: 12.0, color: TextBox::DEFAULT_COLOR }
    }

    #[test]
    fn longer_text_needs_a_taller_box() {
        let one = TextBox::height_for("Approved", 200.0, 12.0).expect("lays out");
        let many = TextBox::height_for(&"Approved with changes. ".repeat(10), 200.0, 12.0).expect("lays out");
        assert!(one > 12.0 && one < 24.0, "{one}");
        assert!(many > 4.0 * one, "{many} vs {one}");
        let arabic = TextBox::height_for("تمت الموافقة مع ملاحظات", 200.0, 12.0).expect("lays out");
        assert!(arabic > 12.0 && arabic < 30.0, "{arabic}");
    }

    #[test]
    fn arabic_text_is_set_in_the_arabic_family_and_aligned_right() {
        let fonts = fonts_for("موافق — OK").expect("fonts");
        assert_eq!(fonts[0].name, "IBMPlexSansArabic");
        assert_eq!(fonts_for("OK — موافق").expect("fonts")[0].name, "IBMPlexSans");
        let entries = text_box("موافق", [0.0, 0.0, 100.0, 20.0]).entries();
        assert!(entries.contains(&(b"Q".to_vec(), Object::Int(2))));
        assert!(text_box("OK", [0.0, 0.0, 100.0, 20.0]).entries().contains(&(b"Q".to_vec(), Object::Int(0))));
    }

    #[test]
    fn the_appearance_embeds_the_fonts_it_draws_with() {
        let appearance = text_box("Mixed: موافق", [10.0, 10.0, 210.0, 40.0]).appearance(0).expect("valid");
        assert_eq!(appearance.fonts.len(), 2);
        assert_eq!(appearance.bbox, [0.0, 0.0, 200.0, 30.0]);
        let content = String::from_utf8(appearance.content).expect("ascii");
        assert!(content.contains("/F0 12 Tf") && content.contains("/F1 12 Tf"), "{content}");
        // On a page turned a quarter, the form is turned back and its box is the displayed one.
        let turned = text_box("OK", [10.0, 10.0, 40.0, 210.0]).appearance(90).expect("valid");
        assert_eq!(turned.bbox, [0.0, 0.0, 200.0, 30.0]);
        assert_eq!(turned.matrix, Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]));
    }

    #[test]
    fn resizing_keeps_the_displayed_top_edge() {
        let rect = [100.0, 500.0, 300.0, 540.0];
        assert_eq!(resized(rect, 0, 60.0), [100.0, 480.0, 300.0, 540.0]);
        assert_eq!(resized(rect, 90, 60.0), [100.0, 500.0, 160.0, 540.0]);
        assert_eq!(resized(rect, 180, 60.0), [100.0, 500.0, 300.0, 560.0]);
        assert_eq!(resized(rect, 270, 60.0), [240.0, 500.0, 300.0, 540.0]);
    }

    #[test]
    fn the_default_appearance_string_is_read_back() {
        assert_eq!(parse_da(b"/Helv 14 Tf 0.5 0 1 rg"), (14.0, [0.5, 0.0, 1.0]));
        assert_eq!(parse_da(b"0.2 g /Helv 9 Tf"), (9.0, [0.2; 3]));
        assert_eq!(parse_da(b"garbage"), (12.0, [0.0; 3]));
    }

    #[test]
    fn empty_or_cramped_boxes_are_refused() {
        assert!(matches!(TextBox::height_for("  ", 200.0, 12.0), Err(Error::InvalidEdit(_))));
        assert!(matches!(TextBox::height_for("x", 3.0, 12.0), Err(Error::InvalidEdit(_))));
        assert!(matches!(TextBox::height_for("x", 200.0, 0.0), Err(Error::InvalidEdit(_))));
    }
}
