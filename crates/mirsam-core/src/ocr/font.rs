//! The font of the OCR text layer: a TrueType program with no outlines.
//!
//! The text layer is never drawn (text render mode 3), so its font needs no
//! glyphs; it needs to be a font every reader loads, and a way back from codes
//! to characters. Like Tesseract's "GlyphLessFont":
//!
//! - **codes are UTF-16 code units** (Identity-H, two bytes each), so no subset
//!   or per-document glyph table is needed;
//! - **every code maps to glyph 1** (`/CIDToGIDMap`), one advance wide (`/DW`),
//!   so a word is stretched to its box by horizontal scaling alone. Its outline
//!   is a rectangle filling the advance from descent to ascent: never painted,
//!   but PDFium skips a text object whose glyphs have no extent, which dropped
//!   one-character words ("1", "و") from Chrome's copy;
//! - **combining marks** (tashkeel) map to glyph 2 and have no advance, so a
//!   mark is drawn from its letter's origin, as shaping engines draw it, and
//!   readers attach it to that letter;
//! - **`/ToUnicode` is the identity** on UTF-16 code units.
//!
//! The program is built here (about 500 bytes), so there is no font licence to
//! think about and nothing to subset.

/// Units per em, and the advance of every glyph in them.
pub(crate) const UNITS_PER_EM: u16 = 1000;
pub(crate) const ADVANCE: u16 = 500;
/// Ascent and descent (positive, below the baseline) in units: a word's box is
/// drawn as 0.8 em above the baseline and 0.2 em below it.
pub(crate) const ASCENT: i16 = 800;
pub(crate) const DESCENT: i16 = 200;
/// The program's PostScript name.
pub(crate) const NAME: &str = "MirsamGlyphLess";

/// The TrueType program: an empty `.notdef` and the glyph every code draws,
/// each [`ADVANCE`] wide.
pub(crate) fn program() -> Vec<u8> {
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"OS/2", os2()),
        (*b"cmap", cmap()),
        (*b"glyf", glyf()),
        (*b"head", head()),
        (*b"hhea", hhea()),
        (*b"hmtx", [ADVANCE, 0, ADVANCE, 0, 0, 0, 0, 0, 0, 0].iter().flat_map(|v| v.to_be_bytes()).collect()),
        // Short offsets (halved): .notdef is empty, then each rectangle in turn.
        (*b"loca", loca()),
        (*b"maxp", maxp()),
        (*b"name", name()),
        (*b"post", post()),
    ];
    tables.sort_by_key(|(tag, _)| *tag);

    let count = tables.len() as u16;
    let mut out = Vec::new();
    let entry_selector = 15 - count.leading_zeros() as u16; // floor(log2(count))
    let search_range = (1u16 << entry_selector) * 16;
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for value in [count, search_range, entry_selector, count * 16 - search_range] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    let mut head_at = 0;
    for (tag, data) in &tables {
        if tag == b"head" {
            head_at = offset;
        }
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables.len() + body.len();
    }
    out.extend_from_slice(&body);
    // head.checkSumAdjustment makes the whole font sum to 0xB1B0AFBA.
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    out[head_at + 8..head_at + 12].copy_from_slice(&adjustment.to_be_bytes());
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn be(values: &[i32], widths: &[u8]) -> Vec<u8> {
    values
        .iter()
        .zip(widths)
        .flat_map(|(&value, &width)| match width {
            2 => (value as i16).to_be_bytes().to_vec(),
            4 => value.to_be_bytes().to_vec(),
            _ => vec![0; usize::from(width)],
        })
        .collect()
}

fn head() -> Vec<u8> {
    let mut out = be(&[0x0001_0000, 0x0001_0000, 0, 0x5F0F_3CF5], &[4, 4, 4, 4]);
    out.extend(be(&[0b1011, i32::from(UNITS_PER_EM)], &[2, 2]));
    out.extend([0; 16]); // created, modified
    out.extend(be(&[0, -i32::from(DESCENT), i32::from(ADVANCE), i32::from(ASCENT), 0, 3, 2, 0, 0], &[2; 9]));
    out
}

/// Glyph 1's outline: the rectangle of its advance from descent to ascent
/// (x0, y0, x1, y1).
const LETTER: [i32; 4] = [0, -(DESCENT as i32), ADVANCE as i32, ASCENT as i32];
/// Glyphs 2–4 (marks): small rectangles over the letter, a different one for
/// shadda (3) and hamza (4), so a mark never covers its letter, nor a mark
/// stacked with it (shadda and a vowel), exactly. Preview drops a glyph that
/// exactly overlaps another, as it does for fake bold.
const MARKS_DRAWN: [[i32; 4]; 3] = [[100, 600, 400, 800], [50, 650, 450, 750], [150, 550, 350, 700]];

fn glyf() -> Vec<u8> {
    let mut out = rectangle(LETTER);
    for mark in MARKS_DRAWN {
        out.extend(rectangle(mark));
    }
    out
}

fn loca() -> Vec<u8> {
    let mut offsets = vec![0, 0];
    let mut end = rectangle(LETTER).len() as i32;
    offsets.push(end / 2);
    for mark in MARKS_DRAWN {
        end += rectangle(mark).len() as i32;
        offsets.push(end / 2);
    }
    be(&offsets, &[2; 5])
}

/// A simple glyph of one closed contour: the rectangle `[x0, y0, x1, y1]`.
fn rectangle([x0, y0, x1, y1]: [i32; 4]) -> Vec<u8> {
    let mut out = be(&[1, x0, y0, x1, y1, 3, 0], &[2; 7]);
    out.extend([1u8; 4]); // four on-curve points, coordinates as 16-bit deltas
    out.extend(be(&[x0, x1 - x0, 0, x0 - x1], &[2; 4]));
    out.extend(be(&[y0, 0, y1 - y0, 0], &[2; 4]));
    out.extend([0, 0]); // to an even length, as short offsets need
    out
}

