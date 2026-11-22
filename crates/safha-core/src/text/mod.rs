//! Text extraction in logical reading order (docs/SPEC.md section 6.1).
//!
//! PDF has no reading order. PDFium reports characters in content-stream order,
//! and producers disagree about what that order means: Chrome and CoreText both
//! store Arabic glyphs in visual order, split words across runs, and shift glyphs
//! off the baseline for cursive attachment, which makes PDFium's own text output
//! insert line breaks inside words. So this module ignores stream order and
//! rebuilds everything from glyph geometry:
//!
//! 1. collect glyphs (one per PDFium character, PDFium's generated spaces and
//!    line breaks dropped),
//! 2. cluster them into lines by baseline and split lines into segments at
//!    column-sized gaps,
//! 3. attach combining marks (tashkeel) to their base letters,
//! 4. place word spaces: where the PDF draws a space glyph, where a space
//!    character separates two neighbouring glyphs in the stream, and — for
//!    producers that write neither — where the gap is wide enough,
//! 5. group segments into blocks and decide each block's direction,
//! 6. convert each line from visual to logical order with the Unicode
//!    Bidirectional Algorithm, keeping every glyph's own text intact,
//! 7. order blocks for reading (columns right to left on right-to-left pages).
//!
//! Steps 2–7 are pure functions over [`Glyph`]s (`layout.rs`), so they are unit
//! tested without PDFium.

mod glyphs;
mod layout;

use serde::Serialize;

pub use layout::{Glyph, SpaceMark, analyze};

/// Writing direction of a block of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Ltr,
    Rtl,
}

/// An axis-aligned rectangle in PDF page space (points, y up).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl Rect {
    pub(crate) fn union(self, other: Rect) -> Rect {
        Rect {
            left: self.left.min(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.max(other.right),
            top: self.top.max(other.top),
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.top - self.bottom
    }
}

/// One visible unit of a line: a base character with its combining marks, a
/// ligature's characters, or an inferred word space.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextSpan {
    /// Byte range of this span in [`TextLine::text`].
    pub start: usize,
    pub end: usize,
    /// Where it is drawn. `None` for inferred spaces, which have no glyph.
    pub rect: Option<Rect>,
    /// The PDFium characters that drew it (none for an inferred space). Not
    /// sent to the UI.
    #[serde(skip)]
    pub chars: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextLine {
    /// The line's text in logical (reading) order.
    pub text: String,
    /// The line's spans in logical order; selection maps through these.
    pub spans: Vec<TextSpan>,
    pub bounds: Rect,
    /// Its baseline's height in page space. Not sent to the UI.
    #[serde(skip)]
    pub baseline: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextBlock {
    pub direction: Direction,
    /// Lines top to bottom.
    pub lines: Vec<TextLine>,
    pub bounds: Rect,
}

/// How far the page's character mapping can be trusted.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TextQuality {
    /// Nothing suspicious found.
    Good,
    /// The page draws text, but its glyph-to-Unicode mapping looks broken (for
    /// example a font whose dots are separate glyphs mapped to `#` and `%`).
    /// Copy and search will be wrong; the page is a candidate for OCR.
    Suspect { suspicious_glyphs: usize, glyphs: usize },
    /// No text at all: a scanned or image-only page.
    NoText,
}

/// A page's text in reading order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageText {
    /// Blocks in reading order.
    pub blocks: Vec<TextBlock>,
    /// The page's dominant direction, which decides column order.
    pub direction: Direction,
    pub quality: TextQuality,
}

impl PageText {
    /// The text drawn inside `rects` (page space), in reading order: the
    /// spans whose centre lies in one of them, with the word spaces between
    /// two such spans; lines joined by a space. What a markup annotation
    /// marks (ADR 0041).
    pub fn text_in(&self, rects: &[Rect]) -> String {
        let inside = |rect: &Rect| {
            let (x, y) = ((rect.left + rect.right) / 2.0, (rect.bottom + rect.top) / 2.0);
            rects.iter().any(|r| r.left <= x && x <= r.right && r.bottom <= y && y <= r.top)
        };
        let mut lines = Vec::new();
        for line in self.blocks.iter().flat_map(|block| &block.lines) {
            let kept: Vec<Option<bool>> =
                line.spans.iter().map(|span| span.rect.as_ref().map(inside)).collect();
            let mut text = String::new();
            for (i, span) in line.spans.iter().enumerate() {
                // An inferred space counts between two kept glyphs.
                let keep = kept[i].unwrap_or_else(|| {
                    kept[..i].iter().rev().find_map(|k| *k).unwrap_or(false)
                        && kept[i + 1..].iter().find_map(|k| *k).unwrap_or(false)
                });
                if keep && let Some(part) = line.text.get(span.start..span.end) {
                    text.push_str(part);
                }
            }
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() {
                lines.push(text);
            }
        }
        lines.join(" ")
    }

    /// Plain text: lines joined with `\n`, blocks separated by a blank line.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| block.lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join("\n"))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

pub(crate) use glyphs::{collect_glyphs, collect_space_marks};
