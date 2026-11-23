//! What redaction needs to know of a font: how wide each character code is
//! and how many bytes a code takes, so a string can be cut into glyphs and
//! each glyph placed (ADR 0042). Nothing here draws anything.

use std::collections::HashMap;

use crate::pdf::{Dict, File, Object, decode_stream};

/// A font's metrics as far as redaction goes.
#[derive(Debug, Clone)]
pub(crate) struct FontInfo {
    /// Bytes per character code: 1 for simple fonts, 2 (or 1) for composite.
    pub(crate) code_bytes: usize,
    widths: HashMap<u32, f64>,
    default_width: f64,
    /// Glyph-space units per text-space unit: 1000 for most fonts; for a
    /// Type 3 font, the reciprocal of its font matrix's horizontal scale.
    pub(crate) per_em: f64,
    /// Ascent and descent, in thousandths of an em.
    pub(crate) ascent: f64,
    pub(crate) descent: f64,
    /// Type 3 glyphs' declared ink boxes (`d1`), as fractions of an em
    /// (left, bottom, right, top) in text space.
    ink: HashMap<u32, [f64; 4]>,
}

impl FontInfo {
    /// Used when a font cannot be read: one byte per code, half an em wide.
    pub(crate) fn fallback() -> Self {
        Self {
            code_bytes: 1,
            widths: HashMap::new(),
            default_width: 500.0,
            per_em: 1000.0,
            ascent: 800.0,
            descent: -200.0,
            ink: HashMap::new(),
        }
    }

    /// The advance of character code `code`, in thousandths of an em.
    pub(crate) fn width(&self, code: u32) -> f64 {
        self.widths.get(&code).copied().unwrap_or(self.default_width) * 1000.0 / self.per_em
    }

    /// The ink box a Type 3 glyph declares, as fractions of an em.
    pub(crate) fn ink(&self, code: u32) -> Option<[f64; 4]> {
        self.ink.get(&code).copied()
    }

    pub(crate) fn read(file: &File<'_>, font: &Object) -> Self {
        Self::try_read(file, font).unwrap_or_else(Self::fallback)
    }

    fn try_read(file: &File<'_>, font: &Object) -> Option<Self> {
        let dict = file.resolve(font)?.as_dict()?.clone();
        let subtype = dict.get(b"Subtype").and_then(Object::as_name).unwrap_or(b"");
        let mut info = Self::fallback();
        if subtype == b"Type0" {
            info.code_bytes = 2;
            if let Some(encoding @ Object::Ref(..)) = dict.get(b"Encoding")
                && let Some(Object::Dict(stream_dict)) = file.resolve_shallow(encoding)
                && let Some(bytes) = cmap_code_bytes(file, encoding, &stream_dict)
            {
                info.code_bytes = bytes;
            }
            let descendant = dict
                .get(b"DescendantFonts")
                .and_then(|d| file.resolve(d))
                .and_then(|d| d.as_array().and_then(|a| a.first().cloned()))
                .and_then(|d| file.resolve(&d))?;
            let descendant = descendant.as_dict()?;
            info.default_width = number(file, descendant.get(b"DW")).unwrap_or(1000.0);
            if let Some(w) = descendant.get(b"W").and_then(|w| file.resolve(w)) {
                read_cid_widths(file, w.as_array()?, &mut info.widths);
            }
            info.read_descriptor(file, descendant);
            return Some(info);
        }

        info.read_descriptor(file, &dict);
        if subtype == b"Type3" {
            let scale = dict
                .get(b"FontMatrix")
                .and_then(|m| file.resolve(m))
                .and_then(|m| m.as_array().and_then(|a| a.first().and_then(|x| number(file, Some(x)))))
                .filter(|a| *a != 0.0)
                .unwrap_or(0.001);
            info.per_em = 1.0 / scale.abs();
            info.default_width = 0.0;
            info.read_glyph_boxes(file, &dict);
        }
        let first = number(file, dict.get(b"FirstChar")).unwrap_or(0.0) as i64;
        if let Some(widths) = dict.get(b"Widths").and_then(|w| file.resolve(w))
            && let Some(widths) = widths.as_array()
        {
            for (i, width) in widths.iter().enumerate() {
                if let Some(width) = number(file, Some(width)) {
                    let code = first + i64::try_from(i).ok()?;
                    if let Ok(code) = u32::try_from(code) {
                        info.widths.insert(code, width);
                    }
                }
            }
        } else if subtype != b"Type3" {
            standard_widths(dict.get(b"BaseFont").and_then(Object::as_name).unwrap_or(b""), &mut info);
        }
        Some(info)
    }

