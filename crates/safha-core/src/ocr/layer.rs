//! Writing OCR text into a page: an invisible text layer over the image (spec §8
//! "Requirements", ADR 0035).
//!
//! The layer is written by our own content-stream writer as an incremental
//! update (ADR 0005's amendment, ADR 0014); PDFium regenerates nothing. Each page
//! gets one content stream of its own, marked `/MirsamOCR /Draft` or `/Final` in
//! its dictionary, appended after the page's own content (wrapped once in
//! `q … Q`, as stamps and added text do). A later layer for the same page takes
//! the earlier one's place in `/Contents`, so a quality result replaces the draft
//! and a re-run replaces both. The original image is never touched.
//!
//! - **Font:** [`super::font`]: codes are UTF-16 code units, every glyph empty
//!   and one advance wide, `/ToUnicode` the identity.
//! - **Words:** each word is stretched to its box with horizontal scaling (`Tz`),
//!   the box spanning the font's ascent and descent, so a reader's character
//!   boxes are where the word is in the image. Text render mode 3: invisible.
//! - **Order** (measured in ADR 0035): see [`Order`].

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use unicode_bidi::{BidiInfo, Level};

use super::font;
use crate::edit::{PageTree, Start, contains_string, set};
use crate::error::{Error, Result};
use crate::pdf::crypt::Crypt;
use crate::pdf::write::Update;
use crate::pdf::{Dict, File, Object};
use crate::text_edit::{WRAP_KEY, num, page_contents};

/// The key in an OCR layer's stream dictionary; its value is the layer's
/// [`LayerQuality`] as a name.
pub const LAYER_KEY: &[u8] = b"MirsamOCR";

/// Whether a page's text is the quick draft or the quality pipeline's result
/// (spec §8: pages are marked "draft" until the quality pass replaces them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerQuality {
    Draft,
    Final,
}

impl LayerQuality {
    fn name(self) -> &'static [u8] {
        match self {
            LayerQuality::Draft => b"Draft",
            LayerQuality::Final => b"Final",
        }
    }

    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"Draft" => Some(LayerQuality::Draft),
            b"Final" => Some(LayerQuality::Final),
            _ => None,
        }
    }
}

/// A word of the layer: its text in logical order, and its box on the displayed
/// page in points (origin top-left, y down).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerWord {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// A text line: its words in reading (logical) order, and its direction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerLine {
    pub words: Vec<LayerWord>,
    pub rtl: bool,
}

/// One page's text layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageLayer {
    pub page: usize,
    pub quality: LayerQuality,
    /// In reading order.
    pub lines: Vec<LayerLine>,
    /// The `cm` matrix from the displayed page as the words were measured on it
    /// (origin top-left, x right, **y up**, in points) to the page's own space.
    /// Taken when the page was rendered for OCR, so a page rotated since still
    /// gets its words where the image has them.
    pub placement: [f32; 6],
}

/// How the words of a line go into the content stream. Every variant writes
/// each word's characters as one-advance glyphs across the word's box; they
/// differ in which order readers meet the characters. ADR 0035 measures them
/// in PDFium (Chrome), PDFKit (Preview), Poppler and this app.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// One text object per line; its words left to right as drawn, each word's
    /// characters in the order they are drawn (UAX #9 visual order in the
    /// line's direction), spaces in the gaps between them. What a word
    /// processor writes.
    LineVisual,
    /// [`Order::LineVisual`], except that a run of left-to-right words inside a
    /// right-to-left line is one text object holding its words last to first,
    /// each still at its own box: PDFium reverses the words of such runs.
    LineVisualRuns,
    /// Words in reading order; each word's characters in visual order.
    WordVisual,
    /// Words in reading order; characters in logical order, a right-to-left
    /// word advancing leftwards from its right edge (a mirrored text matrix).
    WordLogicalMirrored,
    /// Words in reading order; characters in logical order, advancing rightwards.
    WordLogical,
}

/// The order the layer is written in (ADR 0035).
pub const ORDER: Order = Order::LineVisual;