fn hhea() -> Vec<u8> {
    let mut out = be(&[0x0001_0000], &[4]);
    out.extend(be(
        &[i32::from(ASCENT), -i32::from(DESCENT), 0, i32::from(ADVANCE), 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2],
        &[2; 16],
    ));
    out
}

fn maxp() -> Vec<u8> {
    let mut out = be(&[0x0001_0000], &[4]);
    out.extend(be(&[5, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0], &[2; 14]));
    out
}

fn cmap() -> Vec<u8> {
    // One format 4 subtable for Windows Unicode with only the closing segment:
    // no character has a glyph of its own (codes reach glyphs through the PDF's
    // /CIDToGIDMap, not this table).
    let mut out = be(&[0, 1, 3, 1, 12], &[2, 2, 2, 2, 4]);
    out.extend(be(&[4, 24, 0, 2, 2, 0, 0, 0xFFFF, 0, 0xFFFF, 1, 0], &[2; 12]));
    out
}

fn name() -> Vec<u8> {
    let text: Vec<u8> = NAME.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let length = text.len() as i32;
    // Family (1), full name (4) and PostScript name (6), all the same string.
    let ids = [1, 4, 6];
    let mut out = be(&[0, ids.len() as i32, 6 + 12 * ids.len() as i32], &[2, 2, 2]);
    for id in ids {
        out.extend(be(&[3, 1, 0x409, id, length, 0], &[2; 6]));
    }
    out.extend(text);
    out
}

fn post() -> Vec<u8> {
    be(&[0x0003_0000, 0, -100, 50, 1, 0, 0, 0, 0], &[4, 4, 2, 2, 4, 4, 4, 4, 4])
}

fn os2() -> Vec<u8> {
    // Version 4, 96 bytes; installable (fsType 0), regular weight and width.
    let mut out = be(&[4, i32::from(ADVANCE), 400, 5, 0], &[2; 5]);
    out.extend([0; 16]); // subscript/superscript sizes and offsets
    out.extend(be(&[0, 0, 0], &[2, 2, 2])); // strikeout size, position; family class
    out.extend([0; 10]); // PANOSE
    out.extend([0; 16]); // Unicode ranges
    out.extend(*b"NONE"); // vendor
    out.extend(be(&[0x40, 0x20, 0xFFFF], &[2, 2, 2])); // regular; first and last char
    out.extend(be(
        &[i32::from(ASCENT), -i32::from(DESCENT), 0, i32::from(ASCENT), i32::from(DESCENT)],
        &[2; 5],
    ));
    out.extend([0; 8]); // code page ranges
    out.extend(be(&[0, 0, 0, 0x20, 0], &[2; 5])); // x/cap height, default and break char, max context
    out
}

/// The `/ToUnicode` CMap: every two-byte code is the UTF-16 code unit it
/// names. One `bfrange` per high byte, since a range may only step its last byte.
pub(crate) fn to_unicode() -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    for block in (0..256u32).collect::<Vec<_>>().chunks(100) {
        cmap.push_str(&format!("{} beginbfrange\n", block.len()));
        for high in block {
            cmap.push_str(&format!("<{high:02X}00> <{high:02X}FF> <{high:02X}00>\n"));
        }
        cmap.push_str("endbfrange\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end\n");
    cmap
}

/// Combining marks that sit on the letter before them (tashkeel, Quranic
/// marks): zero advance, so a mark is drawn where its letter is and readers
/// attach it there. As UTF-16 code-unit ranges (the CIDs).
pub(crate) const MARKS: &[(u16, u16)] = &[
    (0x0300, 0x036F),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0670, 0x0670),
    (0x06D6, 0x06DC),
    (0x06DF, 0x06E4),
    (0x06E7, 0x06E8),
    (0x06EA, 0x06ED),
    (0x08D3, 0x08E1),
    (0x08E3, 0x08FF),
];

/// Whether a UTF-16 code unit draws with no advance ([`MARKS`]).
pub(crate) fn is_mark(unit: u16) -> bool {
    MARKS.iter().any(|&(first, last)| (first..=last).contains(&unit))
}

/// The `/CIDToGIDMap` stream's data: shadda to glyph 3, hamza to 4, other
/// marks to 2, every other CID to glyph 1.
pub(crate) fn cid_to_gid() -> Vec<u8> {
    (0..=u16::MAX)
        .flat_map(|cid| match cid {
            0x0651 => [0, 3],
            0x0654 | 0x0655 => [0, 4],
            _ if is_mark(cid) => [0, 2],
            _ => [0, 1],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_a_well_formed_font() {
        let data = program();
        let font = read_fonts::FontRef::new(&data).expect("parses");
        use read_fonts::TableProvider as _;
        assert_eq!(font.head().expect("head").units_per_em(), UNITS_PER_EM);
        assert_eq!(font.maxp().expect("maxp").num_glyphs(), 5);
        let metrics = font.hmtx().expect("hmtx");
        assert_eq!(metrics.h_metrics()[1].advance(), ADVANCE);
        assert_eq!(checksum(&data), 0xB1B0_AFBA, "whole-font checksum");
        assert!(data.len() < 1024);
    }

    #[test]
    fn the_cmap_is_the_identity_in_256_ranges() {
        let cmap = to_unicode();
        assert!(cmap.contains("<0600> <06FF> <0600>"));
        assert!(cmap.contains("<FF00> <FFFF> <FF00>"));
        assert_eq!(cmap.matches("beginbfrange").count(), 3);
        assert_eq!(cid_to_gid().len(), 131_072);
    }
}
