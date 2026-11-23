//! Scan to editable text (spec §5.8, ADR 0039): a page's recognised text
//! becomes real page text, paragraph blocks the M3 editor opens (ADR 0023).
//!
//! 1. **Paragraphs** from the page's layer lines, in reading order: lines of
//!    one direction and height, close together and overlapping, until a line
//!    ends short ([`paragraphs`]). Each keeps its box, its alignment (left,
//!    right, centred, justified), its line height and line pitch.
//! 2. **Type:** each paragraph is laid out by the text engine (`typeset`) in a
//!    bundled family (Naskh unless chosen: Amiri, with its matching Latin), at
//!    the size its lines' ink and pitch give, shrunk until it takes no more
//!    lines than the scan did, at the scan's line pitch. The text is in
//!    logical order and extracts so (ADR 0021's writer).
//! 3. **The scan:** kept behind, with a patch of the paper's colour over each
//!    line's ink so the old glyphs do not show through; or removed from the
//!    page (by byte range, ADR 0051), as the person chooses. Ink and paper
//!    colours are sampled from the scan.
//! 4. **One update** per page set: the page's OCR layer (ours) and any other
//!    tool's invisible text go, since the page's text is now its content; one
//!    stream draws patches and paragraphs, marked `/MirsamConverted` with the
//!    digest of the layer it came from, so the review still finds the page's
//!    flagged words (ADR 0037).
//!
//! A page whose words still need a decision is refused by the caller: an
//! uncertain word is never turned into page text without a person deciding.

use std::fmt::Write as _;

use crate::edit::{PageTree, Start, contains_string, set};
use crate::error::{Error, Result};
use crate::fonts::{FontFamily, for_text};
use crate::pdf::crypt::Crypt;
use crate::pdf::text::{NamedFont, Paint, draw_with_subsets, fonts_to_embed, write_font};
use crate::pdf::write::Update;
use crate::pdf::{Dict, Object};
use crate::text_edit::{WRAP_KEY, free_prefix, num, page_contents};
use crate::typeset::{
    Align, BaseDirection, FontFace, LineHeight, ParagraphStyle, Span, TextLayout, layout, subset_fonts,
};

use super::OcrImage;
use super::layer::{CONVERTED_KEY, LAYER_KEY, PageLayer};

/// A page to convert.
#[derive(Clone, Debug, PartialEq)]
pub struct PageConversion {
    pub page: usize,
    /// The page's words as the document's layer has them (corrections in).
    pub layer: PageLayer,
    /// The digest of the document's layer the text comes from.
    pub from: u32,
    /// Keep the scan behind the text (patched where the text is), or remove it.
    pub keep_scan: bool,
    /// The bundled family; Naskh when `None`.
    pub family: Option<FontFamily>,
    /// Ink and paper colour, sampled from the scan.
    pub colors: Colors,
}

/// A paragraph's ink and the paper under it, sRGB 0–1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub ink: [f32; 3],
    pub paper: [f32; 3],
}

impl Default for Colors {
    fn default() -> Self {
        Self { ink: [0.0; 3], paper: [1.0; 3] }
    }
}

/// A paragraph rebuilt from layer lines, on the displayed page (points, y down).
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    /// Its lines' words, lines joined by a space: logical order.
    pub text: String,
    pub rtl: bool,
    pub align: Align,
    /// `[left, top, right, bottom]` of all its lines.
    pub bounds: [f32; 4],
    /// Each line's band: the median top and bottom of its words, its ends.
    pub lines: Vec<[f32; 4]>,
    /// The median height of its lines' ink.
    pub height: f32,
    /// The median distance from one line's bottom to the next's.
    pub pitch: Option<f32>,
}

fn median(mut values: Vec<f32>) -> Option<f32> {
    values.sort_by(f32::total_cmp);
    values.get(values.len() / 2).copied()
}

struct Band {
    text: String,
    rtl: bool,
    b: [f32; 4],
}

/// The layer's lines as paragraphs, in reading order.
pub fn paragraphs(layer: &PageLayer) -> Vec<Paragraph> {
    let bands: Vec<Band> = layer
        .lines
        .iter()
        .filter_map(|line| {
            let words: Vec<_> = line.words.iter().filter(|w| !w.text.trim().is_empty() && w.right > w.left).collect();
            let top = median(words.iter().map(|w| w.top).collect())?;
            let bottom = median(words.iter().map(|w| w.bottom).collect())?;
            let left = words.iter().map(|w| w.left).fold(f32::INFINITY, f32::min);
            let right = words.iter().map(|w| w.right).fold(f32::NEG_INFINITY, f32::max);
            let text = words.iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" ");
            Some(Band { text, rtl: line.rtl, b: [left, top, right, bottom.max(top + 1.0)] })
        })
        .collect();
    let mut groups: Vec<Vec<&Band>> = Vec::new();
    for band in &bands {
        let joins = groups.last().is_some_and(|group| continues(group, band));
        if joins {
            if let Some(group) = groups.last_mut() {
                group.push(band);
            }
        } else {
            groups.push(vec![band]);
        }
    }
    groups.into_iter().map(|group| paragraph(&group)).collect()
}