/// The update that writes `layers` into the document `bytes` (one layer per
/// page at most), replacing any OCR layer those pages already have.
pub(crate) fn text_layer_update(
    bytes: &[u8],
    password: Option<&str>,
    layers: &[PageLayer],
) -> Result<Vec<u8>> {
    text_layer_update_with(bytes, password, layers, ORDER)
}

#[doc(hidden)]
pub fn text_layer_update_with(
    bytes: &[u8],
    password: Option<&str>,
    layers: &[PageLayer],
    order: Order,
) -> Result<Vec<u8>> {
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let crypt = Crypt::of(&file, password)?;
    let mut seen = std::collections::HashSet::new();
    for layer in layers {
        if layer.page >= tree.leaves.len() {
            return Err(Error::PageOutOfRange { index: layer.page, count: tree.leaves.len() });
        }
        if !seen.insert(layer.page) {
            return Err(Error::InvalidEdit("a page is listed twice"));
        }
        if layer.placement.iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidEdit("the text layer needs a place on the page"));
        }
    }
    let mut update = Update::new(bytes);
    let mut take = || -> Result<u32> {
        let number = next_number;
        next_number = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let stream = |update: &mut Update, number: u32, mut dict: Dict, data: &[u8]| {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
        let data = match crypt.as_ref() {
            Some(crypt) => crypt.encrypt_stream(number, 0, &compressed),
            None => compressed,
        };
        update.stream(number, 0, dict, &data);
    };
    let object = |update: &mut Update, number: u32, dict: Dict| {
        let mut object = Object::Dict(dict);
        if let Some(crypt) = crypt.as_ref() {
            crypt.encrypt_strings(number, 0, &mut object);
        }
        update.object(number, 0, &object);
    };

    // The font, once for every page of this update.
    let font_number = {
        let (program, descriptor, map, to_unicode, cid_font, type0) =
            (take()?, take()?, take()?, take()?, take()?, take()?);
        let name = |n: &str| Object::Name(n.as_bytes().to_vec());
        let data = font::program();
        stream(
            &mut update,
            program,
            Dict(vec![(b"Length1".to_vec(), Object::Int(data.len() as i64))]),
            &data,
        );
        object(
            &mut update,
            descriptor,
            Dict(vec![
                (b"Type".to_vec(), name("FontDescriptor")),
                (b"FontName".to_vec(), name(font::NAME)),
                (b"Flags".to_vec(), Object::Int(4)),
                (
                    b"FontBBox".to_vec(),
                    Object::Array(vec![
                        Object::Int(0),
                        Object::Int(-i64::from(font::DESCENT)),
                        Object::Int(i64::from(font::ADVANCE)),
                        Object::Int(i64::from(font::ASCENT)),
                    ]),
                ),
                (b"ItalicAngle".to_vec(), Object::Int(0)),
                (b"Ascent".to_vec(), Object::Int(i64::from(font::ASCENT))),
                (b"Descent".to_vec(), Object::Int(-i64::from(font::DESCENT))),
                (b"CapHeight".to_vec(), Object::Int(i64::from(font::ASCENT))),
                (b"StemV".to_vec(), Object::Int(80)),
                (b"FontFile2".to_vec(), Object::Ref(program, 0)),
            ]),
        );
        stream(&mut update, map, Dict::default(), &font::cid_to_gid());
        stream(&mut update, to_unicode, Dict::default(), font::to_unicode().as_bytes());
        object(
            &mut update,
            cid_font,
            Dict(vec![
                (b"Type".to_vec(), name("Font")),
                (b"Subtype".to_vec(), name("CIDFontType2")),
                (b"BaseFont".to_vec(), name(font::NAME)),
                (
                    b"CIDSystemInfo".to_vec(),
                    Object::Dict(Dict(vec![
                        (b"Registry".to_vec(), Object::Str(b"Adobe".to_vec())),
                        (b"Ordering".to_vec(), Object::Str(b"Identity".to_vec())),
                        (b"Supplement".to_vec(), Object::Int(0)),
                    ])),
                ),
                (b"FontDescriptor".to_vec(), Object::Ref(descriptor, 0)),
                (b"DW".to_vec(), Object::Int(i64::from(font::ADVANCE))),
                (
                    b"W".to_vec(),
                    Object::Array(
                        font::MARKS
                            .iter()
                            .flat_map(|&(first, last)| {
                                [Object::Int(i64::from(first)), Object::Int(i64::from(last)), Object::Int(0)]
                            })
                            .collect(),
                    ),
                ),
                (b"CIDToGIDMap".to_vec(), Object::Ref(map, 0)),
            ]),
        );
        object(
            &mut update,
            type0,
            Dict(vec![
                (b"Type".to_vec(), name("Font")),
                (b"Subtype".to_vec(), name("Type0")),
                (b"BaseFont".to_vec(), name(font::NAME)),
                (b"Encoding".to_vec(), name("Identity-H")),
                (b"DescendantFonts".to_vec(), Object::Array(vec![Object::Ref(cid_font, 0)])),
                (b"ToUnicode".to_vec(), Object::Ref(to_unicode, 0)),
            ]),
        );
        type0
    };

    for layer in layers {
        let leaf = &tree.leaves[layer.page];
        let mut resources = tree
            .attribute(leaf, b"Resources")
            .and_then(|value| file.resolve(&value))
            .and_then(|value| value.as_dict().cloned())
            .unwrap_or_default();
        let mut fonts = resources
            .get(b"Font")
            .and_then(|value| file.resolve(value))
            .and_then(|value| value.as_dict().cloned())
            .unwrap_or_default();
        // Our font from an earlier layer goes; the new one takes a free name.
        fonts.0.retain(|(key, value)| !(key.starts_with(b"MOCR") && is_our_font(&file, value)));
        let font_name = free_font_name(&fonts);
        set(&mut fonts, font_name.as_bytes(), Object::Ref(font_number, 0));
        set(&mut resources, b"Font", Object::Dict(fonts));
        if crypt.is_some() && contains_string(&Object::Dict(resources.clone())) {
            return Err(Error::EditUnsupported(
                "this page's resources in an encrypted file cannot be copied yet",
            ));
        }

        let mut contents = page_contents(&file, &leaf.dict)?;
        contents.retain(|item| layer_of(&file, item).is_none());
        let wrapped = contents
            .first()
            .and_then(|first| file.resolve(first))
            .and_then(|first| first.as_dict().cloned())
            .is_some_and(|dict| dict.get(WRAP_KEY).is_some());
        if !wrapped && !contents.is_empty() {
            let marker = || Dict(vec![(WRAP_KEY.to_vec(), Object::Bool(true))]);
            let (open, close) = (take()?, take()?);
            stream(&mut update, open, marker(), b"q\n");
            stream(&mut update, close, marker(), b"\nQ\n");
            contents.insert(0, Object::Ref(open, 0));
            contents.push(Object::Ref(close, 0));
        }
        let number = take()?;
        let content = layer_content(layer, &font_name, order);
        let marker = Dict(vec![(LAYER_KEY.to_vec(), Object::Name(layer.quality.name().to_vec()))]);
        stream(&mut update, number, marker, content.as_bytes());
        contents.push(Object::Ref(number, 0));

        let mut page = leaf.dict.clone();
        set(&mut page, b"Contents", Object::Array(contents));
        set(&mut page, b"Resources", Object::Dict(resources));
        update.object(leaf.number, leaf.generation, &Object::Dict(page));
    }
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}

