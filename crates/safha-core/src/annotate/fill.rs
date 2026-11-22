//! Fill & Sign (ADR 0045): text, dates, ticks, and signatures placed anywhere
//! on a page, for forms that have no fields.
//!
//! Each is a `/Stamp` annotation whose `/Name` says which of ours it is
//! (`MirsamText`, `MirsamCheck`, `MirsamCross`, `MirsamDot`,
//! `MirsamSignature`), so it is not listed as a comment, and with its own
//! appearance stream: text shaped by the text engine and written with
//! embedded font subsets, marks and drawn signatures as vector paths, image
//! signatures as image XObjects. The appearance's `/BBox` is the item's
//! natural size; viewers fit it to the `/Rect` (ISO 32000-2, 12.5.5), so
//! moving or resizing an item changes only its `/Rect`.
//!
//! Like a text box, each stands upright as the page is displayed: on a
//! turned page its appearance is turned back by the page's `/Rotate`.

use serde::{Deserialize, Serialize};

use super::{AnnotationKind, Appearance, Content, check_color, check_contents, in_range, num, text_string};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, for_text};
use crate::pdf::image::{ImageLimits, image_size, image_to_embed};
use crate::pdf::text::draw_text;
use crate::pdf::{Dict, Object};
use crate::typeset::{Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, TextLayout, layout};
use crate::view::ViewTransform;

const TEXT: &[u8] = b"MirsamText";
const CHECK: &[u8] = b"MirsamCheck";
const CROSS: &[u8] = b"MirsamCross";
const DOT: &[u8] = b"MirsamDot";
const SIGNATURE: &[u8] = b"MirsamSignature";

/// Space around fill text, in points.
const PADDING: f32 = 1.0;
/// The largest image signature file, in bytes.
pub const MAX_SIGNATURE_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// How large an image signature may be: 4,096 pixels on each side (ADR 0045).
const SIGNATURE_IMAGE: ImageLimits = ImageLimits {
    max_side: 4096,
    max_pixels: 4096 * 4096,
    too_large: "an image is 1 to 4,096 pixels on each side",
    unreadable: "only PNG and JPEG images can be used",
};
/// Limits for drawn signatures, as for ink.
const MAX_STROKES: usize = 1_000;
const MAX_POINTS: usize = 100_000;
/// The longest typed signature, in characters.
const MAX_TYPED: usize = 200;
/// The size typed signatures are set at; placing one scales it.
const TYPED_SIZE: f32 = 36.0;

/// Which of ours a `/Stamp` is, by its `/Name`.
pub(crate) fn kind_of_stamp(name: &[u8]) -> Option<AnnotationKind> {
    match name {
        TEXT => Some(AnnotationKind::FillText),
        CHECK | CROSS | DOT => Some(AnnotationKind::Mark),
        SIGNATURE => Some(AnnotationKind::Signature),
        _ => None,
    }
}

/// Which mark a `/Stamp` of ours draws, by its `/Name`.
pub(crate) fn mark_kind_of(name: &[u8]) -> Option<MarkKind> {
    match name {
        CHECK => Some(MarkKind::Check),
        CROSS => Some(MarkKind::Cross),
        DOT => Some(MarkKind::Dot),
        _ => None,
    }
}

/// A `/DA` for fill text: its size and colour, read back when it changes.
pub(crate) fn da(size: f32, [r, g, b]: [f32; 3]) -> String {
    format!("/Helv {} Tf {} {} {} rg", num(size), num(r), num(g), num(b))
}

/// The `/Matrix` that turns an appearance back by the page's clockwise
/// `rotate`, so it stands upright as displayed; none on an unturned page.
pub(crate) fn upright(rotate: i64) -> Option<[f32; 6]> {
    let turn = (rotate.rem_euclid(360) as f32).to_radians();
    // Exact quarter turns; `+ 0.0` turns a negative zero into a plain one.
    let (sin, cos) = (turn.sin().round() + 0.0, turn.cos().round() + 0.0);
    (rotate.rem_euclid(360) != 0).then_some([cos, sin, -sin + 0.0, cos, 0.0, 0.0])
}