/// Whether `next` carries on the paragraph `group`.
fn continues(group: &[&Band], next: &Band) -> bool {
    let Some(last) = group.last() else { return false };
    let h = median(group.iter().map(|b| b.b[3] - b.b[1]).collect()).unwrap_or(1.0).max(1.0);
    let nh = next.b[3] - next.b[1];
    let left = group.iter().map(|b| b.b[0]).fold(f32::INFINITY, f32::min);
    let right = group.iter().map(|b| b.b[2]).fold(f32::NEG_INFINITY, f32::max);
    let gap = next.b[1] - last.b[3];
    let overlap = next.b[2].min(right) - next.b[0].max(left);
    let narrower = (next.b[2] - next.b[0]).min(right - left);
    // The last line ended well short of the paragraph's end: it closed it.
    let ended = group.len() >= 2
        && if last.rtl { last.b[0] > left + 2.0 * h } else { last.b[2] < right - 2.0 * h };
    next.rtl == last.rtl
        && (0.7 * h..=1.4 * h).contains(&nh)
        && (-0.3 * h..=1.1 * h).contains(&gap)
        && overlap >= 0.5 * narrower
        && !ended
}

fn paragraph(group: &[&Band]) -> Paragraph {
    let left = group.iter().map(|b| b.b[0]).fold(f32::INFINITY, f32::min);
    let right = group.iter().map(|b| b.b[2]).fold(f32::NEG_INFINITY, f32::max);
    let top = group.iter().map(|b| b.b[1]).fold(f32::INFINITY, f32::min);
    let bottom = group.iter().map(|b| b.b[3]).fold(f32::NEG_INFINITY, f32::max);
    let height = median(group.iter().map(|b| b.b[3] - b.b[1]).collect()).unwrap_or(1.0).max(1.0);
    let pitch = (group.len() >= 2).then(|| median(group.windows(2).map(|w| w[1].b[3] - w[0].b[3]).collect())).flatten();
    let rtl = group[0].rtl;
    let tol = 0.6 * height;
    let align = if group.len() < 2 {
        Align::Start
    } else {
        // The paragraph's last line is free to end short.
        let body = &group[..group.len() - 1];
        let flush_left = group.iter().all(|b| b.b[0] - left <= tol);
        let flush_right = group.iter().all(|b| right - b.b[2] <= tol);
        let body_left = body.iter().all(|b| b.b[0] - left <= tol);
        let body_right = body.iter().all(|b| right - b.b[2] <= tol);
        let centre = (left + right) / 2.0;
        let centred = group.iter().all(|b| ((b.b[0] + b.b[2]) / 2.0 - centre).abs() <= tol);
        if (rtl && flush_right && body_left) || (!rtl && flush_left && body_right) {
            if flush_left && flush_right { Align::Start } else { Align::Justify }
        } else if flush_left && !flush_right && centred {
            Align::Left
        } else if centred && !flush_left && !flush_right {
            Align::Center
        } else if flush_right && !flush_left {
            Align::Right
        } else if flush_left {
            Align::Left
        } else {
            Align::Start
        }
    };
    Paragraph {
        text: group.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join(" "),
        rtl,
        align,
        bounds: [left, top, right, bottom],
        lines: group.iter().map(|b| b.b).collect(),
        height,
        pitch,
    }
}

/// The ink and paper colour of an area of a page image (`scale` pixels per
/// point, `area` displayed points): the mean of its darkest and its lightest
/// pixels. Faint or low-contrast ink is written black.
pub fn sample_colors(image: &OcrImage, scale: f32, area: [f32; 4]) -> Colors {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let px = |v: f32, max: usize| ((v * scale).round().max(0.0) as usize).min(max);
    let (x0, y0, x1, y1) = (px(area[0], w), px(area[1], h), px(area[2], w), px(area[3], h));
    let rgb = image.rgb();
    let mut pixels: Vec<(f32, [u8; 3])> = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) * 3;
            if let Some(p) = rgb.get(i..i + 3) {
                let lum = 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]);
                pixels.push((lum, [p[0], p[1], p[2]]));
            }
        }
    }
    if pixels.len() < 16 {
        return Colors::default();
    }
    pixels.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mean = |slice: &[(f32, [u8; 3])]| -> ([f32; 3], f32) {
        let n = slice.len().max(1) as f32;
        let mut c = [0.0f32; 3];
        let mut lum = 0.0;
        for (l, p) in slice {
            lum += l;
            for k in 0..3 {
                c[k] += f32::from(p[k]);
            }
        }
        (c.map(|v| v / n / 255.0), lum / n)
    };
    let n = pixels.len();
    let (ink, ink_lum) = mean(&pixels[..(n / 12).max(1)]);
    let (paper, paper_lum) = mean(&pixels[n * 7 / 10..]);
    let ink = if paper_lum - ink_lum < 60.0 || ink_lum > 140.0 { [0.0; 3] } else { ink };
    Colors { ink, paper }
}