    /// The `d1` boxes of a Type 3 font's glyph procedures, by character code.
    fn read_glyph_boxes(&mut self, file: &File<'_>, dict: &Dict) {
        let matrix: Vec<f64> = dict
            .get(b"FontMatrix")
            .and_then(|m| file.resolve(m))
            .and_then(|m| m.as_array().map(<[Object]>::to_vec))
            .map(|m| m.iter().filter_map(|x| number(file, Some(x))).collect())
            .unwrap_or_default();
        let [a, b, c, d, ..] = matrix.as_slice() else { return };
        let Some(procs) = dict.get(b"CharProcs").and_then(|p| file.resolve(p)) else { return };
        let Some(procs) = procs.as_dict() else { return };
        let Some(encoding) = dict.get(b"Encoding").and_then(|e| file.resolve(e)) else { return };
        let Some(differences) =
            encoding.as_dict().and_then(|e| e.get(b"Differences")).and_then(|d| file.resolve(d))
        else {
            return;
        };
        let Some(differences) = differences.as_array() else { return };
        let mut code = 0i64;
        for item in differences {
            match item {
                Object::Name(name) => {
                    if let (Some(proc), Ok(code)) = (procs.get(name), u32::try_from(code))
                        && let Some(bbox) = glyph_box(file, proc)
                    {
                        // Glyph space to text space through the font matrix.
                        let corners =
                            [(bbox[0], bbox[1]), (bbox[2], bbox[1]), (bbox[2], bbox[3]), (bbox[0], bbox[3])]
                                .map(|(x, y)| (x * a + y * c, x * b + y * d));
                        let xs = corners.map(|p| p.0);
                        let ys = corners.map(|p| p.1);
                        self.ink.insert(
                            code,
                            [
                                xs.iter().copied().fold(f64::MAX, f64::min),
                                ys.iter().copied().fold(f64::MAX, f64::min),
                                xs.iter().copied().fold(f64::MIN, f64::max),
                                ys.iter().copied().fold(f64::MIN, f64::max),
                            ],
                        );
                    }
                    code += 1;
                }
                other => {
                    if let Some(n) = other.as_int() {
                        code = n;
                    }
                }
            }
        }
    }

    fn read_descriptor(&mut self, file: &File<'_>, dict: &Dict) {
        let Some(descriptor) = dict.get(b"FontDescriptor").and_then(|d| file.resolve(d)) else { return };
        let Some(descriptor) = descriptor.as_dict() else { return };
        if let Some(missing) = number(file, descriptor.get(b"MissingWidth")) {
            self.default_width = missing;
        }
        // Boxes taller than any sane line would reach into the neighbours.
        let ascent = number(file, descriptor.get(b"Ascent")).filter(|a| *a > 0.0);
        let descent = number(file, descriptor.get(b"Descent")).filter(|d| *d < 0.0);
        if let Some(ascent) = ascent {
            self.ascent = ascent.clamp(600.0, 950.0);
        }
        if let Some(descent) = descent {
            self.descent = descent.clamp(-300.0, -100.0);
        }
    }
}

/// The box a glyph procedure declares with `d1`: `wx wy llx lly urx ury d1`.
fn glyph_box(file: &File<'_>, proc: &Object) -> Option<[f64; 4]> {
    let Object::Ref(number, _) = proc else { return None };
    let crate::pdf::Entry::Offset(offset) = *file.entries.get(number)? else { return None };
    let (dict, data) = file.indirect_at(offset)?;
    let data = decode_stream(&dict, data?, file)?;
    let ops = super::content::tokenize(&data[..data.len().min(4096)])?;
    let d1 = ops.iter().find(|op| op.is(b"d1"))?;
    Some([d1.number(2)?, d1.number(3)?, d1.number(4)?, d1.number(5)?])
}

