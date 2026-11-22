//! Editing a paragraph of page text in place (M3, ADR 0023).
//!
//! 1. **Read** the page's content with our own interpreter (`pdf::content`)
//!    and rebuild its paragraphs ([`crate::blocks`]).
//! 2. **Remove** the paragraph's text-showing operators by byte range. PDFium
//!    never regenerates content (ADR 0005's amendment): every other byte of
//!    the page's content stays as it was. Where a later operator of the same
//!    text object depends on a removed one's advance, a `[n] TJ` that moves
//!    the same distance and draws nothing takes its place.
//! 3. **Write** the new text where the first removed operator was, so z-order
//!    and clipping stay: before its `BT`, under a `cm` that cancels the
//!    matrix in force there and places the box on the page as displayed.
//!    - If the paragraph's own font can draw every character (Latin text,
//!      each character has a code in the font), it is **reused**: the same
//!      font resource, widths from the font, no new font embedded.
//!    - Otherwise the paragraph is **substituted** as a whole: laid out by the
//!      text engine (`typeset`) in a bundled family and embedded as a subset.
//! 4. The result is an incremental update (ADR 0014): new content streams,
//!    the page's `/Contents` and `/Resources`, new fonts. Bundled fonts left
//!    unused by earlier edits drop out of the page's resources.
//!
//! Not edited: text in form XObjects, in encrypted files, text drawn in
//! Type 3 fonts or fonts with no way to read their codes as text, and text
//! that is not upright on the displayed page.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::blocks::{Atom, Block, BlockAlign, RunStyle, analyze};
use crate::edit::{Leaf, PageTree, Start, set};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, bold_of, candidates, fallbacks, family_of, for_text, oblique_of};
use crate::pdf::content::{Matrix, Op, Operand, ShowKind, Shown, interpret, tokenize};
use crate::pdf::fontinfo::{FontInfo, stream_data};
use crate::pdf::text::{NamedFont, Paint, draw_painted_text, write_font};
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};
use crate::text_edit::{free_prefix, num, page_contents, placement};
use crate::typeset::{
    Align, BaseDirection, FontFace, FontRequest, LineHeight, ParagraphStyle, Span, layout, match_font,
};
use crate::view::ViewTransform;

/// Beyond a single line's own width, it may grow to the page's margin.
const MARGIN: f32 = 36.0;

/// Why text cannot be edited in place: what the UI tells someone who presses on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
    /// An operator that drew it also drew other text.
    Shared,
    /// Its codes do not give its text exactly (letters drawn with separate
    /// dots that carry no text, CoreText's Arabic): it needs OCR.
    Unreadable,
    /// Its font cannot be read: Type 3, no `/ToUnicode`, an encoding this
    /// reader does not follow, vertical writing.
    Font,
    /// It is not upright on the page as displayed.
    Rotated,
    /// The file is encrypted.
    Encrypted,
    /// It is drawn in a way the editor does not follow yet (stroked text, an
    /// unusual text state, a content filter it cannot read).
    Unsupported,
}

/// A paragraph found on a page, as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextBlock {
    pub index: usize,
    /// Changes when the page's text does: an edit names the block it read.
    pub fingerprint: u32,
    /// The paragraph's box on the page as displayed: points from the top-left, y down.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub rtl: bool,
    pub align: BlockAlign,
    pub size: f32,
    pub color: [f32; 3],
    pub lines: usize,
    pub font: String,
    /// The block's own font can be used for new text (Latin characters it has codes for).
    pub font_reusable: bool,
    /// It holds more than one style (font, size, colour): editing keeps them,
    /// and typed text takes the style of the text before it (ADR 0024).
    pub styled: bool,
    /// Its styled runs, covering `text` in logical order.
    pub runs: Vec<StyledRun>,
    pub editable: bool,
    /// Why it is not editable.
    pub reason: Option<Refusal>,
}

/// A stretch of a paragraph's text in one style, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyledRun {
    /// UTF-16 offsets into the paragraph's `text`.
    pub start: usize,
    pub end: usize,
    /// The PDF's font, as `/BaseFont` names it (without a subset tag).
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    /// Points, as displayed.
    pub size: f32,
    /// sRGB, 0–1.
    pub color: [f32; 3],
}

impl TextBlock {
    /// Text that cannot be edited, where it sits on the page as displayed:
    /// a press there says why rather than placing new text over it.
    fn refused(index: usize, [x0, x1, top, bottom]: [f32; 4], reason: Refusal) -> Self {
        TextBlock {
            index,
            fingerprint: 0,
            x: x0,
            y: top,
            width: x1 - x0,
            height: bottom - top,
            text: String::new(),
            rtl: false,
            align: BlockAlign::Left,
            size: bottom - top,
            color: [0.0; 3],
            lines: 0,
            font: String::new(),
            font_reusable: false,
            styled: false,
            runs: Vec::new(),
            editable: false,
            reason: Some(reason),
        }
    }
}

/// What to put in a paragraph.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockEdit {
    pub page: usize,
    pub index: usize,
    pub fingerprint: u32,
    /// Empty removes the paragraph.
    pub text: String,
    /// The bundled family for the substitute; `None` chooses from the block's font.
    pub family: Option<FontFamily>,
}

/// What an edit did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedBlock {
    /// Set in a substitute font: the original font's name and the family used.
    pub substituted: Option<Substitution>,
    /// The paragraph's lines after the edit.
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Substitution {
    pub from: String,
    pub to: FontFamily,
    /// The face it is set in (a PostScript name): the closest bundled face
    /// to the original when the family was not chosen (ADR 0023).
    pub font: String,
}

struct StreamData {
    data: Vec<u8>,
    ops: Vec<Op>,
}

/// A page's content, read.
struct PageRead {
    items: Vec<Object>,
    streams: Vec<StreamData>,
    shown: Vec<Shown>,
    fonts: Vec<(Vec<u8>, Option<Rc<FontInfo>>)>,
    blocks: Vec<Block>,
    /// Where text that is not editable sits (x0, x1, top, bottom, displayed): growing text keeps off it.
    obstacles: Vec<[f32; 4]>,
    /// Why each obstacle is not editable.
    obstacle_reasons: Vec<Refusal>,
}

