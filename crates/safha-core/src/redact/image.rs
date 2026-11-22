//! Overwriting the pixels of an image that fall in a redaction area
//! (ADR 0042).
//!
//! Done for images stored uncompressed or with Flate, 1 to 8 bits per
//! sample: the samples are decoded, the covered ones are overwritten, and the
//! image is written again as plain Flate. Any other encoding (JPEG, JPEG
//! 2000, CCITT, JBIG2, 16 bits per sample) cannot be edited without a codec
//! this project has not adopted, so such an image is dropped whole rather than
//! left holding what was meant to go.

use super::interp::{Area, Matrix, apply};
use crate::pdf::{Dict, File, Object, decode_stream};

pub(crate) enum ImageOutcome {
    /// Nothing of the image is in an area.
    Untouched,
    /// The image cannot be edited; it must not be drawn.
    Dropped,
    /// The image with covered pixels overwritten: its dictionary and data.
    Blanked(Dict, Vec<u8>),
}

/// The most pixels an image may have to be edited.
const MAX_PIXELS: u64 = 400_000_000;

pub(crate) fn redact_image(
    file: &File<'_>,
    dict: &Dict,
    raw: &[u8],
    ctm: &Matrix,
    areas: &[Area],
) -> ImageOutcome {
    edit(file, dict, raw, ctm, areas).unwrap_or(ImageOutcome::Dropped)
}

fn int(file: &File<'_>, dict: &Dict, key: &[u8]) -> Option<i64> {
    file.resolve(dict.get(key)?)?.as_int()
}

fn edit(file: &File<'_>, dict: &Dict, raw: &[u8], ctm: &Matrix, areas: &[Area]) -> Option<ImageOutcome> {
    let width = u64::try_from(int(file, dict, b"Width")?).ok().filter(|w| *w > 0)?;
    let height = u64::try_from(int(file, dict, b"Height")?).ok().filter(|h| *h > 0)?;
    if width.checked_mul(height)? > MAX_PIXELS {
        return None;
    }
    let mask = dict.get(b"ImageMask").and_then(|m| file.resolve(m)) == Some(Object::Bool(true));
    let bpc = if mask { 1 } else { u64::try_from(int(file, dict, b"BitsPerComponent")?).ok()? };
    if !matches!(bpc, 1 | 2 | 4 | 8) {
        return None;
    }
    let (components, black) = if mask { (1, vec![0]) } else { colour(file, dict.get(b"ColorSpace")?, bpc)? };

    // Where each pixel centre lands on the page: unit square to the image.
    // The inverse maps an area's corners into pixel space to bound the scan.
    let det = ctm[0] * ctm[3] - ctm[1] * ctm[2];
    if det.abs() < 1e-12 {
        return None;
    }
    let inverse = [
        ctm[3] / det,
        -ctm[1] / det,
        -ctm[2] / det,
        ctm[0] / det,
        (ctm[2] * ctm[5] - ctm[3] * ctm[4]) / det,
        (ctm[1] * ctm[4] - ctm[0] * ctm[5]) / det,
    ];
    let (w, h) = (width as f64, height as f64);

    let data = decode_stream(dict, raw, file)?;
    let row_bytes = usize::try_from((width * components * bpc).div_ceil(8)).ok()?;
    let rows = usize::try_from(height).ok()?;
    if data.len() < row_bytes.checked_mul(rows)? {
        return None;
    }
    let mut data = data;
    let mut changed = false;
    for area in areas {
        // The area in unit-square coordinates, as a bounding box.
        let corners = [(area[0], area[1]), (area[2], area[1]), (area[2], area[3]), (area[0], area[3])]
            .map(|p| apply(&inverse, p));
        let (u0, u1) = (
            corners.iter().map(|c| c.0).fold(f64::MAX, f64::min),
            corners.iter().map(|c| c.0).fold(f64::MIN, f64::max),
        );
        let (v0, v1) = (
            corners.iter().map(|c| c.1).fold(f64::MAX, f64::min),
            corners.iter().map(|c| c.1).fold(f64::MIN, f64::max),
        );
        if u1 < 0.0 || v1 < 0.0 || u0 > 1.0 || v0 > 1.0 {
            continue;
        }
        // Pixel columns/rows (row 0 is the top: v = 1).
        let col0 = ((u0.max(0.0) * w).floor() as u64).min(width - 1);
        let col1 = ((u1.min(1.0) * w).ceil() as u64).min(width);
        let row0 = (((1.0 - v1.min(1.0)) * h).floor() as u64).min(height - 1);
        let row1 = (((1.0 - v0.max(0.0)) * h).ceil() as u64).min(height);
        for row in row0..row1 {
            for col in col0..col1 {
                let centre = ((col as f64 + 0.5) / w, 1.0 - (row as f64 + 0.5) / h);
                let (x, y) = apply(ctm, centre);
                // Half a pixel of margin: a pixel the area only touches may
                // still show a part of what it covers.
                let margin = 0.5 * (1.0 / w * ctm[0].hypot(ctm[1])).max(1.0 / h * ctm[2].hypot(ctm[3]));
                if (area[0] - margin..=area[2] + margin).contains(&x)
                    && (area[1] - margin..=area[3] + margin).contains(&y)
                {
                    overwrite(&mut data, row, col, row_bytes, components, bpc, &black);
                    changed = true;
                }
            }
        }
    }
    if !changed {
        return Some(ImageOutcome::Untouched);
    }
    let mut new_dict = dict.clone();
    new_dict.0.retain(|(k, _)| !matches!(k.as_slice(), b"Filter" | b"DecodeParms" | b"Length"));
    new_dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
    Some(ImageOutcome::Blanked(new_dict, miniz_oxide::deflate::compress_to_vec_zlib(&data, 6)))
}