/// A paragraph set in type: its text, layout, and where its box's top-left goes.
struct Set {
    text: String,
    laid: TextLayout,
    left: f32,
    top: f32,
    color: [f32; 3],
}

/// Lays `p` out in `faces`: as large as its ink says, no more lines than the scan's.
fn set_paragraph(p: &Paragraph, faces: &[FontFace], ink: [f32; 3]) -> Result<Set> {
    let by_ink = p.height / 0.75;
    let first = match p.pitch {
        Some(pitch) => (by_ink + pitch / 1.25) / 2.0,
        None => by_ink,
    }
    .clamp(4.0, 120.0);
    let width = (p.bounds[2] - p.bounds[0]) * 1.02 + 1.0;
    let lines = p.lines.len();
    let mut size = first;
    let laid = loop {
        let style = ParagraphStyle {
            direction: if p.rtl { BaseDirection::Rtl } else { BaseDirection::Ltr },
            align: p.align,
            width: Some(width),
            line_height: match p.pitch {
                Some(pitch) => LineHeight::Points(pitch.max(size)),
                None => LineHeight::Normal,
            },
        };
        let laid = layout(&p.text, &[Span::new(0..p.text.len(), 0, size)], faces, &style)
            .map_err(|err| Error::Typeset(err.code()))?;
        if laid.lines.len() <= lines || size < 0.6 * first {
            break laid;
        }
        size *= 0.95;
    };
    // The first line's baseline where the scan's was: its ink's bottom, a
    // little above it for Arabic (descending letters).
    let baseline = p.lines[0][3] - if p.rtl { 0.2 } else { 0.03 } * size;
    let first_baseline = laid.lines.first().map_or(size, |l| l.baseline);
    Ok(Set { text: p.text.clone(), left: p.bounds[0], top: baseline - first_baseline, laid, color: ink })
}

/// `layouts` (of `texts`) as one layout of the texts joined by line breaks:
/// what a shared font subset is made from.
fn joined(texts: &[&str], layouts: &[&TextLayout]) -> (String, TextLayout) {
    let mut text = String::new();
    let mut lines = Vec::new();
    for (t, laid) in texts.iter().zip(layouts) {
        if !text.is_empty() {
            text.push('\n');
        }
        let offset = text.len();
        text.push_str(t);
        let shift = |r: &std::ops::Range<usize>| r.start + offset..r.end + offset;
        for line in &laid.lines {
            let mut line = line.clone();
            line.text_range = shift(&line.text_range);
            for run in &mut line.runs {
                run.text_range = shift(&run.text_range);
                for cluster in &mut run.clusters {
                    cluster.text = shift(&cluster.text);
                }
            }
            lines.push(line);
        }
    }
    (text, TextLayout { lines, width: 0.0, height: 0.0 })
}

fn take(next: &mut u32) -> Result<u32> {
    let number = *next;
    *next = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
    Ok(number)
}

