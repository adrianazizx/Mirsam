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

use crate::blocks::{Atom, Block, BlockAlign, analyze};
use crate::edit::{Leaf, PageTree, Start, set};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, for_text};
use crate::pdf::content::{Matrix, Op, Operand, ShowKind, Shown, interpret, tokenize};
use crate::pdf::fontinfo::{FontInfo, stream_data};
use crate::pdf::text::{draw_text, write_font};
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};
use crate::text_edit::{free_prefix, num, page_contents, placement};
use crate::typeset::{Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, layout};
use crate::view::ViewTransform;

/// Beyond a single line's own width, it may grow to the page's margin.
const MARGIN: f32 = 36.0;

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
    /// More than one font, size or colour was found; editing sets it in one.
    pub flattened: bool,
    pub editable: bool,
    /// Why it is not editable.
    pub reason: Option<&'static str>,
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

    let (atoms, obstacles) = atoms_of(&shown, &fonts, view);
    let mut blocks = analyze(atoms);
    // Eligible operators only: the block's text is rebuilt from these.
    for block in &mut blocks {
        block.shown.retain(|&i| shown.get(i).is_some());
    }
    Ok(PageRead { items, streams, shown, fonts, blocks, obstacles })
}

/// Whether a text-showing operator can be read and written back, and where
/// its glyphs land in the page as displayed.
fn atoms_of(
    shown: &[Shown],
    fonts: &[(Vec<u8>, Option<Rc<FontInfo>>)],
    view: &ViewTransform,
) -> (Vec<Atom>, Vec<[f32; 4]>) {
    let view = Matrix(view.matrix());
    let mut atoms = Vec::new();
    let mut obstacles = Vec::new();
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
    (atoms, obstacles)
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
    Ok(read
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let font = read.fonts.get(block.font).and_then(|(_, f)| f.clone());
            let reason = if block.shared {
                Some("shares its text with other text")
            } else if block.shown.is_empty() || !readable(&block.text) {
                Some("cannot be read")
            } else {
                None
            };
            TextBlock {
                index,
                fingerprint: fingerprint(block, &read.shown),
                x: block.x0,
                y: block.top,
                width: block.x1 - block.x0,
                height: block.bottom - block.top,
                text: block.text.nfkc().collect(),
                rtl: block.rtl,
                align: block.align,
                size: block.size,
                color: block.color,
                lines: block.lines.len(),
                font: font.as_ref().map(|f| f.name.clone()).unwrap_or_default(),
                font_reusable: font.is_some_and(|f| f.usable() && !f.vertical),
                flattened: block.mixed,
                editable: reason.is_none(),
                reason,
            }
        })
        .collect())
}