fn read_page(file: &File<'_>, tree: &PageTree, leaf: &Leaf, view: &ViewTransform) -> Result<PageRead> {
    if file.trailer.get(b"Encrypt").is_some() {
        return Err(Error::EditUnsupported("text in encrypted files cannot be edited yet"));
    }
    let items = page_contents(file, &leaf.dict)?;
    let mut streams = Vec::new();
    for item in &items {
        let data = stream_data(file, item)
            .ok_or(Error::EditUnsupported("the page's content uses a filter the editor cannot read"))?;
        let ops = tokenize(&data).ok_or(Error::EditUnsupported("the page's content cannot be read"))?;
        streams.push(StreamData { data, ops });
    }

    let resources = tree.attribute(leaf, b"Resources").and_then(|r| file.resolve(&r));
    let font_dict = resources
        .as_ref()
        .and_then(Object::as_dict)
        .and_then(|r| r.get(b"Font"))
        .and_then(|f| file.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap_or_default();
    let mut resolve = |name: &[u8]| -> Option<Rc<FontInfo>> {
        let dict = font_dict.get(name).and_then(|f| file.resolve(f))?;
        FontInfo::read(file, dict.as_dict()?).map(Rc::new)
    };

    let ops: Vec<Vec<Op>> = streams.iter().map(|s| s.ops.clone()).collect();
    let shown = interpret(&ops, &mut resolve);

    // One entry per font resource used, in order of first use.
    let mut fonts: Vec<(Vec<u8>, Option<Rc<FontInfo>>)> = Vec::new();
    for s in &shown {
        if !fonts.iter().any(|(name, _)| *name == s.font_name) {
            fonts.push((s.font_name.clone(), s.font.clone()));
        }
    }

    let (atoms, obstacles, obstacle_reasons) = atoms_of(&shown, &fonts, view);
    let mut blocks = analyze(atoms);
    // Eligible operators only: the block's text is rebuilt from these.
    for block in &mut blocks {
        block.shown.retain(|&i| shown.get(i).is_some());
    }
    Ok(PageRead { items, streams, shown, fonts, blocks, obstacles, obstacle_reasons })
}

/// Whether a text-showing operator can be read and written back, and where
/// its glyphs land in the page as displayed.
fn atoms_of(
    shown: &[Shown],
    fonts: &[(Vec<u8>, Option<Rc<FontInfo>>)],
    view: &ViewTransform,
) -> (Vec<Atom>, Vec<[f32; 4]>, Vec<Refusal>) {
    let view = Matrix(view.matrix());
    let mut atoms = Vec::new();
    let mut obstacles = Vec::new();
    let mut reasons = Vec::new();
    for (index, s) in shown.iter().enumerate() {
        let m = s.tm.then(s.ctm).then(view);
        let [a, b, c, d, ..] = m.0;
        let readable = s.font.as_ref().is_some_and(|f| f.usable())
            && matches!(s.render, 0 | 2)
            && !s.odd_state
            && (0.1..=1000.0).contains(&s.size)
            && s.glyphs.iter().all(|g| g.text.is_some());
        // Upright in the page as displayed: along the baseline, y down.
        let upright = a > 0.0 && b.abs() <= 0.02 * a && d < 0.0 && c.abs() <= 0.35 * d.abs();
        if !readable || !upright || s.glyphs.is_empty() {
            // Not editable, but it takes room: its box, corner by corner.
            if s.glyphs.is_empty() || s.render == 3 {
                continue;
            }
            let (low, high) = (-0.25 * s.size, 0.95 * s.size);
            let corners =
                [(0.0, low), (s.advance, low), (0.0, high), (s.advance, high)].map(|(x, y)| m.apply(x, y));
            let min = |f: fn(&(f64, f64)) -> f64| corners.iter().map(f).fold(f64::MAX, f64::min) as f32;
            let max = |f: fn(&(f64, f64)) -> f64| corners.iter().map(f).fold(f64::MIN, f64::max) as f32;
            obstacles.push([min(|p| p.0), max(|p| p.0), min(|p| p.1), max(|p| p.1)]);
            reasons.push(if !upright {
                Refusal::Rotated
            } else if !s.font.as_ref().is_some_and(|f| f.usable()) {
                Refusal::Font
            } else if !s.glyphs.iter().all(|g| g.text.is_some()) {
                Refusal::Unreadable
            } else {
                Refusal::Unsupported
            });
            continue;
        }
        let key = fonts.iter().position(|(name, _)| *name == s.font_name).unwrap_or(0);
        let size = (s.size * d.abs()) as f32;
        let baseline = m.apply(0.0, 0.0).1 as f32;
        let color = s.fill.unwrap_or([0.0; 3]);
        for g in &s.glyphs {
            let x0 = m.apply(g.x, 0.0).0 as f32;
            let width = (g.width / 1000.0 * s.size * s.tz * a) as f32;
            atoms.push(Atom {
                x0,
                x1: x0 + width,
                baseline,
                size,
                text: g.text.clone().unwrap_or_default(),
                shown: index,
                font: key,
                color,
            });
        }
    }
    (atoms, obstacles, reasons)
}

/// Obstacles of one reason that touch along a line (an em apart at most),
/// joined: one area per run of unreadable text rather than one per glyph.
fn refused_areas(obstacles: &[[f32; 4]], reasons: &[Refusal]) -> Vec<([f32; 4], Refusal)> {
    const MAX_AREAS: usize = 20_000;
    let mut areas: Vec<([f32; 4], Refusal)> = Vec::new();
    for (&area, &reason) in obstacles.iter().zip(reasons).take(MAX_AREAS) {
        let near = |[x0, x1, top, bottom]: [f32; 4], other: [f32; 4]| {
            let em = (bottom - top).max(other[3] - other[2]);
            top < other[3] && other[2] < bottom && x0 <= other[1] + em && other[0] <= x1 + em
        };
        match areas.iter_mut().find(|(other, r)| *r == reason && near(area, *other)) {
            Some((other, _)) => {
                *other = [
                    other[0].min(area[0]),
                    other[1].max(area[1]),
                    other[2].min(area[2]),
                    other[3].max(area[3]),
                ];
            }
            None => areas.push((area, reason)),
        }
    }
    areas
}

/// Whether the file is encrypted (its text is not read, ADR 0023).
pub(crate) fn encrypted(bytes: &[u8]) -> Result<bool> {
    Ok(Start::read(bytes)?.file.trailer.get(b"Encrypt").is_some())
}

fn fingerprint(block: &Block, shown: &[Shown]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            hash = (hash ^ u32::from(*b)).wrapping_mul(0x0100_0193);
        }
    };
    feed(block.text.as_bytes());
    if let Some(first) = block.shown.first().and_then(|&i| shown.get(i)) {
        feed(&first.stream.to_le_bytes());
        feed(&first.range.start.to_le_bytes());
    }
    hash
}