/// The OCR layer a `/Contents` item is, if it is one.
fn layer_of(file: &File<'_>, item: &Object) -> Option<LayerQuality> {
    let dict = file.resolve(item)?.as_dict()?.clone();
    LayerQuality::from_name(dict.get(LAYER_KEY)?.as_name()?)
}

fn is_our_font(file: &File<'_>, value: &Object) -> bool {
    file.resolve(value)
        .and_then(|font| {
            font.as_dict().and_then(|d| d.get(b"BaseFont")).and_then(|n| n.as_name().map(<[u8]>::to_vec))
        })
        .is_some_and(|name| name == font::NAME.as_bytes())
}

/// The OCR layer each page of the document has (`None` where it has none), in page order.
pub(crate) fn layer_qualities(bytes: &[u8]) -> Result<Vec<Option<LayerQuality>>> {
    let Start { file, .. } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    tree.leaves
        .iter()
        .map(|leaf| {
            Ok(page_contents(&file, &leaf.dict)?.iter().filter_map(|item| layer_of(&file, item)).max())
        })
        .collect()
}

/// `MOCR`, or `MOCR2x`, … : a font name the page does not use.
fn free_font_name(fonts: &Dict) -> String {
    (1..)
        .map(|n: u32| if n == 1 { "MOCR".to_string() } else { format!("MOCR{n}x") })
        .find(|name| !fonts.0.iter().any(|(key, _)| key == name.as_bytes()))
        .unwrap_or_else(|| "MOCR".to_string())
}