/// `rect` given a new displayed size, its displayed top-left corner kept.
pub(crate) fn anchored(rect: [f32; 4], rotate: i64, (width, height): (f32, f32)) -> [f32; 4] {
    let [left, bottom, right, top] = rect;
    match rotate.rem_euclid(360) {
        // Displayed top: the page's left edge; displayed left: its bottom.
        90 => [left, bottom, left + height, bottom + width],
        // Displayed top: the page's bottom edge; displayed left: its right.
        180 => [right - width, bottom, right, bottom + height],
        // Displayed top: the page's right edge; displayed left: its top.
        270 => [right - height, top - width, right, top],
        _ => [left, top - height, left + width, top],
    }
}

fn check_rect([left, bottom, right, top]: [f32; 4]) -> Result<()> {
    if !in_range((left, bottom)) || !in_range((right, top)) || right - left < 0.5 || top - bottom < 0.5 {
        return Err(Error::InvalidEdit("an item needs a place on the page"));
    }
    Ok(())
}

/// The page-space box displayed with its top-left corner at (`x`, `y`).
fn rect_at(x: f32, y: f32, width: f32, height: f32, view: &ViewTransform) -> [f32; 4] {
    super::TextBox::rect_at(x, y, width, height, view)
}

/// An appearance of `size` (its natural box), drawn by `content`, fitted to
/// `rect` and upright on a page turned by `rotate`.
fn fitted(rect: [f32; 4], rotate: i64, (width, height): (f32, f32), content: Vec<u8>) -> Appearance {
    Appearance {
        rect,
        bbox: [0.0, 0.0, width, height],
        matrix: upright(rotate),
        resources: Dict::default(),
        content,
        fonts: Vec::new(),
        images: Vec::new(),
    }
}

// ------------------------------------------------------------------ text

/// Text or a date typed anywhere on a page. It is one line per line typed,
/// as wide as its longest line.
#[derive(Debug, Clone, PartialEq)]
pub struct FillText {
    /// The box in page space (left, bottom, right, top).
    pub rect: [f32; 4],
    pub contents: String,
    /// Font size in points.
    pub size: f32,
    /// RGB, each 0 to 1.
    pub color: [f32; 3],
}

impl FillText {
    pub const DEFAULT_SIZE: f32 = 11.0;
    pub const DEFAULT_COLOR: [f32; 3] = [0.0, 0.0, 0.0];

    fn laid(contents: &str, size: f32) -> Result<(TextLayout, Vec<crate::pdf::text::NamedFont>)> {
        check_contents(contents)?;
        if contents.trim().is_empty() {
            return Err(Error::InvalidEdit("fill text needs some text"));
        }
        if !(1.0..=400.0).contains(&size) {
            return Err(Error::InvalidEdit("text is 1 to 400 points in size"));
        }
        let fonts = for_text(FontFamily::Sans, contents)?;
        let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
        let style = ParagraphStyle {
            direction: BaseDirection::Auto,
            align: Align::Start,
            width: None,
            line_height: LineHeight::Normal,
        };
        let laid = layout(contents, &[Span::new(0..contents.len(), 0, size)], &faces, &style)
            .map_err(|err| Error::Typeset(err.code()))?;
        Ok((laid, fonts))
    }

    /// The displayed width and height `contents` needs at `size`, padding included.
    pub fn size_for(contents: &str, size: f32) -> Result<(f32, f32)> {
        let (laid, _) = Self::laid(contents, size)?;
        Ok((laid.width + 2.0 * PADDING, laid.height + 2.0 * PADDING))
    }

    /// Text displayed with its top-left corner at (`x`, `y`).
    pub fn at(
        x: f32,
        y: f32,
        contents: &str,
        size: f32,
        color: [f32; 3],
        view: &ViewTransform,
    ) -> Result<Self> {
        let (width, height) = Self::size_for(contents, size)?;
        Ok(Self { rect: rect_at(x, y, width, height, view), contents: contents.into(), size, color })
    }

    /// New text for the item over `rect`, its displayed top-left corner kept.
    pub(crate) fn replacing(
        rect: [f32; 4],
        rotate: i64,
        contents: &str,
        size: f32,
        color: [f32; 3],
    ) -> Result<Self> {
        let natural = Self::size_for(contents, size)?;
        Ok(Self { rect: anchored(rect, rotate, natural), contents: contents.into(), size, color })
    }

    pub(crate) fn entries(&self) -> Vec<(Vec<u8>, Object)> {
        vec![
            (b"Name".to_vec(), Object::Name(TEXT.to_vec())),
            (b"Contents".to_vec(), Object::Str(text_string(&self.contents))),
            (b"DA".to_vec(), Object::Str(da(self.size, self.color).into_bytes())),
        ]
    }