/// The page's paragraphs, in reading order.
pub fn text_blocks(bytes: &[u8], view: &ViewTransform, page: usize) -> Result<Vec<TextBlock>> {
    let Start { file, .. } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf =
        tree.leaves.get(page).ok_or(Error::PageOutOfRange { index: page, count: tree.leaves.len() })?;
    let read = read_page(&file, &tree, leaf, view)?;
    let mut blocks: Vec<TextBlock> = read
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let font = read.fonts.get(block.font).and_then(|(_, f)| f.clone());
            let reason = if block.shared {
                Some(Refusal::Shared)
            } else if block.shown.is_empty() || !readable(&block.text) {
                Some(Refusal::Unreadable)
            } else {
                None
            };
            let chars = styled_chars(block);
            let text: String = chars.iter().map(|&(c, _)| c).collect();
            let runs = style_runs(&chars)
                .into_iter()
                .map(|(range, style)| {
                    let info = read.fonts.get(style.font).and_then(|(_, f)| f.as_deref());
                    StyledRun {
                        start: text[..range.start].encode_utf16().count(),
                        end: text[..range.end].encode_utf16().count(),
                        font: info.map(|f| f.name.clone()).unwrap_or_default(),
                        bold: info.is_some_and(|f| f.bold),
                        italic: info.is_some_and(|f| f.italic),
                        size: style.size,
                        color: style.color,
                    }
                })
                .collect::<Vec<_>>();
            TextBlock {
                index,
                fingerprint: fingerprint(block, &read.shown),
                x: block.x0,
                y: block.top,
                width: block.x1 - block.x0,
                height: block.bottom - block.top,
                styled: runs.len() > 1,
                runs,
                text,
                rtl: block.rtl,
                align: block.align,
                size: block.size,
                color: block.color,
                lines: block.lines.len(),
                font: font.as_ref().map(|f| f.name.clone()).unwrap_or_default(),
                font_reusable: font.is_some_and(|f| f.usable() && !f.vertical),
                editable: reason.is_none(),
                reason,
            }
        })
        .collect();
    // After the paragraphs, so their indices stay the ones an edit names.
    for (area, reason) in refused_areas(&read.obstacles, &read.obstacle_reasons) {
        blocks.push(TextBlock::refused(blocks.len(), area, reason));
    }
    Ok(blocks)
}

/// Text that cannot be read at all here, as refused areas: the page's
/// blocks as PDFium reads them, each with `reason`.
pub(crate) fn refused_page(areas: impl IntoIterator<Item = [f32; 4]>, reason: Refusal) -> Vec<TextBlock> {
    areas.into_iter().enumerate().map(|(index, area)| TextBlock::refused(index, area, reason)).collect()
}

/// Text a font's codes gave up exactly: not dotless letters, which some
/// producers draw with the dots as separate shapes that carry no text.
fn readable(text: &str) -> bool {
    !text.chars().any(|c| matches!(c, '\u{066E}' | '\u{066F}' | '\u{06A1}' | '\u{06BA}'))
}

/// The block's text as the UI shows and edits it (compatibility forms
/// folded, NFKC), each character with the style it was drawn in.
fn styled_chars(block: &Block) -> Vec<(char, RunStyle)> {
    block
        .runs
        .iter()
        .flat_map(|run| block.text[run.range.clone()].nfkc().map(move |c| (c, run.style)))
        .collect()
}

/// Runs of `chars`: byte ranges of their text, consecutive equal styles merged.
fn style_runs(chars: &[(char, RunStyle)]) -> Vec<(Range<usize>, RunStyle)> {
    let mut runs: Vec<(Range<usize>, RunStyle)> = Vec::new();
    let mut at = 0;
    for &(c, style) in chars {
        let end = at + c.len_utf8();
        match runs.last_mut() {
            Some((range, last)) if *last == style => range.end = end,
            _ => runs.push((at..end, style)),
        }
        at = end;
    }
    runs
}

/// Longest text whose kept characters are matched one by one; past it the
/// middle is taken as replaced whole (a quadratic table stays small).
const RESTYLE_CELLS: usize = 4_000_000;

/// `new` (the edited text) in the styles of `old` (the text as it was):
///
/// - a character the edit kept keeps its style;
/// - typed text that replaced some takes the first replaced character's
///   style (typing over a bold word stays bold);
/// - other typed text takes the style of the character before it, or the
///   one after it at the start.
///
/// What was kept is found by a longest common subsequence of characters.
fn restyle(old: &[(char, RunStyle)], new: &str) -> Vec<(char, RunStyle)> {
    let new: Vec<char> = new.chars().collect();
    let Some(&(_, first_style)) = old.first() else {
        return Vec::new();
    };
    let prefix = old.iter().zip(&new).take_while(|((a, _), b)| a == *b).count();
    let suffix =
        old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|((a, _), b)| a == *b).count();
    let (old_mid, new_mid) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);

    // Edit script over the middle: Some(i) keeps old_mid[i]; None is typed.
    // Deletions are noted per gap, as the style typed text there takes.
    let (n, m) = (old_mid.len(), new_mid.len());
    let mut kept: Vec<Option<usize>> = vec![None; m];
    let mut replaced_at: Vec<Option<RunStyle>> = vec![None; m + 1];
    if n > 0 && m > 0 && (n + 1) * (m + 1) <= RESTYLE_CELLS {
        let width = m + 1;
        let mut table = vec![0u16; (n + 1) * width];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i * width + j] = if old_mid[i].0 == new_mid[j] {
                    table[(i + 1) * width + j + 1] + 1
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if old_mid[i].0 == new_mid[j] {
                kept[j] = Some(i);
                i += 1;
                j += 1;
            } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
                // old_mid[i] was deleted, in the gap before new_mid[j].
                replaced_at[j].get_or_insert(old_mid[i].1);
                i += 1;
            } else {
                j += 1;
            }
        }
        if i < n {
            replaced_at[j].get_or_insert(old_mid[i].1);
        }
    } else if n > 0 {
        replaced_at[0] = Some(old_mid[0].1);
    }

    let mut out: Vec<(char, RunStyle)> = old[..prefix].to_vec();
    // The style of the next kept character, for typed text at the very start.
    let next_kept =
        kept.iter().flatten().next().map(|&i| old_mid[i].1).or(old.get(old.len() - suffix).map(|&(_, s)| s));
    let mut gap_style: Option<RunStyle> = None;
    for (j, &c) in new_mid.iter().enumerate() {
        if let Some(style) = replaced_at[j] {
            gap_style.get_or_insert(style);
        }
        let style = match kept[j] {
            Some(i) => {
                gap_style = None;
                old_mid[i].1
            }
            None => gap_style.or_else(|| out.last().map(|&(_, s)| s)).or(next_kept).unwrap_or(first_style),
        };
        out.push((c, style));
    }
    out.extend_from_slice(&old[old.len() - suffix..]);
    out
}

