//! Writing laid-out text (`typeset`) into PDF (ADR 0005 amendment, ADR 0019):
//! content-stream operators that draw the glyphs, and the fonts they use,
//! embedded as subsets.
//!
//! Each font is a Type 0 font with a CIDFontType2 descendant, Identity-H
//! encoding, and an identity `/CIDToGIDMap`: the subset's glyph ids are the
//! CIDs (`typeset::subset_fonts`). Text extraction goes through a ToUnicode
//! map for glyphs that draw a whole cluster, and `/ActualText` around
//! clusters drawn with several glyphs (a letter and its separate dots). That
//! is the recipe the M3 test writer proved PDFium draws and our extractor
//! reads back in logical order.

use std::fmt::Write as _;

use super::crypt::Crypt;
use super::write::Update;
use super::{Dict, Object};
use crate::error::{Error, Result};
use crate::typeset::{FontFace, FontSubset, TextLayout, subset_fonts};

/// A font for writing, with the PostScript name it is embedded under.
#[derive(Debug, Clone)]
pub(crate) struct NamedFont {
    pub(crate) face: FontFace,
    pub(crate) name: &'static str,
}

/// Text ready for a content stream.
#[derive(Debug)]
pub(crate) struct TextDrawing {
    /// `BT … ET` operators.
    pub(crate) content: Vec<u8>,
    /// The fonts the operators use, as `/{prefix}0`, `/{prefix}1`, … in this order.
    pub(crate) fonts: Vec<FontToEmbed>,
}

/// One font subset to embed.
#[derive(Debug, Clone)]
pub(crate) struct FontToEmbed {
    /// Its resource name (`F0`, …).
    pub(crate) resource: String,
    /// `/BaseFont`: a subset tag and the font's PostScript name.
    pub(crate) base_font: String,
    /// The subset font program (TrueType).
    pub(crate) program: Vec<u8>,
    /// Advance of each CID, in thousandths of an em.
    pub(crate) widths: Vec<(u16, f32)>,
    /// The text each CID stands for, where it draws a whole cluster.
    pub(crate) text: Vec<(u16, String)>,
    /// Ascent and descent (negative), in thousandths of an em.
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
}

/// Draws `layout` (of `text`, made with `fonts`) with the top-left corner of
/// its box at (`left`, `top`) in the content stream's space (y up), in `color`.
/// The fonts' resource names are `prefix` and a number.
pub(crate) fn draw_text(
    text: &str,
    layout: &TextLayout,
    fonts: &[NamedFont],
    (left, top): (f32, f32),
    color: [f32; 3],
    prefix: &str,
) -> Result<TextDrawing> {
    let faces: Vec<FontFace> = fonts.iter().map(|font| font.face.clone()).collect();
    let subsets = subset_fonts(text, layout, &faces).map_err(|err| Error::Typeset(err.code()))?;
    let content = draw_with_subsets(text, layout, &subsets, (left, top), color, prefix);
    Ok(TextDrawing { content, fonts: fonts_to_embed(subsets, fonts, prefix) })
}

