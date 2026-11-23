//! A baseline JPEG encoder (ADR 0050), written for page export.
//!
//! Why our own: the codecs on the market are either release candidates or carry
//! the IJG licence, which the spec's allowlist (§10) does not list (owner item
//! 15 in `PROGRESS.md`). Encoding is small: 8×8 DCT, the standard quantization
//! and Huffman tables (ITU T.81 Annex K), 4:4:4 sampling (no chroma subsampling,
//! so Arabic's thin dots and strokes keep their colour edges). Only baseline
//! sequential, 8-bit, Huffman coding is written; any decoder reads it.

use std::f32::consts::PI;

/// Annex K.1 luminance quantization table, zig-zag order.
const LUMA: [u8; 64] = [
    16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49, 35, 37, 29, 40,
    58, 51, 61, 60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56, 80, 109, 81, 87, 95, 98, 103,
    104, 103, 62, 77, 113, 121, 112, 100, 120, 92, 101, 103, 99,
];
/// Annex K.1 chrominance table, zig-zag order.
const CHROMA: [u8; 64] = [
    17, 18, 18, 24, 21, 24, 47, 26, 26, 47, 99, 66, 56, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7,
    14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39,
    46, 53, 60, 61, 54, 47, 55, 62, 63,
];

// Annex K.3 Huffman tables: code counts per length, then symbols.
const DC_LUMA_COUNTS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_COUNTS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_SYMBOLS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_COUNTS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUMA_SYMBOLS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22,
    0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33,
    0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34,
    0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55,
    0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76,
    0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5,
    0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4,
    0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1,
    0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];
const AC_CHROMA_COUNTS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_SYMBOLS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13,
    0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62,
    0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29,
    0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54,
    0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3,
    0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2,
    0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea,
    0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

/// A Huffman table as the encoder uses it: (code, length) per symbol.
struct Table {
    codes: [(u16, u8); 256],
}

impl Table {
    fn new(counts: &[u8; 16], symbols: &[u8]) -> Self {
        let mut codes = [(0u16, 0u8); 256];
        let (mut code, mut at) = (0u16, 0usize);
        for (length, &count) in counts.iter().enumerate() {
            for _ in 0..count {
                codes[usize::from(symbols[at])] = (code, length as u8 + 1);
                code += 1;
                at += 1;
            }
            code <<= 1;
        }
        Self { codes }
    }
}

/// Packs bits, stuffing a zero byte after every 0xFF.
struct Bits {
    out: Vec<u8>,
    acc: u32,
    used: u8,
}

impl Bits {
    fn put(&mut self, code: u16, length: u8) {
        if length == 0 {
            return;
        }
        self.acc = (self.acc << length) | (u32::from(code) & ((1u32 << length) - 1));
        self.used += length;
        while self.used >= 8 {
            let byte = (self.acc >> (self.used - 8)) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
            self.used -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            let pad = 8 - self.used;
            self.put((1u16 << pad) - 1, pad);
        }
        self.out
    }
}

/// The scaled quantization table for quality 1–100 (the libjpeg convention).
fn quantization(base: &[u8; 64], quality: u8) -> [u8; 64] {
    let quality = u32::from(quality.clamp(1, 100));
    let scale = if quality < 50 { 5000 / quality } else { 200 - quality * 2 };
    base.map(|q| ((u32::from(q) * scale + 50) / 100).clamp(1, 255) as u8)
}

/// Bits needed for `value`'s magnitude, and the bits to write for it.
fn category(value: i32) -> (u8, u16) {
    let magnitude = value.unsigned_abs();
    let size = (32 - magnitude.leading_zeros()) as u8;
    let bits = if value < 0 { (value - 1) as u32 & ((1u32 << size) - 1) } else { value as u32 };
    (size, bits as u16)
}

/// The forward DCT of one 8×8 block (level-shifted), separable, direct form.
fn dct(block: &[f32; 64], cosines: &[[f32; 8]; 8]) -> [f32; 64] {
    let mut rows = [0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            let mut sum = 0.0;
            for x in 0..8 {
                sum += block[y * 8 + x] * cosines[u][x];
            }
            rows[y * 8 + u] = sum;
        }
    }
    let mut out = [0f32; 64];
    for u in 0..8 {
        for v in 0..8 {
            let mut sum = 0.0;
            for y in 0..8 {
                sum += rows[y * 8 + u] * cosines[v][y];
            }
            out[v * 8 + u] = sum / 4.0;
        }
    }
    out
}