/// One word, ready to write: the characters in the order they are written and
/// the horizontal span they are stretched across.
struct Placed {
    /// UTF-16 code units in stream order.
    units: Vec<u16>,
    left: f32,
    right: f32,
    /// Advance leftwards from `right` (a mirrored text matrix).
    mirrored: bool,
}

fn layer_content(layer: &PageLayer, font_name: &str, order: Order) -> String {
    let mut out = String::new();
    let m = layer.placement;
    let _ = writeln!(
        out,
        "q {} {} {} {} {} {} cm BT 3 Tr",
        num(m[0]),
        num(m[1]),
        num(m[2]),
        num(m[3]),
        num(m[4]),
        num(m[5])
    );
    let bands = line_bands(&layer.lines);
    for (line, band) in layer.lines.iter().zip(&bands) {
        let Some(Band { bottom, height }) = *band else { continue };
        // Words of a line share one baseline and size, so readers see one line;
        // lines of a column share one size, so readers see one paragraph.
        let size = height * f32::from(font::UNITS_PER_EM) / f32::from(font::ASCENT + font::DESCENT);
        // Local space is y up: the displayed y of the baseline, negated.
        let baseline = -(bottom - size * f32::from(font::DESCENT) / f32::from(font::UNITS_PER_EM));
        let visual = visual_texts(line);
        let mut placed: Vec<Placed> = Vec::new();
        let mut groups: Vec<Vec<Placed>> = Vec::new();
        // A space fills the gap between two words, and is never narrower than a
        // fifth of the line's height (readers drop one too narrow to see).
        let space = |left: f32, right: f32| {
            let least = 0.2 * height;
            let (left, right) = if right - left >= least {
                (left, right)
            } else {
                let middle = (left + right) / 2.0;
                (middle - least / 2.0, middle + least / 2.0)
            };
            Placed { units: vec![0x20], left, right, mirrored: false }
        };
        match order {
            Order::LineVisualRuns => {
                let run_of = ltr_runs(line);
                let mut indices: Vec<usize> =
                    (0..line.words.len()).filter(|&i| words_ok(&line.words[i])).collect();
                indices.sort_by(|&a, &b| line.words[a].left.total_cmp(&line.words[b].left));
                let mut k = 0;
                while k < indices.len() {
                    let i = indices[k];
                    let mut end = k + 1;
                    while end < indices.len() && run_of[i].is_some() && run_of[indices[end]] == run_of[i] {
                        end += 1;
                    }
                    // Words k..end, left to right; a run of several goes last to first.
                    let mut group: Vec<Placed> = Vec::new();
                    for j in k..end {
                        let word = &line.words[indices[j]];
                        if j > 0 {
                            let before = &line.words[indices[j - 1]];
                            group.push(space(before.right, word.left.max(before.right)));
                        }
                        group.push(Placed {
                            units: visual[indices[j]].encode_utf16().collect(),
                            left: word.left,
                            right: word.right,
                            mirrored: false,
                        });
                    }
                    if end - k > 1 {
                        // The space before the run stays first, at its place.
                        let lead = if k > 0 { Some(group.remove(0)) } else { None };
                        group.reverse();
                        if let Some(lead) = lead {
                            groups.push(vec![lead]);
                        }
                        groups.push(group);
                    } else {
                        groups.extend(group.into_iter().map(|p| vec![p]));
                    }
                    k = end;
                }
            }
            Order::LineVisual => {
                let mut indices: Vec<usize> =
                    (0..line.words.len()).filter(|&i| words_ok(&line.words[i])).collect();
                indices.sort_by(|&a, &b| line.words[a].left.total_cmp(&line.words[b].left));
                // The line's characters as drawn, cut at its spaces: one run per
                // box, left to right. A word's punctuation drawn on the far side
                // of a neighbour ("؟" after an English word in an Arabic line)
                // goes into the box beside it, where the image has it.
                let runs = visual_runs(line, &indices);
                let spans = closed_gaps(
                    &indices.iter().map(|&i| (line.words[i].left, line.words[i].right)).collect::<Vec<_>>(),
                    height,
                );
                for (k, &(left, right)) in spans.iter().enumerate() {
                    if k > 0 {
                        let before = spans[k - 1].1;
                        placed.push(space(before, left.max(before)));
                    }
                    let text = runs.as_ref().map_or(&visual[indices[k]], |runs| &runs[k]);
                    placed.push(Placed {
                        units: text.encode_utf16().collect(),
                        left,
                        right,
                        mirrored: false,
                    });
                }
            }
            Order::WordVisual | Order::WordLogicalMirrored | Order::WordLogical => {
                let indices: Vec<usize> =
                    (0..line.words.len()).filter(|&i| words_ok(&line.words[i])).collect();
                for (k, &i) in indices.iter().enumerate() {
                    let word = &line.words[i];
                    let rtl_word = visual[i] != word.text && order == Order::WordLogicalMirrored;
                    placed.push(Placed {
                        units: match order {
                            Order::WordVisual => visual[i].encode_utf16().collect(),
                            _ => word.text.encode_utf16().collect(),
                        },
                        left: word.left,
                        right: word.right,
                        mirrored: rtl_word,
                    });
                    if let Some(&next) = indices.get(k + 1) {
                        let next = &line.words[next];
                        // The gap between this word and the next, wherever it is drawn.
                        let (a, b) = if next.left >= word.right {
                            (word.right, next.left)
                        } else if next.right <= word.left {
                            (next.right, word.left)
                        } else {
                            (word.right, word.right)
                        };
                        placed.push(space(a, b));
                    }
                }
            }
        }
        let _ = writeln!(out, "/{font_name} {} Tf", num(size));
        let em = size * f32::from(font::ADVANCE) / f32::from(font::UNITS_PER_EM);
        groups.extend(placed.into_iter().map(|piece| vec![piece]));
        for group in groups {
            if let [piece] = group.as_slice() {
                let count = advancing(&piece.units).max(1) as f32;
                let width = (piece.right - piece.left).max(0.01 * size);
                let scale = 100.0 * width / (count * em);
                let hex: String = piece.units.iter().map(|u| format!("{u:04X}")).collect();
                let (a, x) = if piece.mirrored { ("-1", piece.right) } else { ("1", piece.left) };
                let _ =
                    writeln!(out, "{} Tz {a} 0 0 1 {} {} Tm <{hex}> Tj", num(scale), num(x), num(baseline));
                continue;
            }
            // One text object: one scale (the narrowest character's), each piece
            // moved to its place with TJ offsets.
            let scale = group
                .iter()
                .map(|p| (p.right - p.left).max(0.01 * size) / advancing(&p.units).max(1) as f32 / em)
                .fold(f32::INFINITY, f32::min);
            let advance = em * scale;
            let mut parts = String::new();
            let mut pen = group[0].left;
            for piece in &group {
                let shift = piece.left - pen;
                if shift.abs() > 0.001 {
                    let _ = write!(parts, " {} ", num(-shift * 1000.0 / (size * scale)));
                }
                let hex: String = piece.units.iter().map(|u| format!("{u:04X}")).collect();
                let _ = write!(parts, "<{hex}>");
                pen = piece.left + advance * advancing(&piece.units) as f32;
            }
            let _ = writeln!(
                out,
                "{} Tz 1 0 0 1 {} {} Tm [{parts}] TJ",
                num(scale * 100.0),
                num(group[0].left),
                num(baseline)
            );
        }
    }
    out.push_str("ET Q\n");
    out
}