/// [`draw_text`]'s operators for `layout` (of `text`), drawn with glyphs of
/// `subsets` made beforehand: several texts can share one set of subsets
/// when those were made from text that holds them all (ADR 0050).
pub(crate) fn draw_with_subsets(
    text: &str,
    layout: &TextLayout,
    subsets: &[FontSubset],
    (left, top): (f32, f32),
    color: [f32; 3],
    prefix: &str,
) -> Vec<u8> {
    // Resource names in the order the subsets come (by font index).
    let resource_of = |font: usize| subsets.iter().position(|s| s.font == font);

    let mut content = String::new();
    let [r, g, b] = color;
    for line in &layout.lines {
        let baseline = top - line.baseline;
        for run in &line.runs {
            let (Some(resource), Some(subset)) =
                (resource_of(run.font), subsets.iter().find(|s| s.font == run.font))
            else {
                continue;
            };
            let _ = writeln!(
                content,
                "BT /{prefix}{resource} {} Tf {} {} {} rg",
                num(run.size),
                num(r),
                num(g),
                num(b)
            );
            for (i, glyph) in run.glyphs.iter().enumerate() {
                let cluster = &run.clusters[glyph.cluster];
                // A cluster drawn with several glyphs says what it stands for
                // once. (One glyph drawing several letters, a ligature, says it
                // in the ToUnicode map: labelling it too misplaces its letters
                // in PDFium's text inside form XObjects.)
                let several = cluster.glyphs.len() > 1;
                if several && i == cluster.glyphs.start {
                    let utf16: String =
                        text[cluster.text.clone()].encode_utf16().map(|u| format!("{u:04X}")).collect();
                    let _ = writeln!(content, "/Span << /ActualText <FEFF{utf16}> >> BDC");
                }
                let cid = subset.cid(glyph.id).unwrap_or(0);
                let _ = writeln!(
                    content,
                    "1 0 0 1 {} {} Tm <{cid:04X}> Tj",
                    num(left + glyph.x),
                    num(baseline + glyph.y_offset)
                );
                if several && i + 1 == cluster.glyphs.end {
                    content.push_str("EMC\n");
                }
            }
            content.push_str("ET\n");
        }
    }

    content.into_bytes()
}

/// The fonts to embed for `subsets` (of `fonts`), named `prefix` and a number.
pub(crate) fn fonts_to_embed(
    subsets: Vec<FontSubset>,
    fonts: &[NamedFont],
    prefix: &str,
) -> Vec<FontToEmbed> {
    subsets
        .into_iter()
        .enumerate()
        .map(|(resource, subset)| {
            let font = &fonts[subset.font];
            let upem = f32::from(font.face.units_per_em());
            let metrics = font.face.line_metrics();
            FontToEmbed {
                resource: format!("{prefix}{resource}"),
                base_font: format!("{}+{}", subset_tag(&subset.data), font.name),
                widths: subset.glyphs.iter().map(|g| (g.cid, g.width)).collect(),
                text: subset.glyphs.iter().filter_map(|g| Some((g.cid, g.text.clone()?))).collect(),
                ascent: metrics.ascent * 1000.0 / upem,
                descent: -metrics.descent * 1000.0 / upem,
                program: subset.data,
            }
        })
        .collect()
}

/// Six capital letters from the subset's bytes, as PDF names subsets
/// (`ABCDEF+Name`).
fn subset_tag(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| {
            let letter = ((hash >> (i * 5)) % 26) as u8;
            char::from(b'A' + letter)
        })
        .collect()
}