/// Writes `value` (one sample per component) into the pixel at (`row`, `col`).
fn overwrite(
    data: &mut [u8],
    row: u64,
    col: u64,
    row_bytes: usize,
    components: u64,
    bpc: u64,
    value: &[u16],
) {
    let first = (col * components * bpc) as usize;
    for (k, sample) in value.iter().take(components as usize).enumerate() {
        let bit = first + k * bpc as usize;
        let at = row as usize * row_bytes + bit / 8;
        if bpc == 8 {
            data[at] = *sample as u8;
        } else {
            let shift = 8 - bpc as usize - bit % 8;
            let mask = ((1u16 << bpc) - 1) as u8;
            data[at] = data[at] & !(mask << shift) | ((*sample as u8 & mask) << shift);
        }
    }
}

/// The number of components of a colour space, and the sample values that
/// make black (or the nearest the space has).
fn colour(file: &File<'_>, space: &Object, bpc: u64) -> Option<(u64, Vec<u16>)> {
    let space = file.resolve(space)?;
    let name = |n: &[u8]| -> Option<(u64, Vec<u16>)> {
        match n {
            b"DeviceGray" | b"G" | b"CalGray" => Some((1, vec![0])),
            b"DeviceRGB" | b"RGB" | b"CalRGB" | b"Lab" => Some((3, vec![0, 0, 0])),
            b"DeviceCMYK" | b"CMYK" => Some((4, vec![0, 0, 0, (1u16 << bpc) - 1])),
            _ => None,
        }
    };
    match &space {
        Object::Name(n) => name(n),
        Object::Array(items) => match items.first()?.as_name()? {
            b"ICCBased" => {
                let profile = file.resolve_shallow(items.get(1)?)?;
                let n = u64::try_from(profile.as_dict()?.get(b"N")?.as_int()?).ok()?;
                match n {
                    1 => Some((1, vec![0])),
                    3 => Some((3, vec![0, 0, 0])),
                    4 => Some((4, vec![0, 0, 0, (1u16 << bpc) - 1])),
                    _ => None,
                }
            }
            b"Indexed" | b"I" => {
                // The darkest palette entry.
                let base = colour_of_base(file, items.get(1)?)?;
                let hival = usize::try_from(items.get(2)?.as_int()?).ok()?;
                let lookup = match file.resolve(items.get(3)?)? {
                    Object::Str(s) => s,
                    _ => return Some((1, vec![0])),
                };
                let n = usize::try_from(base).ok()?;
                let darkest = (0..=hival.min(255))
                    .filter(|i| lookup.len() >= (i + 1) * n)
                    .min_by_key(|i| {
                        let entry = &lookup[i * n..(i + 1) * n];
                        // CMYK: darkness is ink; others: brightness.
                        if n == 4 {
                            -entry.iter().map(|v| i32::from(*v)).sum::<i32>()
                        } else {
                            entry.iter().map(|v| i32::from(*v)).sum::<i32>()
                        }
                    })
                    .unwrap_or(0);
                Some((1, vec![u16::try_from(darkest).ok()?]))
            }
            b"CalGray" => Some((1, vec![0])),
            b"CalRGB" | b"Lab" => Some((3, vec![0, 0, 0])),
            b"Separation" => Some((1, vec![(1u16 << bpc) - 1])),
            _ => None,
        },
        _ => None,
    }
}

fn colour_of_base(file: &File<'_>, base: &Object) -> Option<u64> {
    let (n, _) = colour(file, base, 8)?;
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_written_at_every_depth() {
        // Two pixels of 3 components at 8 bits.
        let mut data = vec![255u8; 6];
        overwrite(&mut data, 0, 1, 6, 3, 8, &[0, 0, 0]);
        assert_eq!(data, [255, 255, 255, 0, 0, 0]);
        // 1 bit: eight pixels in a byte.
        let mut bits = vec![0xFFu8];
        overwrite(&mut bits, 0, 2, 1, 1, 1, &[0]);
        assert_eq!(bits, [0b1101_1111]);
        // 4 bits: two pixels per byte.
        let mut nibbles = vec![0xFFu8];
        overwrite(&mut nibbles, 0, 1, 1, 1, 4, &[0x3]);
        assert_eq!(nibbles, [0xF3]);
    }
}