/// In a right-to-left line, which words belong to a run of left-to-right text
/// of two or more words (by UAX #9 levels, the spaces between them included):
/// the run's number per word, `None` elsewhere.
fn ltr_runs(line: &LayerLine) -> Vec<Option<usize>> {
    let mut runs = vec![None; line.words.len()];
    if !line.rtl {
        return runs;
    }
    let mut text = String::new();
    let mut ranges = Vec::new();
    for (i, word) in line.words.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        let start = text.len();
        text.push_str(&word.text);
        ranges.push(start..text.len());
    }
    let bidi = BidiInfo::new(&text, Some(Level::rtl()));
    let ltr = |range: &std::ops::Range<usize>| {
        bidi.levels[range.clone()].iter().all(|level| level.is_ltr()) && !range.is_empty()
    };
    let mut current = 0;
    for i in 0..line.words.len() {
        if !ltr(&ranges[i]) {
            continue;
        }
        let joined = i > 0 && runs[i - 1].is_some() && bidi.levels[ranges[i].start - 1].is_ltr();
        if !joined {
            current += 1;
        }
        runs[i] = Some(current);
    }
    runs
}

/// Where a line's text goes: the bottom of its band and the band's height, in
/// displayed points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Band {
    bottom: f32,
    height: f32,
}