/// Encodes 8-bit RGB (`width * height * 3` bytes, rows top to bottom) as a
/// baseline JPEG at `quality` (1–100). `None` when the size does not fit.
pub fn encode_rgb(rgb: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return None;
    }
    if rgb.len() != (width as usize).checked_mul(height as usize)?.checked_mul(3)? {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let quant = [quantization(&LUMA, quality), quantization(&CHROMA, quality)];
    let tables = [
        (Table::new(&DC_LUMA_COUNTS, &DC_SYMBOLS), Table::new(&AC_LUMA_COUNTS, &AC_LUMA_SYMBOLS)),
        (Table::new(&DC_CHROMA_COUNTS, &DC_SYMBOLS), Table::new(&AC_CHROMA_COUNTS, &AC_CHROMA_SYMBOLS)),
    ];
    let mut cosines = [[0f32; 8]; 8];
    for (u, row) in cosines.iter_mut().enumerate() {
        let scale = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
        for (x, value) in row.iter_mut().enumerate() {
            *value = scale * ((2 * x + 1) as f32 * u as f32 * PI / 16.0).cos();
        }
    }

    let mut out = vec![0xFF, 0xD8];
    // APP0 (JFIF), so every reader takes the data as YCbCr.
    out.extend_from_slice(&[0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]);
    // The tables are kept in zig-zag order, which is the order they are stored in.
    for (id, table) in quant.iter().enumerate() {
        out.extend_from_slice(&[0xFF, 0xDB, 0, 67, id as u8]);
        out.extend_from_slice(table);
    }
    out.extend_from_slice(&[0xFF, 0xC0, 0, 17, 8]);
    out.extend_from_slice(&(height as u16).to_be_bytes());
    out.extend_from_slice(&(width as u16).to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1]);
    for (class, id, counts, symbols) in [
        (0u8, 0u8, &DC_LUMA_COUNTS, &DC_SYMBOLS[..]),
        (1, 0, &AC_LUMA_COUNTS, &AC_LUMA_SYMBOLS[..]),
        (0, 1, &DC_CHROMA_COUNTS, &DC_SYMBOLS[..]),
        (1, 1, &AC_CHROMA_COUNTS, &AC_CHROMA_SYMBOLS[..]),
    ] {
        let length = 2 + 1 + 16 + symbols.len();
        out.extend_from_slice(&[0xFF, 0xC4, (length >> 8) as u8, length as u8, class << 4 | id]);
        out.extend_from_slice(counts);
        out.extend_from_slice(symbols);
    }
    out.extend_from_slice(&[0xFF, 0xDA, 0, 12, 3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);

    let mut bits = Bits { out: Vec::new(), acc: 0, used: 0 };
    let mut previous_dc = [0i32; 3];
    for by in (0..h).step_by(8) {
        for bx in (0..w).step_by(8) {
            let mut planes = [[0f32; 64]; 3];
            for y in 0..8 {
                for x in 0..8 {
                    // Edge blocks repeat the last row and column.
                    let (px, py) = ((bx + x).min(w - 1), (by + y).min(h - 1));
                    let at = (py * w + px) * 3;
                    let (r, g, b) = (f32::from(rgb[at]), f32::from(rgb[at + 1]), f32::from(rgb[at + 2]));
                    let i = y * 8 + x;
                    planes[0][i] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                    planes[1][i] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
                    planes[2][i] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
                }
            }
            for (component, plane) in planes.iter().enumerate() {
                let kind = usize::from(component > 0);
                let coefficients = dct(plane, &cosines);
                let mut quantized = [0i32; 64];
                for (k, &natural) in ZIGZAG.iter().enumerate() {
                    quantized[k] = (coefficients[natural] / f32::from(quant[kind][k])).round() as i32;
                }
                let (dc_table, ac_table) = (&tables[kind].0, &tables[kind].1);
                let difference = quantized[0] - previous_dc[component];
                previous_dc[component] = quantized[0];
                let (size, value) = category(difference);
                let (code, length) = dc_table.codes[usize::from(size)];
                bits.put(code, length);
                bits.put(value, size);
                let mut zeros = 0u8;
                for &coefficient in &quantized[1..] {
                    if coefficient == 0 {
                        zeros += 1;
                        continue;
                    }
                    while zeros >= 16 {
                        let (code, length) = ac_table.codes[0xF0];
                        bits.put(code, length);
                        zeros -= 16;
                    }
                    let (size, value) = category(coefficient);
                    let (code, length) = ac_table.codes[usize::from(zeros << 4 | size)];
                    bits.put(code, length);
                    bits.put(value, size);
                    zeros = 0;
                }
                if zeros > 0 {
                    let (code, length) = ac_table.codes[0x00];
                    bits.put(code, length);
                }
            }
        }
    }
    out.extend(bits.finish());
    out.extend_from_slice(&[0xFF, 0xD9]);
    Some(out)
}

/// The size and component count of a JPEG, read from its frame header (the
/// watermark embeds a JPEG as it is). `None` when it is not a baseline or
/// progressive Huffman JPEG of 1 or 3 components at 8 bits.
pub fn frame(data: &[u8]) -> Option<(u32, u32, u8)> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            at += 1;
            continue;
        }
        let marker = data[at + 1];
        if marker == 0xFF {
            at += 1;
            continue;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            at += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
        if matches!(marker, 0xC0..=0xC2) {
            let body = data.get(at + 4..at + 2 + length)?;
            let precision = *body.first()?;
            let height = u32::from(u16::from_be_bytes([*body.get(1)?, *body.get(2)?]));
            let width = u32::from(u16::from_be_bytes([*body.get(3)?, *body.get(4)?]));
            let components = *body.get(5)?;
            return (precision == 8 && matches!(components, 1 | 3) && width > 0 && height > 0)
                .then_some((width, height, components));
        }
        at += 2 + length;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_codes_match_the_standard() {
        assert_eq!(category(0), (0, 0));
        assert_eq!(category(1), (1, 1));
        assert_eq!(category(-1), (1, 0));
        assert_eq!(category(5), (3, 5));
        assert_eq!(category(-5), (3, 2));
    }

    #[test]
    fn the_standard_tables_build_known_codes() {
        // Annex K: luminance DC symbol 0 is `00`, symbol 1 is `010`.
        let table = Table::new(&DC_LUMA_COUNTS, &DC_SYMBOLS);
        assert_eq!(table.codes[0], (0b00, 2));
        assert_eq!(table.codes[1], (0b010, 3));
    }

    #[test]
    fn output_has_the_markers_of_a_baseline_file() {
        let rgb: Vec<u8> = (0..20 * 12 * 3).map(|i| (i * 7 % 251) as u8).collect();
        let jpeg = encode_rgb(&rgb, 20, 12, 85).expect("encoded");
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9]);
        assert_eq!(frame(&jpeg), Some((20, 12, 3)));
        assert!(encode_rgb(&rgb, 20, 13, 85).is_none());
    }
}