/// One change to a content stream's bytes.
struct Splice {
    stream: usize,
    range: Range<usize>,
    with: Vec<u8>,
}

/// The bytes to append to `bytes` to apply `edit`: an incremental update.
pub fn edit_block_update(
    bytes: &[u8],
    view: &ViewTransform,
    page_width: f32,
    edit: &BlockEdit,
) -> Result<(Vec<u8>, EditedBlock)> {
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let leaf = tree
        .leaves
        .get(edit.page)
        .ok_or(Error::PageOutOfRange { index: edit.page, count: tree.leaves.len() })?;
    let read = read_page(&file, &tree, leaf, view)?;
    let block = read
        .blocks
        .get(edit.index)
        .filter(|b| fingerprint(b, &read.shown) == edit.fingerprint)
        .ok_or(Error::InvalidEdit("the page's text changed since it was read"))?;
    if block.shared || block.shown.is_empty() || !readable(&block.text) {
        return Err(Error::InvalidEdit("this text cannot be edited"));
    }
    if edit.text.encode_utf16().count() > crate::text_edit::MAX_TEXT {
        return Err(Error::InvalidEdit("text is edited at most 20,000 characters at a time"));
    }
    let text = edit.text.replace("\r\n", "\n").replace('\r', "\n");
    // The new text in the old one's styles (ADR 0024).
    let runs = style_runs(&restyle(&styled_chars(block), &text));

    // ---- Removing: the block's operators.
    let removed: std::collections::HashSet<usize> = block.shown.iter().copied().collect();
    let mut splices: Vec<Splice> = Vec::new();
    // A text object that held only this paragraph, in a font this app wrote,
    // goes whole: `BT … ET` sets its own state, so nothing after it needs it,
    // and its font can then drop out of the page.
    let mut whole: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for &i in &block.shown {
        let s = &read.shown[i];
        let ours = is_ours(&s.font_name);
        let alone = read.shown.iter().enumerate().all(|(j, t)| t.object != s.object || removed.contains(&j));
        if ours && alone && whole.insert(s.object) {
            let ops = &read.streams[s.object_start.0].ops;
            let begin = ops.iter().position(|op| op.name == b"BT" && op.range.start == s.object_start.1);
            let end = begin.and_then(|b| ops[b..].iter().find(|op| op.name == b"ET"));
            if let (Some(b), Some(end)) = (begin, end) {
                splices.push(Splice {
                    stream: s.object_start.0,
                    range: ops[b].range.start..end.range.end,
                    with: Vec::new(),
                });
            } else {
                whole.remove(&s.object);
            }
        }
    }
    for &i in &block.shown {
        let s = &read.shown[i];
        if whole.contains(&s.object) {
            continue;
        }
        let needed = read
            .shown
            .iter()
            .enumerate()
            .any(|(j, t)| j > i && t.object == s.object && !removed.contains(&j));
        let advance = if s.size.abs() > 1e-9 && s.tz.abs() > 1e-9 {
            -s.advance / (s.size * s.tz) * 1000.0
        } else {
            0.0
        };
        let spacer = format!("[{}] TJ", num(advance as f32));
        let with = match s.kind {
            ShowKind::Tj | ShowKind::TJ => {
                if needed {
                    spacer
                } else {
                    String::new()
                }
            }
            ShowKind::NextLine => {
                if needed {
                    format!("T* {spacer}")
                } else {
                    "T*".into()
                }
            }
            ShowKind::NextLineSpaced => {
                let (aw, ac) = s.word_char;
                let head = format!("{} Tw {} Tc T*", num(aw as f32), num(ac as f32));
                if needed { format!("{head} {spacer}") } else { head }
            }
        };
        splices.push(Splice { stream: s.stream, range: s.range.clone(), with: with.into_bytes() });
    }

    // ---- Where the new text goes: before the first removed operator's `BT`.
    let anchor = block
        .shown
        .iter()
        .map(|&i| &read.shown[i])
        .min_by_key(|s| s.object_start)
        .ok_or(Error::InvalidEdit("this text cannot be edited"))?;
    let inverse =
        anchor.ctm.inverse().ok_or(Error::EditUnsupported("the page's content scales to nothing here"))?;

    // The box.
    let font_info = read.fonts.get(block.font).and_then(|(_, f)| f.clone());
    let (box_x0, box_x1) = box_of(&read.blocks, &read.obstacles, edit.index, page_width);
    let width = (box_x1 - box_x0).max(1.0);
    let first_baseline = block.lines[0].baseline;
    let align = match block.align {
        BlockAlign::Left => Align::Left,
        BlockAlign::Right => Align::Right,
        BlockAlign::Center => Align::Center,
        BlockAlign::Justify => Align::Justify,
    };

    let mut resources = tree
        .attribute(leaf, b"Resources")
        .and_then(|v| file.resolve(&v))
        .and_then(|v| v.as_dict().cloned())
        .unwrap_or_default();
    let mut font_names = resources
        .get(b"Font")
        .and_then(|v| file.resolve(v))
        .and_then(|v| v.as_dict().cloned())
        .unwrap_or_default();

    let mut update = Update::new(bytes);
    let mut inserted: Option<Vec<u8>> = None;
    let mut substituted = None;
    let mut lines_after = 0;

    if !text.trim().is_empty() {
        // Each run in its own font, when every one can set its text.
        let reuse = edit.family.is_none()
            && runs.iter().all(|(range, style)| {
                read.fonts
                    .get(style.font)
                    .and_then(|(_, f)| f.as_deref())
                    .is_some_and(|f| reusable_for(f, &text[range.clone()]))
            });
        let (local, matrix_top, lines) = if reuse {
            let pitch = block.pitch.unwrap_or(block.size * 1.2);
            let (content, lines) = reuse_content(&text, &runs, &read.fonts, width, pitch, align);
            (content, first_baseline, lines)
        } else {
            let set_in = substitutes(&text, &runs, &read.fonts, block.font, edit.family)?;
            let faces: Vec<FontFace> = set_in.fonts.iter().map(|f| f.face.clone()).collect();
            let style = ParagraphStyle {
                direction: if block.rtl { BaseDirection::Rtl } else { BaseDirection::Ltr },
                align,
                width: Some(width),
                line_height: block.pitch.map_or(LineHeight::Normal, LineHeight::Points),
            };
            let laid = layout(&text, &set_in.spans, &faces, &style).map_err(|e| Error::Typeset(e.code()))?;
            let prefix = free_prefix(&font_names);
            let drawing =
                draw_painted_text(&text, &laid, &set_in.fonts, (0.0, 0.0), &set_in.paints, &prefix)?;
            for font in &drawing.fonts {
                let number = write_font(&mut update, None, &mut next_number, font)?;
                set(&mut font_names, font.resource.as_bytes(), Object::Ref(number, 0));
            }
            let top = first_baseline - laid.lines.first().map_or(0.0, |l| l.baseline);
            substituted = Some(Substitution {
                from: font_info.as_ref().map(|f| f.name.clone()).unwrap_or_default(),
                to: set_in.family,
                font: set_in.font,
            });
            (drawing.content, top, laid.lines.len())
        };
        lines_after = lines;
        let [pa, pb, pc, pd, pe, pf] = placement(view, box_x0, matrix_top)?;
        let combined = Matrix([
            f64::from(pa),
            f64::from(pb),
            f64::from(pc),
            f64::from(pd),
            f64::from(pe),
            f64::from(pf),
        ])
        .then(inverse);
        let [a, b2, c, d, e, f] = combined.0.map(|v| v as f32);
        let mut content = format!(
            "q\n{} {} {} {} {} {} cm\n/MirsamText BMC\n",
            num(a),
            num(b2),
            num(c),
            num(d),
            num(e),
            num(f)
        )
        .into_bytes();
        content.extend_from_slice(&local);
        content.extend_from_slice(b"EMC\nQ\n");
        inserted = Some(content);
        splices.push(Splice {
            stream: anchor.object_start.0,
            range: anchor.object_start.1..anchor.object_start.1,
            with: inserted.clone().unwrap_or_default(),
        });
    }

    // ---- Apply the splices stream by stream.
    let mut changed: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut touched: Vec<usize> = splices.iter().map(|s| s.stream).collect();
    touched.sort_unstable();
    touched.dedup();
    for stream in touched {
        let mut mine: Vec<&Splice> = splices.iter().filter(|s| s.stream == stream).collect();
        mine.sort_by_key(|s| (s.range.start, s.range.end));
        if mine.windows(2).any(|w| w[0].range.end > w[1].range.start) {
            return Err(Error::EditUnsupported("the page's text operators overlap"));
        }
        let data = &read.streams[stream].data;
        let mut out = Vec::with_capacity(data.len());
        let mut at = 0;
        for splice in mine {
            out.extend_from_slice(&data[at..splice.range.start]);
            out.extend_from_slice(&splice.with);
            at = splice.range.end;
        }
        out.extend_from_slice(&data[at..]);
        changed.insert(stream, out);
    }

    // ---- Our own fonts no stream uses any more drop out of the page.
    let mut used: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    for (i, stream) in read.streams.iter().enumerate() {
        let ops = match changed.get(&i) {
            Some(data) => {
                tokenize(data).ok_or(Error::EditUnsupported("the edited content cannot be read"))?
            }
            None => stream.ops.clone(),
        };
        for op in ops.iter().filter(|op| op.name == b"Tf") {
            if let Some(Operand::Name(name)) = op.operands.first() {
                used.insert(name.clone());
            }
        }
    }
    font_names.0.retain(|(name, _)| !is_ours(name) || used.contains(name));
    set(&mut resources, b"Font", Object::Dict(font_names));

    // ---- New content streams, then the page.
    let mut items = read.items.clone();
    let mut take = || -> Result<u32> {
        let number = next_number;
        next_number = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let mut order: Vec<_> = changed.into_iter().collect();
    order.sort_by_key(|(i, _)| *i);
    for (i, data) in order {
        let number = take()?;
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        let dict = Dict(vec![(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()))]);
        update.stream(number, 0, dict, &compressed);
        items[i] = Object::Ref(number, 0);
    }
    let mut page = leaf.dict.clone();
    set(&mut page, b"Contents", Object::Array(items));
    set(&mut page, b"Resources", Object::Dict(resources));
    update.object(leaf.number, leaf.generation, &Object::Dict(page));
    let _ = inserted;
    let bytes = update.finish(form, file.trailer.clone(), section, next_number);
    Ok((bytes, EditedBlock { substituted, lines: lines_after }))
}