    pub(crate) fn appearance(&self, rotate: i64) -> Result<Appearance> {
        check_color(self.color)?;
        check_rect(self.rect)?;
        let (laid, fonts) = Self::laid(&self.contents, self.size)?;
        let (width, height) = (laid.width + 2.0 * PADDING, laid.height + 2.0 * PADDING);
        let drawing = draw_text(&self.contents, &laid, &fonts, (PADDING, height - PADDING), self.color, "F")?;
        let mut appearance = fitted(self.rect, rotate, (width, height), drawing.content);
        appearance.fonts = drawing.fonts;
        Ok(appearance)
    }
}

// ----------------------------------------------------------------- marks

/// What a mark draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MarkKind {
    Check,
    Cross,
    Dot,
}

/// A tick, cross, or dot.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub rect: [f32; 4],
    pub kind: MarkKind,
    pub color: [f32; 3],
}

/// Marks are drawn in a box this many units square.
const MARK_BOX: f32 = 100.0;

/// The paths of a mark in a `MARK_BOX` square, in `color`.
pub(crate) fn mark_paths(kind: MarkKind, [r, g, b]: [f32; 3]) -> String {
    let (r, g, b) = (num(r), num(g), num(b));
    match kind {
        MarkKind::Check => format!("{r} {g} {b} RG 12 w 1 J 1 j\n16 52 m 40 26 l 86 80 l S\n"),
        MarkKind::Cross => format!("{r} {g} {b} RG 12 w 1 J\n20 20 m 80 80 l S\n20 80 m 80 20 l S\n"),
        // A circle of radius 24 from four Bézier quarters (k = 0.5523).
        MarkKind::Dot => format!(
            "{r} {g} {b} rg\n74 50 m 74 63.255 63.255 74 50 74 c 36.745 74 26 63.255 26 50 c\n\
             26 36.745 36.745 26 50 26 c 63.255 26 74 36.745 74 50 c f\n"
        ),
    }
}

/// A mark centred in a `width` × `height` box, as large as fits.
pub(crate) fn mark_in(kind: MarkKind, color: [f32; 3], (width, height): (f32, f32), scale: f32) -> String {
    let side = width.min(height) * scale;
    let factor = side / MARK_BOX;
    format!(
        "q {} 0 0 {} {} {} cm\n{}Q\n",
        num(factor),
        num(factor),
        num((width - side) / 2.0),
        num((height - side) / 2.0),
        mark_paths(kind, color)
    )
}

impl Mark {
    /// A new mark's side, in points.
    pub const DEFAULT_SIZE: f32 = 14.0;
    pub const DEFAULT_COLOR: [f32; 3] = [0.0, 0.0, 0.0];

    /// A mark `size` points square, displayed with its top-left corner at (`x`, `y`).
    pub fn at(x: f32, y: f32, size: f32, kind: MarkKind, color: [f32; 3], view: &ViewTransform) -> Self {
        Self { rect: rect_at(x, y, size, size, view), kind, color }
    }

    pub(crate) fn entries(&self) -> Vec<(Vec<u8>, Object)> {
        let (name, text) = match self.kind {
            MarkKind::Check => (CHECK, "✓"),
            MarkKind::Cross => (CROSS, "✗"),
            MarkKind::Dot => (DOT, "•"),
        };
        vec![
            (b"Name".to_vec(), Object::Name(name.to_vec())),
            (b"Contents".to_vec(), Object::Str(text_string(text))),
        ]
    }

    pub(crate) fn appearance(&self, rotate: i64) -> Result<Appearance> {
        check_color(self.color)?;
        check_rect(self.rect)?;
        Ok(fitted(self.rect, rotate, (MARK_BOX, MARK_BOX), mark_paths(self.kind, self.color).into_bytes()))
    }
}

// ------------------------------------------------------------ signatures

/// A point of a drawn signature: y grows down, in any unit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SignaturePoint {
    pub x: f32,
    pub y: f32,
    /// 0 to 1; 0.5 for a mouse.
    pub pressure: f32,
}

/// A signature drawn with a mouse, a pen, or a finger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawnSignature {
    pub strokes: Vec<Vec<SignaturePoint>>,
    /// The pen's nominal width, in the points' unit.
    pub width: f32,
}

