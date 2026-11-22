//! Editing a block of a page's text in place (spec §7, steps 3–6; ADR 0024).
//!
//! A [`BlockEditor`] is made once per block being edited:
//!
//! - **the font:** the paragraph's own when the engine has it (bundled or
//!   installed, matched by name and style), otherwise the closest one, and
//!   then the *whole* paragraph is set in it (spec §7.4), with the
//!   substitution reported;
//! - **the box:** the block's own, as displayed. A paragraph reflows inside
//!   its width; a single line (a heading, a label, a cell) grows along its
//!   line as typed, the way its alignment points, up to the next text beside
//!   it or the page's edge, and only then wraps. The first baseline stays
//!   where the original's was; lines keep the original's distance apart;
//!   alignment and direction are the block's. Text may grow or shrink
//!   downwards, but does not flow into other blocks (spec §7.6).
//!
//! Each layout (one per keystroke while editing) reuses that setting, and
//! applying the edit removes the block's operations and writes the new text
//! in one update ([`crate::Document::with_replaced_block`]).

use std::sync::OnceLock;

use super::{NewText, PageAnalysis, Refusal, Setting, SourceOp};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, candidates, fallbacks};
use crate::pdf::text::NamedFont;
use crate::text::Direction;
use crate::typeset::{
    Align, BaseDirection, FontFace, FontLibrary, FontRequest, LineHeight, MatchKind, TextLayout, match_font,
};
use crate::view::ViewTransform;

/// The font a block is set in while edited.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockFont {
    /// The PDF's font, as `/BaseFont` names it (without a subset tag; empty
    /// for an unnamed Type 3 font).
    pub original: String,
    /// The font used, by its PostScript name.
    pub used: String,
    /// Exact: the same font. SameFamily: another style of it. Substitute:
    /// another family, chosen by classification and style.
    pub kind: MatchKind,
    /// It came from the installed fonts, not the bundled ones.
    pub installed: bool,
}

/// One block being edited.
pub struct BlockEditor {
    pub page: usize,
    pub block: usize,
    /// The block's text as it was, made editable: soft line breaks become
    /// spaces, so the paragraph reflows.
    pub text: String,
    pub font: BlockFont,
    /// The box as displayed: its left edge, width, and height, and its first
    /// baseline. Text taller than `height` grows into what is below it.
    pub x: f32,
    pub width: f32,
    pub height: f32,
    pub first_baseline: f32,
    pub size: f32,
    pub color: [f32; 3],
    /// Its runs differ in size or colour. The paragraph is set in the first
    /// run's style, so those differences go (styled runs come later).
    pub mixed: bool,
    pub align: Align,
    pub direction: BaseDirection,
    line_height: LineHeight,
    fonts: Vec<NamedFont>,
    sources: Vec<SourceOp>,
}

/// The installed fonts, read once.
fn library() -> &'static FontLibrary {
    static LIBRARY: OnceLock<FontLibrary> = OnceLock::new();
    LIBRARY.get_or_init(FontLibrary::system)
}