/// The bytes to append to `bytes` to convert `pages` (each once): their OCR
/// layer goes, a stream with the scan's patches and the paragraphs comes.
/// Another tool's invisible text and the scan image are removed beforehand
/// by the caller (`Document::with_conversion`).
pub(crate) fn convert_update(bytes: &[u8], password: Option<&str>, pages: &[PageConversion]) -> Result<Vec<u8>> {
    let Start { file, section, form, mut next_number } = Start::read(bytes)?;
    let tree = PageTree::read(&file)?;
    let crypt = Crypt::of(&file, password)?;
    let mut update = Update::new(bytes);
    let mut seen = std::collections::HashSet::new();
    for conversion in pages {
        if conversion.page >= tree.leaves.len() {
            return Err(Error::PageOutOfRange { index: conversion.page, count: tree.leaves.len() });
        }
        if !seen.insert(conversion.page) {
            return Err(Error::InvalidEdit("a page is listed twice"));
        }
    }
    for conversion in pages {
        let leaf = &tree.leaves[conversion.page];
        let paragraphs = paragraphs(&conversion.layer);
        if paragraphs.is_empty() {
            return Err(Error::InvalidEdit("the page has no recognised text to convert"));
        }
        let family = conversion.family.unwrap_or(FontFamily::Naskh);
        let all: String = paragraphs.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n");
        let fonts: Vec<NamedFont> = for_text(family, &all)?;
        let faces: Vec<FontFace> = fonts.iter().map(|f| f.face.clone()).collect();
        let sets: Vec<Set> =
            paragraphs.iter().map(|p| set_paragraph(p, &faces, conversion.colors.ink)).collect::<Result<_>>()?;
        let (text, combined) = joined(
            &sets.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            &sets.iter().map(|s| &s.laid).collect::<Vec<_>>(),
        );
        let subsets = subset_fonts(&text, &combined, &faces).map_err(|err| Error::Typeset(err.code()))?;

        let mut resources = tree
            .attribute(leaf, b"Resources")
            .and_then(|value| file.resolve(&value))
            .and_then(|value| value.as_dict().cloned())
            .unwrap_or_default();
        let mut font_names = resources
            .get(b"Font")
            .and_then(|value| file.resolve(value))
            .and_then(|value| value.as_dict().cloned())
            .unwrap_or_default();
        let prefix = free_prefix(&font_names);
        for font in fonts_to_embed(subsets.clone(), &fonts, &prefix) {
            let number = write_font(&mut update, crypt.as_ref(), &mut next_number, &font)?;
            set(&mut font_names, font.resource.as_bytes(), Object::Ref(number, 0));
        }
        set(&mut resources, b"Font", Object::Dict(font_names));
        if crypt.is_some() && contains_string(&Object::Dict(resources.clone())) {
            return Err(Error::EditUnsupported("this page's resources in an encrypted file cannot be copied yet"));
        }

        // The stream: patches over the scan's lines, then the paragraphs.
        let m = conversion.layer.placement;
        let mut content = String::new();
        let _ = writeln!(
            content,
            "q {} {} {} {} {} {} cm",
            num(m[0]),
            num(m[1]),
            num(m[2]),
            num(m[3]),
            num(m[4]),
            num(m[5])
        );
        if conversion.keep_scan {
            let [r, g, b] = conversion.colors.paper;
            let _ = writeln!(content, "{} {} {} rg", num(r), num(g), num(b));
            for p in &paragraphs {
                for line in &p.lines {
                    let pad = 0.3 * (line[3] - line[1]).max(1.0);
                    let _ = writeln!(
                        content,
                        "{} {} {} {} re f",
                        num(line[0] - pad),
                        num(-(line[3] + pad)),
                        num(line[2] - line[0] + 2.0 * pad),
                        num(line[3] - line[1] + 2.0 * pad)
                    );
                }
            }
        }
        content.push_str("/MirsamText BMC\n");
        let mut data = content.into_bytes();
        for s in &sets {
            let paint = [Paint { range: 0..s.text.len(), color: s.color, slant: false }];
            data.extend(draw_with_subsets(&s.text, &s.laid, &subsets, (s.left, -s.top), &paint, &prefix)?);
        }
        data.extend_from_slice(b"EMC\nQ\n");

        let stream = |update: &mut Update, number: u32, mut dict: Dict, data: &[u8]| {
            let compressed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
            dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
            let data = match crypt.as_ref() {
                Some(crypt) => crypt.encrypt_stream(number, 0, &compressed),
                None => compressed,
            };
            update.stream(number, 0, dict, &data);
        };
        // The page's own content wrapped once; its OCR layer and earlier
        // converted text go: the page's text is the new stream's.
        let mut contents = page_contents(&file, &leaf.dict)?;
        contents.retain(|item| {
            !file
                .resolve(item)
                .and_then(|o| o.as_dict().cloned())
                .is_some_and(|d| d.get(LAYER_KEY).is_some() || d.get(CONVERTED_KEY).is_some())
        });
        let wrapped = contents
            .first()
            .and_then(|first| file.resolve(first))
            .and_then(|first| first.as_dict().cloned())
            .is_some_and(|dict| dict.get(WRAP_KEY).is_some());
        if !wrapped && !contents.is_empty() {
            let marker = || Dict(vec![(WRAP_KEY.to_vec(), Object::Bool(true))]);
            let (open, close) = (take(&mut next_number)?, take(&mut next_number)?);
            stream(&mut update, open, marker(), b"q\n");
            stream(&mut update, close, marker(), b"\nQ\n");
            contents.insert(0, Object::Ref(open, 0));
            contents.push(Object::Ref(close, 0));
        }
        let number = take(&mut next_number)?;
        let marker = Dict(vec![(CONVERTED_KEY.to_vec(), Object::Int(i64::from(conversion.from)))]);
        stream(&mut update, number, marker, &data);
        contents.push(Object::Ref(number, 0));

        let mut page = leaf.dict.clone();
        set(&mut page, b"Contents", Object::Array(contents));
        set(&mut page, b"Resources", Object::Dict(resources));
        update.object(leaf.number, leaf.generation, &Object::Dict(page));
    }
    Ok(update.finish(form, file.trailer.clone(), section, next_number))
}