/// Writes a font's five objects (font program, descriptor, CID font,
/// ToUnicode map, Type 0 font) into `update`, encrypted where the file is,
/// numbering them from `next`. Returns the Type 0 font's number.
pub(crate) fn write_font(
    update: &mut Update,
    crypt: Option<&Crypt>,
    next: &mut u32,
    font: &FontToEmbed,
) -> Result<u32> {
    let mut take = || -> Result<u32> {
        let number = *next;
        *next = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let (file, descriptor, cid_font, to_unicode, type0) = (take()?, take()?, take()?, take()?, take()?);
    let name = |n: &str| Object::Name(n.as_bytes().to_vec());
    let real = |value: f32| Object::Real(f64::from(value.round()));
    let base = name(&font.base_font);

    let stream = |update: &mut Update, number: u32, mut dict: Dict, data: &[u8]| {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        dict.0.push((b"Filter".to_vec(), name("FlateDecode")));
        let data = match crypt {
            Some(crypt) => crypt.encrypt_stream(number, 0, &compressed),
            None => compressed,
        };
        update.stream(number, 0, dict, &data);
    };
    let object = |update: &mut Update, number: u32, dict: Dict| {
        let mut object = Object::Dict(dict);
        if let Some(crypt) = crypt {
            crypt.encrypt_strings(number, 0, &mut object);
        }
        update.object(number, 0, &object);
    };

    let length = i64::try_from(font.program.len()).map_err(|_| Error::InvalidEdit("font too large"))?;
    stream(update, file, Dict(vec![(b"Length1".to_vec(), Object::Int(length))]), &font.program);
    object(
        update,
        descriptor,
        Dict(vec![
            (b"Type".to_vec(), name("FontDescriptor")),
            (b"FontName".to_vec(), base.clone()),
            // Symbolic: the font's own glyph ids, not a standard encoding.
            (b"Flags".to_vec(), Object::Int(4)),
            (
                b"FontBBox".to_vec(),
                Object::Array(vec![real(0.0), real(font.descent), real(1000.0), real(font.ascent)]),
            ),
            (b"ItalicAngle".to_vec(), Object::Int(0)),
            (b"Ascent".to_vec(), real(font.ascent)),
            (b"Descent".to_vec(), real(font.descent)),
            (b"CapHeight".to_vec(), real(font.ascent * 0.7)),
            (b"StemV".to_vec(), Object::Int(80)),
            (b"FontFile2".to_vec(), Object::Ref(file, 0)),
        ]),
    );
    let widths = font
        .widths
        .iter()
        .flat_map(|&(cid, width)| [Object::Int(i64::from(cid)), Object::Array(vec![real(width)])])
        .collect();
    object(
        update,
        cid_font,
        Dict(vec![
            (b"Type".to_vec(), name("Font")),
            (b"Subtype".to_vec(), name("CIDFontType2")),
            (b"BaseFont".to_vec(), base.clone()),
            (
                b"CIDSystemInfo".to_vec(),
                Object::Dict(Dict(vec![
                    (b"Registry".to_vec(), Object::Str(b"Adobe".to_vec())),
                    (b"Ordering".to_vec(), Object::Str(b"Identity".to_vec())),
                    (b"Supplement".to_vec(), Object::Int(0)),
                ])),
            ),
            (b"FontDescriptor".to_vec(), Object::Ref(descriptor, 0)),
            (b"CIDToGIDMap".to_vec(), name("Identity")),
            (b"DW".to_vec(), Object::Int(0)),
            (b"W".to_vec(), Object::Array(widths)),
        ]),
    );
    stream(update, to_unicode, Dict::default(), to_unicode_cmap(&font.text).as_bytes());
    object(
        update,
        type0,
        Dict(vec![
            (b"Type".to_vec(), name("Font")),
            (b"Subtype".to_vec(), name("Type0")),
            (b"BaseFont".to_vec(), base),
            (b"Encoding".to_vec(), name("Identity-H")),
            (b"DescendantFonts".to_vec(), Object::Array(vec![Object::Ref(cid_font, 0)])),
            (b"ToUnicode".to_vec(), Object::Ref(to_unicode, 0)),
        ]),
    );
    Ok(type0)
}

fn to_unicode_cmap(text: &[(u16, String)]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    for chunk in text.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for (cid, meaning) in chunk {
            let utf16: String = meaning.encode_utf16().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(cmap, "<{cid:04X}> <{utf16}>");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
    cmap
}

/// A number for a content stream: three decimals at most, no exponent.
fn num(value: f32) -> String {
    crate::annotate::num(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subset_tags_are_six_capitals() {
        let tag = subset_tag(b"some font bytes");
        assert_eq!(tag.len(), 6);
        assert!(tag.chars().all(|c| c.is_ascii_uppercase()));
        assert_eq!(tag, subset_tag(b"some font bytes"));
        assert_ne!(tag, subset_tag(b"other font bytes"));
    }

    #[test]
    fn the_cmap_maps_cids_to_utf16() {
        let cmap = to_unicode_cmap(&[(3, "ل".into()), (4, "ﻻ".into()), (5, "ff".into())]);
        assert!(cmap.contains("3 beginbfchar\n<0003> <0644>\n<0004> <FEFB>\n<0005> <00660066>\nendbfchar"));
    }
}