/// The closest font to `name` for `text`: bundled or installed, whichever
/// matches better (bundled when equal), then the bundled fallbacks.
fn choose_font(
    name: &str,
    widths: &[(char, f32)],
    text: &str,
    installed: bool,
) -> Result<(Vec<NamedFont>, BlockFont)> {
    let mut request = FontRequest::from_pdf(name, None, None, None, None);
    // The glyphs' widths as drawn: a substitute that comes close keeps the
    // lines where they broke.
    request.widths = widths.to_vec();
    let bundled = candidates()?;
    let faces: Vec<FontFace> = bundled.iter().map(|font| font.face.clone()).collect();
    let from_bundle = match_font(&request, text, &faces).map(|found| {
        let font = bundled[found.index].clone();
        (found, font)
    });
    let from_system =
        installed.then(|| library().find(&request, text)).flatten().map(|(face, entry, found)| {
            let ps =
                entry.traits.postscript_name.clone().unwrap_or_else(|| entry.traits.family.replace(' ', ""));
            (found, NamedFont { face, name: ps })
        });
    let rank = |found: &crate::typeset::FontMatch| (!found.covers_text, found.kind);
    let (found, chosen, is_installed) = match (from_bundle, from_system) {
        (Some(b), Some(s)) if rank(&s.0) < rank(&b.0) => (s.0, s.1, true),
        (Some(b), _) => (b.0, b.1, false),
        (None, Some(s)) => (s.0, s.1, true),
        (None, None) => return Err(Error::Typeset("no font to set the text in")),
    };
    let font = BlockFont {
        original: name.to_string(),
        used: chosen.name.clone(),
        kind: found.kind,
        installed: is_installed,
    };
    let mut fonts = vec![chosen];
    for fallback in fallbacks()? {
        if fonts.iter().all(|font| font.name != fallback.name) {
            fonts.push(fallback);
        }
    }
    Ok((fonts, font))
}

/// The room a one-line block has to grow along its line, as displayed: from
/// its fixed side (its alignment's) to the nearest other text on that line,
/// an em short of it, or to the page's edge. Centred text grows both ways,
/// as far as the closer limit allows.
fn room_along_line(
    analysis: &PageAnalysis,
    block: usize,
    view: &ViewTransform,
    page_width: f32,
    size: f32,
    align: Align,
) -> (f32, f32) {
    let shown = view.rect(analysis.blocks[block].bounds);
    let (left, right) = (shown.x, shown.x + shown.width);
    let (mut limit_left, mut limit_right) = (0.0f32, page_width);
    for (index, other) in analysis.blocks.iter().enumerate() {
        if index == block {
            continue;
        }
        let beside = view.rect(other.bounds);
        let overlaps = beside.y < shown.y + shown.height && shown.y < beside.y + beside.height;
        if !overlaps {
            continue;
        }
        if beside.x >= right {
            limit_right = limit_right.min(beside.x - size);
        } else if beside.x + beside.width <= left {
            limit_left = limit_left.max(beside.x + beside.width + size);
        }
    }
    // Never less than the block itself.
    let limit_left = limit_left.min(left);
    let limit_right = limit_right.max(right);
    match align {
        Align::Right | Align::End => (limit_left, right - limit_left + 0.5),
        Align::Center | Align::Justify => {
            let centre = (left + right) / 2.0;
            let half = (centre - limit_left).min(limit_right - centre);
            (centre - half, 2.0 * half + 0.5)
        }
        Align::Left | Align::Start => (left, limit_right - left + 0.5),
    }
}