/// A resource name this app wrote its own fonts under (`MT0`, `MT2x1`, …).
fn is_ours(name: &[u8]) -> bool {
    let Some(rest) = name.strip_prefix(b"MT") else { return false };
    let rest = match rest.iter().position(|b| *b == b'x') {
        Some(x) if x > 0 && rest[..x].iter().all(u8::is_ascii_digit) => &rest[x + 1..],
        Some(_) => return false,
        None => rest,
    };
    !rest.is_empty() && rest.iter().all(u8::is_ascii_digit)
}

/// The box a paragraph is set in: its own width, except that a single line
/// may grow to the page's margin (or the next text) on the side it reads away from.
fn box_of(blocks: &[Block], obstacles: &[[f32; 4]], index: usize, page_width: f32) -> (f32, f32) {
    let block = &blocks[index];
    let (mut x0, mut x1) = (block.x0, block.x1 + 0.5);
    if block.lines.len() == 1 && block.align != BlockAlign::Center {
        // Text beside it on the same line: [x0, x1, top, bottom].
        let beside: Vec<[f32; 4]> = blocks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, o)| [o.x0, o.x1, o.top, o.bottom])
            .chain(obstacles.iter().copied())
            .filter(|o| o[2] < block.bottom && o[3] > block.top)
            .collect();
        if block.align == BlockAlign::Left {
            let limit = beside
                .iter()
                .filter(|o| o[0] >= block.x1 - 0.5)
                .map(|o| o[0] - 6.0)
                .fold(page_width - MARGIN, f32::min);
            x1 = x1.max(limit);
        } else {
            let limit =
                beside.iter().filter(|o| o[1] <= block.x0 + 0.5).map(|o| o[1] + 6.0).fold(MARGIN, f32::max);
            x0 = x0.min(limit);
        }
    }
    (x0, x1)
}