fn number(file: &File<'_>, object: Option<&Object>) -> Option<f64> {
    match file.resolve(object?)? {
        Object::Int(i) => Some(i as f64),
        Object::Real(r) if r.is_finite() => Some(r),
        _ => None,
    }
}

/// `/W`: `c [w1 w2 …]` or `c_first c_last w`.
fn read_cid_widths(file: &File<'_>, array: &[Object], out: &mut HashMap<u32, f64>) {
    let mut i = 0;
    while i < array.len() {
        let Some(first) = number(file, array.get(i)).and_then(|n| u32::try_from(n as i64).ok()) else {
            return;
        };
        match array.get(i + 1).and_then(|next| file.resolve(next)) {
            Some(Object::Array(widths)) => {
                for (k, width) in widths.iter().enumerate() {
                    if let (Some(width), Ok(k)) = (number(file, Some(width)), u32::try_from(k)) {
                        out.insert(first.saturating_add(k), width);
                    }
                }
                i += 2;
            }
            Some(_) => {
                let last = number(file, array.get(i + 1)).and_then(|n| u32::try_from(n as i64).ok());
                let width = number(file, array.get(i + 2));
                if let (Some(last), Some(width)) = (last, width) {
                    // A range of a few thousand at most: a hostile file could
                    // claim four billion.
                    for code in first..=last.min(first.saturating_add(70_000)) {
                        out.insert(code, width);
                    }
                }
                i += 3;
            }
            None => return,
        }
    }
}

/// Bytes per code of an embedded CMap: the width of its first codespace
/// range. Predefined and identity CMaps are two bytes.
fn cmap_code_bytes(file: &File<'_>, reference: &Object, dict: &Dict) -> Option<usize> {
    let Object::Ref(number, _) = reference else { return None };
    let crate::pdf::Entry::Offset(offset) = *file.entries.get(number)? else { return None };
    let (_, data) = file.indirect_at(offset)?;
    let data = decode_stream(dict, data?, file)?;
    let text = String::from_utf8_lossy(&data);
    let at = text.find("begincodespacerange")?;
    let rest = &text[at + "begincodespacerange".len()..];
    let open = rest.find('<')?;
    let close = rest[open..].find('>')? + open;
    let digits = rest[open + 1..close].chars().filter(char::is_ascii_hexdigit).count();
    Some((digits / 2).clamp(1, 2))
}

/// Widths of the standard 14 fonts for fonts that name one and give none:
/// Courier is 600 throughout; the rest follow Helvetica's.
fn standard_widths(base_font: &[u8], info: &mut FontInfo) {
    let name = String::from_utf8_lossy(base_font).to_ascii_lowercase();
    if name.contains("courier") {
        info.default_width = 600.0;
        return;
    }
    for (i, width) in HELVETICA.iter().enumerate() {
        info.widths.insert(32 + i as u32, f64::from(*width));
    }
    info.default_width = 556.0;
}

/// Helvetica's advance widths for the characters 32 to 126.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // space - /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, 278, 584, 584, 584, 556, 1015, // : - @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722,
    667, 944, 667, 667, 611, // A-Z
    278, 278, 278, 469, 556, 333, // [ - `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556,
    500, 722, 500, 500, 500, // a-z
    334, 260, 334, 584, // { - ~
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helvetica_table_covers_the_printable_ascii_range() {
        assert_eq!(HELVETICA.len(), 95);
        let mut info = FontInfo::fallback();
        standard_widths(b"Helvetica", &mut info);
        assert_eq!(info.width(u32::from(b'i')), 222.0);
        assert_eq!(info.width(u32::from(b'W')), 944.0);
        assert_eq!(info.width(u32::from(b'~')), 584.0);
    }
}