fn median(mut values: Vec<f32>) -> Option<f32> {
    values.sort_by(f32::total_cmp);
    values.get(values.len() / 2).copied()
}

/// Each line's band (`None` for a line with no words). Word boxes are tight to
/// ink, so a line's lowest descender or tallest letter (or a neighbour's ink)
/// would make its band taller than its pitch and merge it with the next line in
/// a reader: the band is the median of its words' tops and bottoms. Readers
/// group lines into paragraphs only at one font size, so a line takes the median
/// height of the lines in its column (those it overlaps horizontally), unless
/// its own is far from it (a heading).
fn line_bands(lines: &[LayerLine]) -> Vec<Option<Band>> {
    let raw: Vec<Option<(f32, f32, f32, f32)>> = lines
        .iter()
        .map(|line| {
            let words: Vec<&LayerWord> = line.words.iter().filter(|w| words_ok(w)).collect();
            let top = median(words.iter().map(|w| w.top).collect())?;
            let bottom = median(words.iter().map(|w| w.bottom).collect())?;
            let left = words.iter().map(|w| w.left).fold(f32::INFINITY, f32::min);
            let right = words.iter().map(|w| w.right).fold(f32::NEG_INFINITY, f32::max);
            Some((bottom, (bottom - top).max(1.0), left, right))
        })
        .collect();
    raw.iter()
        .map(|line| {
            let (bottom, height, left, right) = (*line)?;
            let column = median(
                raw.iter()
                    .flatten()
                    .filter(|(_, _, l, r)| r.min(right) - l.max(left) > 0.3 * (r - l).min(right - left))
                    .map(|(_, h, _, _)| *h)
                    .collect(),
            )
            .unwrap_or(height);
            let height = if (0.6 * column..=1.6 * column).contains(&height) { column } else { height };
            Some(Band { bottom, height })
        })
        .collect()
}

/// Words' spans (left to right) with gaps between neighbours closed to 0.4 of
/// the line's height, each word grown halfway into it. Boxes tight to faint ink
/// (a 100 dpi fax) leave word gaps over an em wide, and readers cut a line into
/// columns at such gaps. A gap of 1.5 heights or more is left: that is a real
/// gap between table cells or columns.
fn closed_gaps(spans: &[(f32, f32)], height: f32) -> Vec<(f32, f32)> {
    let (widest, kept) = (0.4 * height, 1.5 * height);
    let mut out = spans.to_vec();
    for k in 1..out.len() {
        let gap = out[k].0 - out[k - 1].1;
        if gap > widest && gap < kept {
            let grow = (gap - widest) / 2.0;
            out[k - 1].1 += grow;
            out[k].0 -= grow;
        }
    }
    out
}

