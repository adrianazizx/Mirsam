//! Text layout for *writing* text (ADR 0020): the engine that turns a styled
//! paragraph into positioned glyphs, for the PDF writer and for the editor
//! surface that must show exactly what will be saved.
//!
//! Pipeline, all inside [`layout`]:
//!
//! 1. **Itemize** — bidi paragraphs and embedding levels (UAX #9, `unicode-bidi`),
//!    script runs (ICU Script / Script_Extensions, brackets paired), and a font per
//!    grapheme cluster from the caller's font list (fallback).
//! 2. **Shape** — each item with `harfrust` in its own direction, with the
//!    surrounding paragraph as context, to measure it.
//! 3. **Break** — UAX #14 opportunities (ICU4X); greedy fill; a word wider than
//!    the line is broken between grapheme clusters.
//! 4. **Position** — per line, the bidi runs in visual order (rule L1 applied to
//!    the line), each piece reshaped exactly, aligned or justified.
//!
//! Nothing here knows about PDF or a UI. The output is a list of lines, each a
//! list of glyph runs in visual order with every glyph's position and the text
//! it came from; a writer turns those into text objects (ADR 0005), an editor
//! into a caret map.
//!
//! Coordinates are in points. `x` grows to the right from the left edge of the
//! layout box. Each line has a `baseline`, measured *down* from the top of the
//! box; a glyph's `y_offset` is measured *up* from that baseline (raised marks
//! are positive), as in PDF text space.

mod font;
mod itemize;
mod lines;
mod matching;
mod shape;
mod subset;

use std::ops::Range;

pub use font::{FontFace, LineMetrics};
pub use lines::layout;
pub use matching::{FontMatch, FontRequest, FontTraits, MatchKind, match_font};
pub use subset::{FontSubset, SubsetGlyph, subset_fonts};

/// Why a layout could not be made.
#[derive(Debug, thiserror::Error)]
pub enum TypesetError {
    #[error("the font file cannot be parsed")]
    FontParse,
    #[error("shaping failed: {0}")]
    Shape(String),
    #[error("invalid spans: {0}")]
    Spans(String),
    #[error("the font cannot be subset: {0}")]
    Subset(String),
}

impl TypesetError {
    /// Stable identifier, for logs and for the UI's message catalogue.
    pub fn code(&self) -> &'static str {
        match self {
            TypesetError::FontParse => "font_unreadable",
            TypesetError::Shape(_) => "shaping_failed",
            TypesetError::Spans(_) => "invalid_text_spans",
            TypesetError::Subset(_) => "font_subset_failed",
        }
    }
}

/// Paragraph direction (spec §6.3: auto-detected from the first strong
/// character, overridable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaseDirection {
    /// UAX #9 rules P2–P3: the first strong character decides; left to right if
    /// there is none.
    #[default]
    Auto,
    Ltr,
    Rtl,
}

/// Horizontal alignment. `Start` and `End` follow the paragraph's direction
/// (spec §6.3: "alignment defaults follow direction").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
    /// Every line but a paragraph's last is stretched to the full width by
    /// widening its spaces. Kashida justification is P2.
    Justify,
}

/// Distance from one baseline to the next.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LineHeight {
    /// The fonts' own ascent + descent + line gap.
    #[default]
    Normal,
    /// A multiple of `Normal` (word processors' "1.15 lines").
    Multiple(f32),
    /// A fixed baseline-to-baseline distance in points, e.g. measured from the
    /// lines of an existing paragraph being edited.
    Points(f32),
}

/// A styled stretch of the text. Spans are in logical order, contiguous, and
/// cover the whole text.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// Byte range of the text.
    pub range: Range<usize>,
    /// Index into the font list passed to [`layout`]. Characters this font
    /// lacks fall back to the first font in the list that has them.
    pub font: usize,
    /// Font size in points.
    pub size: f32,
    /// BCP 47 language tag, e.g. `ar`, `fa`, `ur`: selects language-specific
    /// glyph forms (Persian and Urdu digits) where the font has them.
    pub language: Option<String>,
}

impl Span {
    pub fn new(range: Range<usize>, font: usize, size: f32) -> Self {
        Span { range, font, size, language: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphStyle {
    pub direction: BaseDirection,
    pub align: Align,
    /// Line width in points; `None` lays each paragraph out on one line
    /// (breaking only at hard line breaks).
    pub width: Option<f32>,
    pub line_height: LineHeight,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        ParagraphStyle {
            direction: BaseDirection::Auto,
            align: Align::Start,
            width: None,
            line_height: LineHeight::Normal,
        }
    }
}

/// The laid-out text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    /// In logical order (the first line holds the start of the text).
    pub lines: Vec<Line>,
    /// The box width: the requested width, or the widest line when unbounded.
    pub width: f32,
    /// Sum of the line heights.
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The text this line holds, in logical order, including trailing spaces
    /// and the line break character that ended it, if any.
    pub text_range: Range<usize>,
    /// Direction of the paragraph this line belongs to.
    pub rtl: bool,
    /// Top of the line box, down from the top of the layout box.
    pub top: f32,
    /// Baseline, down from the top of the layout box.
    pub baseline: f32,
    /// Height of the line box (baseline to baseline for consecutive lines).
    pub height: f32,
    /// Largest ascent and descent of the fonts on the line, in points.
    pub ascent: f32,
    pub descent: f32,
    /// Left edge of the line's visible content, after alignment.
    pub x: f32,
    /// Width of the visible content. Trailing spaces hang outside it, on the
    /// side the paragraph ends (right for LTR, left for RTL).
    pub width: f32,
    /// Glyph runs in visual order, left to right.
    pub runs: Vec<GlyphRun>,
}

/// Glyphs from one font at one size, in one direction.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    /// Index into the font list passed to [`layout`].
    pub font: usize,
    pub size: f32,
    pub rtl: bool,
    /// ISO 15924 script tag the run was shaped as, e.g. `*b"Arab"`.
    pub script: [u8; 4],
    /// Text the run covers, in logical order.
    pub text_range: Range<usize>,
    /// Visual order, left to right.
    pub glyphs: Vec<PositionedGlyph>,
    /// Glyph ↔ text correspondence, in visual order. Every glyph belongs to
    /// exactly one cluster; every character of `text_range` to exactly one.
    pub clusters: Vec<Cluster>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    /// Glyph id in the font.
    pub id: u32,
    /// Where the glyph is drawn: the pen position plus the shaper's offset,
    /// from the left edge of the layout box.
    pub x: f32,
    /// Offset up from the line's baseline (a raised mark is positive).
    pub y_offset: f32,
    /// How far the pen moves after this glyph.
    pub advance: f32,
    /// Index into the run's `clusters`.
    pub cluster: usize,
}

/// The smallest unit that maps glyphs to text: a ligature (several characters,
/// one glyph), a decomposed letter (one character, several glyphs), or the
/// usual one-to-one.
#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    /// Byte range of the source text.
    pub text: Range<usize>,
    /// Range into the run's `glyphs`.
    pub glyphs: Range<usize>,
}

impl TextLayout {
    /// Every glyph run, line by line.
    pub fn runs(&self) -> impl Iterator<Item = &GlyphRun> {
        self.lines.iter().flat_map(|l| l.runs.iter())
    }
}