/// The bundled face closest to the paragraph's own font for `text`, then
/// the faces that fill its gaps, and the family it belongs with. The text's
/// coverage decides first; then the name, the serif flag, and the widths of
/// the font's letters that do not join (`typeset::match_font`), so a
/// substitute keeps the lines where they broke as far as it can.
fn closest_fonts(font: Option<&FontInfo>, text: &str) -> Result<(Vec<NamedFont>, FontFamily)> {
    let mut request = FontRequest::from_pdf(font.map_or("", |f| f.name.as_str()), None, None, None, None);
    if let Some(font) = font {
        // A set serif flag says something; a clear one, from many producers, does not.
        request.serif = font.serif.then_some(true);
        request.italic = Some(font.italic);
        if font.bold {
            request.weight = Some(700.0);
        }
        request.widths = font.letter_widths();
    }
    let candidates = candidates()?;
    let faces: Vec<FontFace> = candidates.iter().map(|c| c.face.clone()).collect();
    let chosen = match_font(&request, text, &faces)
        .and_then(|found| candidates.get(found.index).cloned())
        .ok_or(Error::Typeset("no font to set the text in"))?;
    let family = family_of(chosen.name);
    let mut fonts = vec![chosen];
    for fallback in fallbacks()? {
        if fonts.iter().all(|f| f.name != fallback.name) {
            fonts.push(fallback);
        }
    }
    Ok((fonts, family))
}

/// The paragraph's own font can set `text`: Latin characters it has codes for.
fn reusable_for(font: &FontInfo, text: &str) -> bool {
    font.usable()
        && !font.vertical
        && text.chars().all(|c| {
            c == '\n' || (!is_complex(c) && font.encode(if c == '\u{a0}' { ' ' } else { c }).is_some())
        })
        && font.encode(' ').is_some()
}

/// Scripts whose letters change shape or order with their neighbours.
fn is_complex(c: char) -> bool {
    matches!(c, '\u{0590}'..='\u{08FF}' | '\u{0900}'..='\u{0DFF}' | '\u{FB1D}'..='\u{FEFF}' | '\u{200F}' | '\u{200E}')
}

/// A name for a content stream (`/F1`), escaped.
fn pdf_name(name: &[u8]) -> String {
    let mut out = String::from("/");
    for &b in name {
        if b.is_ascii_graphic()
            && !matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' | b'#')
        {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("#{b:02X}"));
        }
    }
    out
}