/// How many of `units` advance the pen (marks do not).
fn advancing(units: &[u16]) -> usize {
    units.iter().filter(|&&u| !font::is_mark(u)).count()
}

fn words_ok(word: &LayerWord) -> bool {
    !word.text.trim().is_empty() && word.right > word.left
}

/// The text of `indices`' words (left to right by box) as drawn: UAX #9 on the
/// whole line, cut at the spaces between its words. `None` when the runs do not
/// match the boxes one to one.
fn visual_runs(line: &LayerLine, indices: &[usize]) -> Option<Vec<String>> {
    let words: Vec<&str> = indices
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<usize>>()
        .into_iter()
        .map(|i| line.words[i].text.as_str())
        .collect();
    let text = words.join(" ");
    let bidi = BidiInfo::new(&text, Some(if line.rtl { Level::rtl() } else { Level::ltr() }));
    let paragraph = bidi.paragraphs.first()?;
    let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
    let mut drawn = String::with_capacity(text.len());
    for run in runs {
        let piece = &text[run.clone()];
        if levels[run.start].is_rtl() {
            drawn.extend(piece.chars().rev());
        } else {
            drawn.push_str(piece);
        }
    }
    let pieces: Vec<String> = drawn.split(' ').filter(|p| !p.is_empty()).map(str::to_owned).collect();
    (pieces.len() == indices.len()).then_some(pieces)
}

/// Each word's text in the order its characters are drawn left to right (UAX #9
/// on the whole line, in the line's direction), so a word keeps the order its
/// neighbours give it ("64-bit" in an Arabic line is drawn "bit-64").
fn visual_texts(line: &LayerLine) -> Vec<String> {
    let mut text = String::new();
    let mut ranges = Vec::new();
    for (i, word) in line.words.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        let start = text.len();
        text.push_str(&word.text);
        ranges.push(start..text.len());
    }
    let level = if line.rtl { Level::rtl() } else { Level::ltr() };
    let bidi = BidiInfo::new(&text, Some(level));
    let Some(paragraph) = bidi.paragraphs.first() else {
        return line.words.iter().map(|w| w.text.clone()).collect();
    };
    let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
    // Byte offsets of the line's characters, left to right.
    let mut visual: Vec<usize> = Vec::with_capacity(text.len());
    for run in runs {
        let mut offsets: Vec<usize> =
            text[run.clone()].char_indices().map(|(at, _)| run.start + at).collect();
        if levels[run.start].is_rtl() {
            offsets.reverse();
        }
        visual.extend(offsets);
    }
    ranges
        .iter()
        .map(|range| {
            visual
                .iter()
                .filter(|&&at| range.contains(&at))
                .filter_map(|&at| text[at..].chars().next())
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(words: &[&str], rtl: bool) -> LayerLine {
        LayerLine {
            words: words
                .iter()
                .map(|t| LayerWord { text: (*t).into(), left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 })
                .collect(),
            rtl,
        }
    }

    #[test]
    fn words_are_drawn_in_the_lines_visual_order() {
        assert_eq!(visual_texts(&line(&["سلام", "عليكم"], true)), ["مالس", "مكيلع"]);
        assert_eq!(
            visual_texts(&line(&["الإصدار", "(64-bit)"], true)),
            ["رادصإلا", ")bit-64("],
            "code points; drawn mirrored"
        );
        assert_eq!(visual_texts(&line(&["The", "end."], false)), ["The", "end."]);
        // A mark stays with its letter: drawn just left of it.
        assert_eq!(visual_texts(&line(&["بِ"], true)), ["\u{650}ب"]);
    }

    #[test]
    fn our_font_keeps_its_name_and_others_are_avoided() {
        let names =
            |keys: &[&str]| Dict(keys.iter().map(|k| (k.as_bytes().to_vec(), Object::Null)).collect());
        assert_eq!(free_font_name(&names(&["F1"])), "MOCR");
        assert_eq!(free_font_name(&names(&["MOCR"])), "MOCR2x");
    }
}
