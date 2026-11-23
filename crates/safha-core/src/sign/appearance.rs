//! What a visible certificate signature shows (ADR 0048): one of the
//! user's kept signatures (drawn, typed or an image, as Fill & Sign places
//! them, ADR 0045) above a caption, both inside the box the user chose. The
//! caption (the signer's name and the date, worded and with digits as the UI
//! shows them) is shaped by the text engine, so an Arabic name joins and
//! reads right to left, and is embedded as font subsets like any text we
//! write. Without a kept signature the caption fills the box.
//!
//! The box stands upright as the page is displayed, as Fill & Sign items do.

use crate::annotate::{Appearance, Signature, num, upright};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, for_text};
use crate::pdf::Dict;
use crate::pdf::text::draw_text;
use crate::typeset::{Align, FontFace, ParagraphStyle, Span, layout};
use crate::view::ViewTransform;

/// The caption's ink: near black, so it reads on any paper.
const CAPTION_COLOR: [f32; 3] = [0.12, 0.12, 0.12];
/// The longest caption, in characters.
pub const MAX_CAPTION: usize = 400;

/// A signature shown on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleSignature {
    pub page: usize,
    /// The box in page space (left, bottom, right, top).
    pub rect: [f32; 4],
    /// A kept signature to draw, if any.
    pub graphic: Option<Signature>,
    /// Lines under it (or alone), separated by `\n`.
    pub caption: String,
    /// The ink of a drawn or typed graphic.
    pub color: [f32; 3],
}

impl VisibleSignature {
    /// A box `width` × `height` points as displayed, its top-left corner
    /// displayed at (`x`, `y`) on `page`.
    #[allow(clippy::too_many_arguments)]
    pub fn at(
        page: usize,
        (x, y): (f32, f32),
        (width, height): (f32, f32),
        view: &ViewTransform,
        graphic: Option<Signature>,
        caption: String,
        color: [f32; 3],
    ) -> Result<Self> {
        if !(8.0..=2000.0).contains(&width) || !(8.0..=2000.0).contains(&height) {
            return Err(Error::InvalidEdit("a signature box is 8 to 2,000 points on each side"));
        }
        if caption.chars().count() > MAX_CAPTION {
            return Err(Error::InvalidEdit("a signature caption is at most 400 characters"));
        }
        if let Some(graphic) = &graphic {
            graphic.check()?;
        }
        if graphic.is_none() && caption.trim().is_empty() {
            return Err(Error::InvalidEdit("a visible signature shows a signature or a caption"));
        }
        let rect = crate::annotate::TextBox::rect_at(x, y, width, height, view);
        Ok(Self { page, rect, graphic, caption, color })
    }