/// A signature typed as text and set in one of the bundled families.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedSignature {
    pub text: String,
    pub family: FontFamily,
}

/// A signature (or initials), as the app keeps it (ADR 0045).
#[derive(Debug, Clone, PartialEq)]
pub enum Signature {
    Drawn(DrawnSignature),
    Typed(TypedSignature),
    /// A PNG or JPEG file's bytes.
    Image(Vec<u8>),
}

/// A drawn stroke's width at a pressure, in tenths (as ink).
fn pen_width(width: f32, pressure: f32) -> f32 {
    ((width * (0.5 + pressure.clamp(0.0, 1.0))) * 10.0).round() / 10.0
}

impl Signature {
    /// The colour drawn and typed signatures are placed in: dark blue ink.
    pub const DEFAULT_COLOR: [f32; 3] = [0.06, 0.13, 0.45];

    /// Checks it can be placed.
    pub fn check(&self) -> Result<()> {
        match self {
            Signature::Drawn(drawn) => {
                let count: usize = drawn.strokes.iter().map(Vec::len).sum();
                if drawn.strokes.is_empty()
                    || drawn.strokes.len() > MAX_STROKES
                    || drawn.strokes.iter().any(Vec::is_empty)
                    || count > MAX_POINTS
                {
                    return Err(Error::InvalidEdit("a drawn signature needs 1 to 1,000 strokes"));
                }
                let finite = |v: f32| v.is_finite() && v.abs() <= 1.0e5;
                if drawn
                    .strokes
                    .iter()
                    .flatten()
                    .any(|p| !finite(p.x) || !finite(p.y) || !p.pressure.is_finite())
                {
                    return Err(Error::InvalidEdit("a drawn signature's points are out of range"));
                }
                if !(0.1..=100.0).contains(&drawn.width) {
                    return Err(Error::InvalidEdit("a pen is 0.1 to 100 units wide"));
                }
                Ok(())
            }
            Signature::Typed(typed) => {
                let length = typed.text.chars().count();
                if typed.text.trim().is_empty() || length > MAX_TYPED {
                    return Err(Error::InvalidEdit("a typed signature is 1 to 200 characters"));
                }
                Ok(())
            }
            Signature::Image(data) => {
                if data.len() > MAX_SIGNATURE_IMAGE_BYTES {
                    return Err(Error::InvalidEdit("a signature image is at most 8 MB"));
                }
                image_size(data, &SIGNATURE_IMAGE).map(|_| ())
            }
        }
    }

    /// Its natural width and height (any unit): placing it keeps this aspect.
    pub fn natural_size(&self) -> Result<(f32, f32)> {
        self.check()?;
        Ok(match self {
            Signature::Drawn(drawn) => {
                let (_, size) = drawn_frame(drawn);
                size
            }
            Signature::Typed(typed) => {
                let (laid, _) = typed_layout(typed)?;
                (laid.width + 2.0 * PADDING, laid.height + 2.0 * PADDING)
            }
            Signature::Image(data) => {
                let (width, height) = image_size(data, &SIGNATURE_IMAGE)?;
                (width as f32, height as f32)
            }
        })
    }

    /// The content drawing it in its natural box, in `color` (images keep their own).
    fn appearance(&self, rect: [f32; 4], rotate: i64, color: [f32; 3]) -> Result<Appearance> {
        let size = self.natural_size()?;
        Ok(match self {
            Signature::Drawn(drawn) => {
                let ((left, top), (_, height)) = drawn_frame(drawn);
                let mut out = Content::default();
                let [r, g, b] = color;
                out.op(&format!("{} {} {} RG 1 J 1 j", num(r), num(g), num(b)));
                // y down in the drawing, y up in PDF.
                let point = |p: &SignaturePoint| (p.x - left, height - (p.y - top));
                for stroke in &drawn.strokes {
                    if let [only] = stroke.as_slice() {
                        out.op(&format!("{} w", num(pen_width(drawn.width, only.pressure))));
                        out.move_to(point(only));
                        out.line_to(point(only));
                        out.op("S");
                        continue;
                    }
                    let mut current: Option<String> = None;
                    for pair in stroke.windows(2) {
                        let width = num(pen_width(drawn.width, (pair[0].pressure + pair[1].pressure) / 2.0));
                        if current.as_deref() != Some(width.as_str()) {
                            if current.is_some() {
                                out.op("S");
                            }
                            out.op(&format!("{width} w"));
                            out.move_to(point(&pair[0]));
                            current = Some(width);
                        }
                        out.line_to(point(&pair[1]));
                    }
                    out.op("S");
                }
                fitted(rect, rotate, size, out.0)
            }
            Signature::Typed(typed) => {
                let (laid, fonts) = typed_layout(typed)?;
                let drawing = draw_text(&typed.text, &laid, &fonts, (PADDING, size.1 - PADDING), color, "F")?;
                let mut appearance = fitted(rect, rotate, size, drawing.content);
                appearance.fonts = drawing.fonts;
                appearance
            }
            Signature::Image(data) => {
                let image = image_to_embed(data, "Im0", &SIGNATURE_IMAGE)?;
                let content = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q\n", num(size.0), num(size.1));
                let mut appearance = fitted(rect, rotate, size, content.into_bytes());
                appearance.images = vec![image];
                appearance
            }
        })
    }
}