/// Lays `text` out in its runs' own fonts (each can set its run: Latin
/// characters it has codes for) and draws it: words wrapped greedily by the
/// fonts' widths, aligned or justified with `TJ` adjustments at the spaces.
/// The font, size and colour change wherever a run does. Lines are `pitch`
/// apart, the first baseline at y = 0. Returns the content and its line count.
fn reuse_content(
    text: &str,
    runs: &[(Range<usize>, RunStyle)],
    fonts: &[(Vec<u8>, Option<Rc<FontInfo>>)],
    width: f32,
    pitch: f32,
    align: Align,
) -> (Vec<u8>, usize) {
    // Each character with the run it is in.
    let mut chars: Vec<(char, usize)> = Vec::with_capacity(text.len());
    for (index, (range, _)) in runs.iter().enumerate() {
        chars.extend(text[range.clone()].chars().map(|c| (if c == '\u{a0}' { ' ' } else { c }, index)));
    }
    let font = |run: usize| fonts.get(runs[run].1.font).and_then(|(_, f)| f.as_deref());
    let code = |c: char, run: usize| -> Option<Vec<u8>> { font(run).and_then(|f| f.encode(c)) };
    let advance = |c: char, run: usize| -> f32 {
        let (Some(f), Some(bytes)) = (font(run), code(c, run)) else { return 0.0 };
        let code = bytes.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b));
        f.width(code) as f32 * runs[run].1.size / 1000.0
    };

    // A line: words, each with the space before it (its run) after the first.
    struct Word {
        space: Option<usize>,
        chars: Vec<(char, usize)>,
    }
    let mut lines: Vec<(Vec<Word>, bool)> = Vec::new();
    for paragraph in chars.split(|&(c, _)| c == '\n') {
        // Words, each with the first space before it (spaces in a row are one).
        let mut words: Vec<Word> = Vec::new();
        let mut space: Option<usize> = None;
        let mut word: Vec<(char, usize)> = Vec::new();
        for &(c, run) in paragraph {
            if c == ' ' {
                if !word.is_empty() {
                    words.push(Word { space: space.take(), chars: std::mem::take(&mut word) });
                }
                space.get_or_insert(run);
            } else {
                word.push((c, run));
            }
        }
        if !word.is_empty() {
            words.push(Word { space: space.take(), chars: word });
        }
        let mut line: Vec<Word> = Vec::new();
        let mut used = 0.0f32;
        for mut word in words {
            let w: f32 = word.chars.iter().map(|&(c, run)| advance(c, run)).sum();
            let gap = word.space.map_or(0.0, |run| advance(' ', run));
            if !line.is_empty() && used + gap + w > width {
                lines.push((std::mem::take(&mut line), false));
                used = 0.0;
            }
            if line.is_empty() {
                word.space = None;
                used = w;
            } else {
                used += gap + w;
            }
            line.push(word);
        }
        lines.push((line, true));
    }

    let mut out = String::from("0 Tc 0 Tw 100 Tz 0 Ts 0 Tr\n");
    for (n, (words, last)) in lines.iter().enumerate() {
        let natural: f32 = words
            .iter()
            .map(|w| {
                w.space.map_or(0.0, |run| advance(' ', run))
                    + w.chars.iter().map(|&(c, r)| advance(c, r)).sum::<f32>()
            })
            .sum();
        let gaps = words.iter().filter(|w| w.space.is_some()).count();
        let (x, extra) = match align {
            Align::Right | Align::End => (width - natural, 0.0),
            Align::Center => ((width - natural) / 2.0, 0.0),
            Align::Justify if !last && gaps > 0 => (0.0, (width - natural) / gaps as f32),
            _ => (0.0, 0.0),
        };
        let y = -(n as f32) * pitch;
        out.push_str(&format!("BT 1 0 0 1 {} {} Tm\n", num(x), num(y)));
        // Pieces of one run go in one `TJ`; a run change sets font and colour.
        let mut current: Option<usize> = None;
        let mut open = false;
        let mut emit = |out: &mut String, run: usize, element: String| {
            if current != Some(run) {
                if open {
                    out.push_str("] TJ\n");
                }
                let style = &runs[run].1;
                let resource = fonts.get(style.font).map_or(&b""[..], |(name, _)| name.as_slice());
                let [r, g, b] = style.color;
                out.push_str(&format!(
                    "{} {} Tf {} {} {} rg [",
                    pdf_name(resource),
                    num(style.size),
                    num(r),
                    num(g),
                    num(b)
                ));
                current = Some(run);
                open = true;
            }
            out.push_str(&element);
        };
        let hex = |c: char, run: usize| -> String {
            code(c, run).unwrap_or_default().iter().map(|b| format!("{b:02X}")).collect()
        };
        for word in words {
            if let Some(run) = word.space {
                let mut element = format!("<{}>", hex(' ', run));
                if extra != 0.0 {
                    element.push_str(&format!(" {} ", num(-extra * 1000.0 / runs[run].1.size)));
                }
                emit(&mut out, run, element);
            }
            for &(c, run) in &word.chars {
                emit(&mut out, run, format!("<{}>", hex(c, run)));
            }
        }
        if open {
            out.push_str("] TJ\n");
        }
        out.push_str("ET\n");
    }
    (out.into_bytes(), lines.len())
}

/// What a substituted paragraph is set in: the faces (each run's original
/// font's closest bundled face, or the chosen family, at the run's weight,
/// with the faces that fill its gaps), a span and a paint per run, and the
/// face and family the paragraph's own font became.
struct Substitute {
    fonts: Vec<NamedFont>,
    spans: Vec<Span>,
    paints: Vec<Paint>,
    family: FontFamily,
    font: String,
}