    /// The widget's appearance on a page turned by `rotate`.
    pub(crate) fn appearance(&self, rotate: i64) -> Result<Appearance> {
        let [left, bottom, right, top] = self.rect;
        let (page_width, page_height) = (right - left, top - bottom);
        // Its size as displayed.
        let (width, height) =
            if rotate.rem_euclid(180) == 90 { (page_height, page_width) } else { (page_width, page_height) };
        let pad = (width.min(height) * 0.05).clamp(1.0, 4.0);
        let caption = self.caption.trim();
        let caption_share = match (&self.graphic, caption.is_empty()) {
            (Some(_), false) => 0.34,
            (None, _) => 1.0,
            (Some(_), true) => 0.0,
        };
        let caption_top = pad + (height - 2.0 * pad) * caption_share;

        let mut content = Vec::new();
        let mut fonts = Vec::new();
        let mut images = Vec::new();
        if let Some(graphic) = &self.graphic {
            let (natural_width, natural_height) = graphic.natural_size()?;
            let drawn = graphic.appearance([0.0, 0.0, natural_width, natural_height], 0, self.color)?;
            let area = (width - 2.0 * pad, height - pad - caption_top);
            if area.0 > 0.5 && area.1 > 0.5 {
                let scale = (area.0 / natural_width).min(area.1 / natural_height);
                let x = pad + (area.0 - natural_width * scale) / 2.0;
                let y = caption_top + (area.1 - natural_height * scale) / 2.0;
                content.extend_from_slice(
                    format!(
                        "q {s} 0 0 {s} {} {} cm 0 0 {} {} re W n\n",
                        num(x),
                        num(y),
                        num(natural_width),
                        num(natural_height),
                        s = num(scale),
                    )
                    .as_bytes(),
                );
                content.extend_from_slice(&drawn.content);
                content.extend_from_slice(b"\nQ\n");
                fonts.extend(drawn.fonts);
                images.extend(drawn.images);
            }
        }
        if !caption.is_empty() {
            let area = (width - 2.0 * pad, caption_top - pad);
            if area.0 > 0.5 && area.1 > 0.5 {
                let named = for_text(FontFamily::Sans, caption)?;
                let faces: Vec<FontFace> = named.iter().map(|font| font.face.clone()).collect();
                let lines = caption.lines().count().max(1) as f32;
                let style = ParagraphStyle { align: Align::Center, ..ParagraphStyle::default() };
                let lay = |size: f32| {
                    layout(caption, &[Span::new(0..caption.len(), 0, size)], &faces, &style)
                        .map_err(|err| Error::Typeset(err.code()))
                };
                let mut size = (area.1 / (lines * 1.25)).clamp(2.0, 11.0);
                let mut laid = lay(size)?;
                if laid.width > area.0 || laid.height > area.1 {
                    size *= (area.0 / laid.width).min(area.1 / laid.height).min(1.0);
                    laid = lay(size.max(1.0))?;
                }
                let x = pad + (area.0 - laid.width) / 2.0;
                let y_top = caption_top - (area.1 - laid.height) / 2.0;
                let drawing = draw_text(caption, &laid, &named, (x, y_top), CAPTION_COLOR, "C")?;
                content.extend_from_slice(&drawing.content);
                fonts.extend(drawing.fonts);
            }
        }
        Ok(Appearance {
            rect: self.rect,
            bbox: [0.0, 0.0, width, height],
            matrix: upright(rotate),
            resources: Dict::default(),
            content,
            fonts,
            images,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{DrawnSignature, SignaturePoint};

    fn drawn() -> Signature {
        let point = |x: f32, y: f32| SignaturePoint { x, y, pressure: 0.5 };
        Signature::Drawn(DrawnSignature {
            strokes: vec![vec![point(0.0, 0.0), point(40.0, 10.0)]],
            width: 2.0,
        })
    }

    #[test]
    fn the_graphic_sits_above_an_arabic_caption() {
        let view = ViewTransform::unrotated(800.0);
        let visible = VisibleSignature::at(
            0,
            (100.0, 100.0),
            (180.0, 70.0),
            &view,
            Some(drawn()),
            "وقّعه سارة\n٢٣‏/١١‏/٢٠٢٦".into(),
            Signature::DEFAULT_COLOR,
        )
        .expect("fits");
        assert_eq!(visible.rect, [100.0, 630.0, 280.0, 700.0]);
        let appearance = visible.appearance(0).expect("draws");
        assert_eq!(appearance.bbox, [0.0, 0.0, 180.0, 70.0]);
        assert!(appearance.fonts.iter().any(|font| font.resource.starts_with('C')));
        assert!(appearance.fonts.iter().any(|font| font.base_font.contains("Arabic")));
        let content = String::from_utf8_lossy(&appearance.content);
        assert!(content.contains(" cm 0 0 ") && content.contains("BT"), "{content}");
        // Upright on a turned page: the box's displayed size, turned back.
        let turned = visible.appearance(90).expect("draws");
        assert_eq!(turned.bbox, [0.0, 0.0, 70.0, 180.0]);
        assert!(turned.matrix.is_some());
    }

    #[test]
    fn a_box_needs_something_to_show() {
        let view = ViewTransform::unrotated(800.0);
        let empty = VisibleSignature::at(0, (0.0, 0.0), (100.0, 40.0), &view, None, " ".into(), [0.0; 3]);
        assert!(matches!(empty, Err(Error::InvalidEdit(_))));
        let tiny = VisibleSignature::at(0, (0.0, 0.0), (2.0, 40.0), &view, None, "x".into(), [0.0; 3]);
        assert!(matches!(tiny, Err(Error::InvalidEdit(_))));
    }
}