/// A drawn signature's top-left corner (the strokes' bounds less a margin
/// for the widest line) and its size with that margin.
fn drawn_frame(drawn: &DrawnSignature) -> ((f32, f32), (f32, f32)) {
    let [left, top, right, bottom] = super::bounds(drawn.strokes.iter().flatten().map(|p| (p.x, p.y)));
    let margin = drawn.width * 0.75 + 0.5;
    ((left - margin, top - margin), (right - left + 2.0 * margin, bottom - top + 2.0 * margin))
}

fn typed_layout(typed: &TypedSignature) -> Result<(TextLayout, Vec<crate::pdf::text::NamedFont>)> {
    let fonts = for_text(typed.family, &typed.text)?;
    let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
    let style = ParagraphStyle::default();
    // One line: line breaks become spaces.
    let text = typed.text.replace(['\n', '\r'], " ");
    let laid = layout(&text, &[Span::new(0..text.len(), 0, TYPED_SIZE)], &faces, &style)
        .map_err(|err| Error::Typeset(err.code()))?;
    Ok((laid, fonts))
}

/// A signature placed on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureStamp {
    pub rect: [f32; 4],
    pub signature: Signature,
    /// The ink of drawn and typed signatures.
    pub color: [f32; 3],
}

impl SignatureStamp {
    /// `signature`, `width` points wide as displayed and as tall as its
    /// aspect makes it, displayed with its top-left corner at (`x`, `y`).
    pub fn at(
        x: f32,
        y: f32,
        width: f32,
        signature: Signature,
        color: [f32; 3],
        view: &ViewTransform,
    ) -> Result<Self> {
        let (w, h) = signature.natural_size()?;
        if !(4.0..=14_400.0).contains(&width) {
            return Err(Error::InvalidEdit("a signature is 4 to 14,400 points wide"));
        }
        let height = width * h / w.max(0.01);
        Ok(Self { rect: rect_at(x, y, width, height, view), signature, color })
    }

    pub(crate) fn entries(&self) -> Vec<(Vec<u8>, Object)> {
        let mut entries = vec![(b"Name".to_vec(), Object::Name(SIGNATURE.to_vec()))];
        if let Signature::Typed(typed) = &self.signature {
            entries.push((b"Contents".to_vec(), Object::Str(text_string(&typed.text))));
        }
        entries
    }