fn substitutes(
    text: &str,
    runs: &[(Range<usize>, RunStyle)],
    fonts: &[(Vec<u8>, Option<Rc<FontInfo>>)],
    dominant: usize,
    family: Option<FontFamily>,
) -> Result<Substitute> {
    let info = |key: usize| fonts.get(key).and_then(|(_, f)| f.as_deref());
    // Each original font's faces, once.
    let mut chains: HashMap<usize, (Vec<NamedFont>, FontFamily)> = HashMap::new();
    for (_, style) in runs {
        if chains.contains_key(&style.font) {
            continue;
        }
        let its_text: String = runs
            .iter()
            .filter(|(_, s)| s.font == style.font)
            .map(|(range, _)| &text[range.clone()])
            .collect();
        let chain = match family {
            Some(family) => (for_text(family, text)?, family),
            None => closest_fonts(info(style.font), &its_text)?,
        };
        chains.insert(style.font, chain);
    }
    let mut list: Vec<NamedFont> = Vec::new();
    let mut index_of = |font: NamedFont| -> usize {
        match list.iter().position(|f| f.name == font.name) {
            Some(i) => i,
            None => {
                list.push(font);
                list.len() - 1
            }
        }
    };
    let mut spans = Vec::with_capacity(runs.len());
    let mut paints = Vec::with_capacity(runs.len());
    for (range, style) in runs {
        let (chain, _) = &chains[&style.font];
        let bold = info(style.font).is_some_and(|f| f.bold);
        let italic = info(style.font).is_some_and(|f| f.italic);
        let mut indices = Vec::with_capacity(chain.len());
        for font in chain {
            let font = if bold { bold_of(font)? } else { font.clone() };
            indices.push(index_of(if italic { oblique_of(&font) } else { font }));
        }
        let mut span = Span::new(range.clone(), indices[0], style.size);
        span.fallbacks = indices[1..].to_vec();
        spans.push(span);
        paints.push(Paint {
            range: range.clone(),
            color: style.color,
            slant: info(style.font).is_some_and(|f| f.italic),
        });
    }
    let (chain, family) = chains
        .get(&dominant)
        .or_else(|| runs.first().and_then(|(_, s)| chains.get(&s.font)))
        .cloned()
        .ok_or(Error::InvalidEdit("an empty paragraph"))?;
    Ok(Substitute {
        fonts: list,
        spans,
        paints,
        family,
        font: chain.first().map(|f| f.name.to_string()).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page font standing for `face`: its Latin letters and digits at the
    /// face's own advances, under a name no bundled face has.
    fn widths_of(face: &str, serif: bool) -> FontInfo {
        let fonts = candidates().expect("bundled");
        let font = fonts.iter().find(|f| f.name == face).expect("bundled face");
        let chars: Vec<(char, f64)> = "0123456789aehinoprstPS "
            .chars()
            .map(|c| (c, f64::from(font.face.advance(c).unwrap_or(0.0))))
            .collect();
        FontInfo::simple_for_test("ABCDEF+UnknownDocumentFont", serif, &chars)
    }

    #[test]
    fn a_substitute_follows_the_widths_the_page_was_drawn_with() {
        // Same widths as Noto Naskh Arabic, though not its name: chosen over
        // Amiri, the default Naskh, and over the Sans faces.
        let (fonts, family) =
            closest_fonts(Some(&widths_of("NotoNaskhArabic-Regular", false)), "نص Pro 15").expect("fonts");
        assert_eq!(fonts[0].name, "NotoNaskhArabic-Regular");
        assert_eq!(family, FontFamily::Naskh);
        let (fonts, family) =
            closest_fonts(Some(&widths_of("NotoSansArabic-Regular", false)), "نص Pro 15").expect("fonts");
        assert_eq!((fonts[0].name, family), ("NotoSansArabic-Regular", FontFamily::Sans));
        // Plex Sans Arabic's Latin is Plex Sans's: text with Arabic takes the face that has it.
        let (fonts, _) = closest_fonts(Some(&widths_of("IBMPlexSans", false)), "نص Pro").expect("fonts");
        assert_eq!(fonts[0].name, "IBMPlexSansArabic");
        // The fallbacks follow, once each.
        assert_eq!(fonts.iter().filter(|f| f.name == "IBMPlexSansArabic").count(), 1);
        assert!(fonts.iter().any(|f| f.name == "IBMPlexSans"));
    }

    #[test]
    fn with_no_widths_a_substitute_goes_by_name_then_class() {
        let named = FontInfo::simple_for_test("Amiri-Regular", false, &[]);
        assert_eq!(closest_fonts(Some(&named), "نص").expect("fonts").0[0].name, "Amiri-Regular");
        // A serif with an unknown name: a Naskh face.
        let serif = FontInfo::simple_for_test("SomeSerif", true, &[]);
        assert_eq!(closest_fonts(Some(&serif), "نص").expect("fonts").1, FontFamily::Naskh);
        // Nothing known: still a face that has the text.
        let (fonts, _) = closest_fonts(None, "نص").expect("fonts");
        assert!(fonts[0].face.covers('ن'));
    }

    /// Text in two styles: `B` marks bold (font 1) characters of `pattern`.
    fn styled(text: &str, pattern: &str) -> Vec<(char, RunStyle)> {
        let style = |font| RunStyle { font, size: 12.0, color: [0.0; 3] };
        text.chars().zip(pattern.chars()).map(|(c, p)| (c, style(usize::from(p == 'B')))).collect()
    }

    /// The pattern of `chars`: `B` where bold, `.` elsewhere.
    fn pattern(chars: &[(char, RunStyle)]) -> String {
        chars.iter().map(|(_, s)| if s.font == 1 { 'B' } else { '.' }).collect()
    }

    #[test]
    fn an_edit_keeps_each_characters_style_and_typed_text_takes_its_neighbours() {
        // `dots(n)` plain characters, `bold(n)` bold ones.
        let dots = |n: usize| ".".repeat(n);
        let bold = |n: usize| "B".repeat(n);
        // "The rent is due monthly": "due" (12..15) bold.
        let old = styled("The rent is due monthly", &format!("{}{}{}", dots(12), bold(3), dots(8)));
        // A word typed after the bold one takes the style before it: the space's.
        assert_eq!(
            pattern(&restyle(&old, "The rent is due now monthly")),
            format!("{}{}{}", dots(12), bold(3), dots(12))
        );
        // Typed onto the end of the bold word: bold.
        assert_eq!(
            pattern(&restyle(&old, "The rent is dues monthly")),
            format!("{}{}{}", dots(12), bold(4), dots(8))
        );
        // Typed over the bold word: bold, whatever letters it shares with it.
        assert_eq!(
            pattern(&restyle(&old, "The rent is payable monthly")),
            format!("{}{}{}", dots(12), bold(7), dots(8))
        );
        // Typed at the very start: the style of what follows.
        let lead = styled("due today", &format!("{}{}", bold(3), dots(6)));
        assert_eq!(pattern(&restyle(&lead, "Rent due today")), format!("{}{}", bold(8), dots(6)));
        // Deleting keeps the rest as it was; emptying gives nothing.
        assert_eq!(pattern(&restyle(&old, "The rent is monthly")), dots(19));
        assert_eq!(pattern(&restyle(&old, "rent due")), format!("{}{}", dots(5), bold(3)));
        assert!(restyle(&old, "").is_empty());
        // Arabic, by characters: "يدفع الإيجار شهريا" with "الإيجار" bold, a word added after it.
        let arabic = styled("يدفع الإيجار شهريا", &format!("{}{}{}", dots(5), bold(7), dots(6)));
        assert_eq!(
            pattern(&restyle(&arabic, "يدفع الإيجار كاملا شهريا")),
            format!("{}{}{}", dots(5), bold(7), dots(12))
        );
    }

    #[test]
    fn unreadable_runs_join_into_one_area_per_line() {
        // Three glyphs on one line, one on the next, one rotated beside them.
        let obstacles = [
            [10.0, 16.0, 0.0, 12.0],
            [17.0, 23.0, 0.0, 12.0],
            [24.0, 30.0, 1.0, 12.0],
            [10.0, 16.0, 20.0, 32.0],
            [31.0, 33.0, 0.0, 12.0],
        ];
        let reasons = [Refusal::Font, Refusal::Font, Refusal::Font, Refusal::Font, Refusal::Rotated];
        let areas = refused_areas(&obstacles, &reasons);
        assert_eq!(
            areas,
            vec![
                ([10.0, 30.0, 0.0, 12.0], Refusal::Font),
                ([10.0, 16.0, 20.0, 32.0], Refusal::Font),
                ([31.0, 33.0, 0.0, 12.0], Refusal::Rotated),
            ]
        );
    }

    #[test]
    fn our_font_names_are_told_from_the_pages() {
        for ours in ["MT0", "MT12", "MT2x0", "MT3x14"] {
            assert!(is_ours(ours.as_bytes()), "{ours}");
        }
        for theirs in ["F1", "MT", "MTx", "MTextra", "MT2y0", "Arial", "xMT0"] {
            assert!(!is_ours(theirs.as_bytes()), "{theirs}");
        }
    }

    #[test]
    fn names_are_escaped_for_content_streams() {
        assert_eq!(pdf_name(b"F1"), "/F1");
        assert_eq!(pdf_name(b"A B/c"), "/A#20B#2Fc");
    }
}