impl BlockEditor {
    /// An editor for block `block` of `analysis` (of page `page`, displayed
    /// `page_width` wide), or why it cannot be edited in place. `view` is the
    /// page's map to the display. With `installed`, installed fonts are
    /// candidates too.
    pub fn new(
        analysis: &PageAnalysis,
        page: usize,
        block: usize,
        view: &ViewTransform,
        page_width: f32,
        installed: bool,
    ) -> Result<Self> {
        let edited = analysis.blocks.get(block).ok_or(Error::InvalidEdit("no such block"))?;
        match analysis.refusal(block, view.upright()) {
            // Its text is not what it draws: setting it again would write that.
            Some(Refusal::NeedsOcr) => {
                return Err(Error::EditUnsupported(
                    "this page's text cannot be read reliably (it needs OCR)",
                ));
            }
            // Its remaining glyphs would stay, or another block's would go.
            Some(Refusal::Untraced) => {
                return Err(Error::EditUnsupported(
                    "some of this text cannot be traced to the page's content",
                ));
            }
            Some(Refusal::Shared) => {
                return Err(Error::EditUnsupported("this text is drawn together with other text"));
            }
            Some(Refusal::Turned) => {
                return Err(Error::EditUnsupported("text on a turned page cannot be edited yet"));
            }
            None => {}
        }
        let style =
            edited.runs.first().map(|run| run.style.clone()).ok_or(Error::InvalidEdit("an empty block"))?;
        let text = edited.text.replace('\n', " ");
        let (fonts, font) = choose_font(&style.font, &edited.widths, &text, installed)?;
        let shown = view.rect(edited.bounds);
        let first = edited.lines.first().ok_or(Error::InvalidEdit("an empty block"))?;
        let first_baseline = view.point(first.left, first.baseline).1;
        let align = match edited.align {
            super::BlockAlign::Left => Align::Left,
            super::BlockAlign::Right => Align::Right,
            super::BlockAlign::Center => Align::Center,
            super::BlockAlign::Justified => Align::Justify,
        };
        let (x, width) = if edited.lines.len() == 1 && align != Align::Justify {
            room_along_line(analysis, block, view, page_width, style.size, align)
        } else {
            // A hair of room: the same text in the same font re-breaks where it did.
            (shown.x, shown.width + 0.5)
        };
        Ok(BlockEditor {
            page,
            block,
            text,
            font,
            x,
            width,
            height: shown.height,
            first_baseline,
            size: style.size,
            color: style.color,
            mixed: edited.runs.iter().any(|run| {
                (run.style.size - style.size).abs() > 0.1
                    || run.style.color.iter().zip(style.color).any(|(a, b)| (a - b).abs() > 0.02)
            }),
            align,
            direction: if edited.direction == Direction::Rtl {
                BaseDirection::Rtl
            } else {
                BaseDirection::Ltr
            },
            line_height: edited.line_pitch.map_or(LineHeight::Normal, LineHeight::Points),
            fonts,
            sources: edited.sources.iter().map(|&op| analysis.ops[op].clone()).collect(),
        })
    }

    /// The new text as the writer takes it, placed so its first baseline is
    /// the original's.
    fn new_text(&self, text: &str) -> Result<(NewText, TextLayout)> {
        let mut new = NewText {
            page: self.page,
            x: self.x,
            y: 0.0,
            width: self.width,
            text: text.to_string(),
            family: FontFamily::Sans,
            size: self.size,
            color: self.color,
            align: self.align,
            direction: self.direction,
        };
        let (laid, _) = new.laid_out_with(&self.setting())?;
        new.y = self.first_baseline - laid.lines.first().map_or(0.0, |line| line.baseline);
        Ok((new, laid))
    }

    fn setting(&self) -> Setting {
        Setting { fonts: Some(self.fonts.clone()), line_height: self.line_height }
    }

    /// `text` laid out as it will be written, and the top of its box as
    /// displayed (its left edge is [`BlockEditor::x`]).
    pub fn layout(&self, text: &str) -> Result<(TextLayout, f32)> {
        let (new, laid) = self.new_text(text)?;
        Ok((laid, new.y))
    }

    /// The faces the text is set in, first tried first, with their PostScript names.
    pub fn faces(&self) -> Vec<(FontFace, String)> {
        self.fonts.iter().map(|font| (font.face.clone(), font.name.clone())).collect()
    }

    /// The bytes to append to `bytes` to replace the block with `text` (or
    /// remove it, when `text` is blank): one incremental update.
    pub(crate) fn update(
        &self,
        bytes: &[u8],
        password: Option<&str>,
        view: &ViewTransform,
        text: &str,
    ) -> Result<Vec<u8>> {
        let mut update = super::remove_text_update(bytes, password, self.page, &self.sources)?;
        if text.trim().is_empty() {
            return Ok(update);
        }
        let (new, _) = self.new_text(text)?;
        let mut removed = bytes.to_vec();
        removed.extend_from_slice(&update);
        update.extend(super::add_text_update_with(&removed, password, view, &new, &self.setting())?);
        Ok(update)
    }
}