    pub(crate) fn appearance(&self, rotate: i64) -> Result<Appearance> {
        check_color(self.color)?;
        check_rect(self.rect)?;
        self.signature.appearance(self.rect, rotate, self.color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ViewTransform {
        ViewTransform::unrotated(800.0)
    }

    #[test]
    fn fill_text_is_as_wide_as_its_text_and_set_in_its_script() {
        let (short, h) = FillText::size_for("Ali", 11.0).expect("lays out");
        let (long, _) = FillText::size_for("Ali Hassan Mohammed", 11.0).expect("lays out");
        assert!(long > 2.0 * short && h > 11.0 && h < 20.0, "{short} {long} {h}");
        let text =
            FillText::at(100.0, 100.0, "محمد علي", 11.0, FillText::DEFAULT_COLOR, &view()).expect("fits");
        assert_eq!(text.rect[0], 100.0);
        assert!((text.rect[3] - 700.0).abs() < 0.01, "top-left as displayed: {:?}", text.rect);
        let appearance = text.appearance(0).expect("valid");
        assert_eq!(appearance.fonts[0].base_font.split('+').nth(1), Some("IBMPlexSansArabic"));
        assert!(text.entries().contains(&(b"Name".to_vec(), Object::Name(TEXT.to_vec()))));
        assert!(matches!(FillText::size_for("  ", 11.0), Err(Error::InvalidEdit(_))));
    }

    #[test]
    fn changed_text_keeps_its_displayed_top_left_corner() {
        let rect = [100.0, 500.0, 160.0, 520.0];
        assert_eq!(anchored(rect, 0, (80.0, 30.0)), [100.0, 490.0, 180.0, 520.0]);
        assert_eq!(anchored(rect, 90, (80.0, 30.0)), [100.0, 500.0, 130.0, 580.0]);
        assert_eq!(anchored(rect, 180, (80.0, 30.0)), [80.0, 500.0, 160.0, 530.0]);
        assert_eq!(anchored(rect, 270, (80.0, 30.0)), [130.0, 440.0, 160.0, 520.0]);
    }

    #[test]
    fn marks_draw_paths_in_their_box() {
        let mark = Mark::at(10.0, 10.0, 14.0, MarkKind::Check, [0.0; 3], &view());
        let appearance = mark.appearance(0).expect("valid");
        assert_eq!(appearance.bbox, [0.0, 0.0, 100.0, 100.0]);
        let content = String::from_utf8(appearance.content).expect("ascii");
        assert!(content.contains("16 52 m 40 26 l 86 80 l S"), "{content}");
        assert!(mark_in(MarkKind::Dot, [0.0; 3], (20.0, 10.0), 0.8).starts_with("q 0.08 0 0 0.08 6 1 cm"));
        // On a turned page it stands upright.
        assert_eq!(mark.appearance(90).expect("valid").matrix, Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]));
    }

    #[test]
    fn signatures_keep_their_aspect() {
        let point = |x: f32, y: f32| SignaturePoint { x, y, pressure: 0.5 };
        let drawn = Signature::Drawn(DrawnSignature {
            strokes: vec![vec![point(10.0, 10.0), point(210.0, 60.0)], vec![point(50.0, 30.0)]],
            width: 2.0,
        });
        let (w, h) = drawn.natural_size().expect("valid");
        assert!((w - 204.0).abs() < 0.01 && (h - 54.0).abs() < 0.01, "{w} × {h}");
        let stamp =
            SignatureStamp::at(100.0, 100.0, 120.0, drawn, Signature::DEFAULT_COLOR, &view()).expect("fits");
        let [l, b, r, t] = stamp.rect;
        assert!(((r - l) / (t - b) - w / h).abs() < 0.01);
        let content = String::from_utf8(stamp.appearance(0).expect("valid").content).expect("ascii");
        // y is flipped: the first point (top-left of the strokes) is high in the box.
        assert!(content.contains("2 52 m"), "{content}");

        let typed =
            Signature::Typed(TypedSignature { text: "سارة Sarah".into(), family: FontFamily::Naskh });
        let stamp =
            SignatureStamp::at(0.0, 0.0, 150.0, typed, Signature::DEFAULT_COLOR, &view()).expect("fits");
        let appearance = stamp.appearance(0).expect("valid");
        assert_eq!(appearance.fonts[0].base_font.split('+').nth(1), Some("Amiri-Regular"));

        let image = Signature::Image(crate::pdf::image::tests::png(40, 10));
        assert_eq!(image.natural_size().expect("png"), (40.0, 10.0));
        let stamp = SignatureStamp::at(0.0, 0.0, 80.0, image, [0.0; 3], &view()).expect("fits");
        let appearance = stamp.appearance(0).expect("valid");
        assert_eq!(appearance.images.len(), 1);
        assert_eq!(appearance.content, b"q 40 0 0 10 0 0 cm /Im0 Do Q\n");
    }

    #[test]
    fn bad_signatures_are_refused() {
        let empty = Signature::Drawn(DrawnSignature { strokes: vec![], width: 2.0 });
        assert!(matches!(empty.check(), Err(Error::InvalidEdit(_))));
        let blank = Signature::Typed(TypedSignature { text: " ".into(), family: FontFamily::Sans });
        assert!(matches!(blank.check(), Err(Error::InvalidEdit(_))));
        assert!(matches!(Signature::Image(b"not an image".to_vec()).check(), Err(Error::InvalidEdit(_))));
    }
}