/// Text a font's codes gave up exactly: not dotless letters, which some
/// producers draw with the dots as separate shapes that carry no text.
fn readable(text: &str) -> bool {
    !text.chars().any(|c| matches!(c, '\u{066E}' | '\u{066F}' | '\u{06A1}' | '\u{06BA}'))
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
    let [r, g, b] = block.color;

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
        let reuse = font_info.as_ref().filter(|f| edit.family.is_none() && reusable_for(f, &text)).cloned();
        let (local, matrix_top, lines) = if let Some(font) = reuse {
            let pitch = block.pitch.unwrap_or(block.size * 1.2);
            let (content, lines) =
                reuse_content(&text, &font, &anchor.font_name, block.size, [r, g, b], width, pitch, align);
            (content, first_baseline, lines)
        } else {
            let family = edit.family.unwrap_or_else(|| default_family(font_info.as_deref()));
            let fonts = for_text(family, &text)?;
            let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
            let style = ParagraphStyle {
                direction: if block.rtl { BaseDirection::Rtl } else { BaseDirection::Ltr },
                align,
                width: Some(width),
                line_height: block.pitch.map_or(LineHeight::Normal, LineHeight::Points),
            };
            let laid = layout(&text, &[Span::new(0..text.len(), 0, block.size)], &faces, &style)
                .map_err(|e| Error::Typeset(e.code()))?;
            let prefix = free_prefix(&font_names);
            let drawing = draw_text(&text, &laid, &fonts, (0.0, 0.0), [r, g, b], &prefix)?;
            for font in &drawing.fonts {
                let number = write_font(&mut update, None, &mut next_number, font)?;
                set(&mut font_names, font.resource.as_bytes(), Object::Ref(number, 0));
            }
            let top = first_baseline - laid.lines.first().map_or(0.0, |l| l.baseline);
            substituted = Some(Substitution {
                from: font_info.as_ref().map(|f| f.name.clone()).unwrap_or_default(),
                to: family,
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

fn default_family(font: Option<&FontInfo>) -> FontFamily {
    if font.is_some_and(|f| f.serif) { FontFamily::Naskh } else { FontFamily::Sans }
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

/// Text set in a font the page already has: greedy lines by the font's own
/// widths, aligned in a box `width` wide. In box space: origin at the first
/// baseline's start, y up. Returns the operators and the number of lines.
#[allow(clippy::too_many_arguments)]
fn reuse_content(
    text: &str,
    font: &FontInfo,
    resource: &[u8],
    size: f32,
    color: [f32; 3],
    width: f32,
    pitch: f32,
    align: Align,
) -> (Vec<u8>, usize) {
    let word_width = |word: &str| -> f32 {
        word.chars()
            .filter_map(|c| font.encode(c))
            .map(|bytes| {
                let code = bytes.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b));
                font.width(code) as f32 * size / 1000.0
            })
            .sum()
    };
    let space = word_width(" ");
    // Lines of words; a hard break ends a paragraph.
    let mut lines: Vec<(Vec<&str>, bool)> = Vec::new();
    for paragraph in text.split('\n') {
        let mut current: Vec<&str> = Vec::new();
        let mut used = 0.0f32;
        for word in paragraph.split(' ').filter(|w| !w.is_empty()) {
            let w = word_width(word);
            let extra = if current.is_empty() { w } else { space + w };
            if !current.is_empty() && used + extra > width {
                lines.push((std::mem::take(&mut current), false));
                used = 0.0;
            }
            used += if current.is_empty() { w } else { space + w };
            current.push(word);
        }
        lines.push((current, true));
    }
    let mut out = String::new();
    let [r, g, b] = color;
    out.push_str(&format!("{} {} {} rg 0 Tc 0 Tw 100 Tz 0 Ts 0 Tr\n", num(r), num(g), num(b)));
    let hex = |text: &str| -> String {
        let mut s = String::new();
        for c in text.chars() {
            for byte in font.encode(c).unwrap_or_default() {
                s.push_str(&format!("{byte:02X}"));
            }
        }
        s
    };
    for (n, (words, last)) in lines.iter().enumerate() {
        let natural: f32 =
            words.iter().map(|w| word_width(w)).sum::<f32>() + space * words.len().saturating_sub(1) as f32;
        let gaps = words.len().saturating_sub(1);
        let (x, extra) = match align {
            Align::Right | Align::End => (width - natural, 0.0),
            Align::Center => ((width - natural) / 2.0, 0.0),
            Align::Justify if !last && gaps > 0 => (0.0, (width - natural) / gaps as f32),
            _ => (0.0, 0.0),
        };
        let y = -(n as f32) * pitch;
        out.push_str(&format!(
            "BT {} {} Tf 1 0 0 1 {} {} Tm [",
            pdf_name(resource),
            num(size),
            num(x),
            num(y)
        ));
        for (i, word) in words.iter().enumerate() {
            if i > 0 {
                out.push_str(&format!("<{}>", hex(" ")));
                if extra != 0.0 {
                    out.push_str(&format!(" {} ", num(-extra * 1000.0 / size)));
                }
            }
            out.push_str(&format!("<{}>", hex(word)));
        }
        out.push_str("] TJ ET\n");
    }
    (out.into_bytes(), lines.len())
}

#[cfg(test)]
mod tests {
    use super::*;

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
