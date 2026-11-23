//! Compression (M6, ADR 0050): the document written again, smaller.
//!
//! Our own writer, not PDFium's save (which keeps every object it read and
//! writes no object streams). The file is rebuilt from its newest revision:
//!
//! 1. **Unused objects go.** Only what the catalog and the document
//!    information reach is written, so earlier revisions, deleted pages, and
//!    orphaned resources are left behind. Objects are numbered again from 1.
//! 2. **Images are downsampled** to a target resolution, worked out from how
//!    large each image is *drawn*: page content and form XObjects are walked
//!    with their transformation matrices, and an image is reduced only when
//!    every placement shows it at more than 1.5× the target. Supported: 8-bit
//!    Gray, RGB, ICC-based (1 or 3 components), unfiltered or Flate. The
//!    pixels are averaged over each new pixel's area and written as Flate, or
//!    as JPEG for RGB when asked. JPEG and other images are kept as they are
//!    (a JPEG decoder is owner item 15), and so are images that are not found
//!    on a page (in annotations, or behind unreadable content).
//! 3. **Uncompressed streams are Flate-compressed.**
//! 4. **Object streams.** Objects other than streams are packed, a hundred at
//!    a time, into compressed object streams, with a cross-reference stream
//!    (PDF 1.5).
//!
//! Encrypted files are refused (this reader does not decrypt), and the shell
//! refuses signed ones, whose signatures a rewrite breaks.

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;

use serde::{Deserialize, Serialize};

use crate::edit::PageTree;
use crate::error::{Error, Result};
use crate::pdf::content::{Operand, tokenize};
use crate::pdf::write::{dict as write_dict, object as write_object};
use crate::pdf::{Dict, Entry, File, Object, decode_stream};

/// At most this many objects are written.
const MAX_OBJECTS: usize = 2_000_000;
/// Objects per object stream.
const PER_OBJECT_STREAM: usize = 100;
/// Form XObjects nest at most this deep when images are measured.
const MAX_FORM_DEPTH: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressOptions {
    /// Reduce images drawn at more than this resolution (dots per inch);
    /// `None` keeps every image as it is.
    pub image_dpi: Option<u32>,
    /// Write reduced colour images as JPEG at this quality (1–100) instead of
    /// Flate (lossless).
    pub jpeg_quality: Option<u8>,
    /// Pack objects into object streams.
    pub object_streams: bool,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self { image_dpi: Some(150), jpeg_quality: None, object_streams: true }
    }
}

/// What compressing did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressReport {
    pub before: u64,
    pub after: u64,
    pub images_reduced: usize,
    /// Images drawn at more than the target that could not be reduced.
    pub images_kept: usize,
    pub streams_compressed: usize,
    /// Objects of the newest revision that nothing reaches.
    pub objects_removed: usize,
}

/// `bytes` written again as [`CompressOptions`] say.
pub fn compress(bytes: &[u8], options: CompressOptions) -> Result<(Vec<u8>, CompressReport)> {
    if let Some(dpi) = options.image_dpi
        && !(36..=1200).contains(&dpi)
    {
        return Err(Error::InvalidEdit("images are reduced to 36 to 1,200 dpi"));
    }
    if let Some(quality) = options.jpeg_quality
        && !(1..=100).contains(&quality)
    {
        return Err(Error::InvalidEdit("JPEG quality runs from 1 to 100"));
    }
    let unreadable = || Error::EditUnsupported("its cross-reference table cannot be read");
    let file = File::new(bytes).ok_or_else(unreadable)?;
    if file.trailer.get(b"Encrypt").is_some() {
        return Err(Error::EditUnsupported("it is encrypted; compressing needs it decrypted first"));
    }
    let mut report = CompressReport { before: bytes.len() as u64, ..CompressReport::default() };

    // 1. Everything the catalog and the information dictionary reach.
    let mut objects: HashMap<u32, (Object, Option<Vec<u8>>)> = HashMap::new();
    let mut order: Vec<u32> = Vec::new();
    let mut queue: VecDeque<u32> = VecDeque::new();
    let enqueue = |object: &Object, queue: &mut VecDeque<u32>| {
        for_each_ref(object, &mut |number| queue.push_back(number));
    };
    for key in [b"Root".as_slice(), b"Info"] {
        if let Some(value) = file.trailer.get(key) {
            enqueue(value, &mut queue);
        }
    }
    while let Some(number) = queue.pop_front() {
        if objects.contains_key(&number) {
            continue;
        }
        if objects.len() >= MAX_OBJECTS {
            return Err(Error::EditUnsupported("it has too many objects"));
        }
        let (mut object, data) = match file.entries.get(&number) {
            Some(Entry::Offset(offset)) => match file.indirect_object_at(*offset) {
                Some((object, data)) => (object, data.map(<[u8]>::to_vec)),
                None => (Object::Null, None),
            },
            Some(Entry::InStream { stream, index }) => {
                (file.in_object_stream(*stream, *index, number).unwrap_or(Object::Null), None)
            }
            None => (Object::Null, None),
        };
        if data.is_some()
            && let Object::Dict(dict) = &mut object
        {
            // Lengths are written directly; a length object is no longer needed.
            dict.0.retain(|(key, _)| key != b"Length");
        }
        enqueue(&object, &mut queue);
        objects.insert(number, (object, data));
        order.push(number);
    }
    report.objects_removed = file.entries.len().saturating_sub(objects.len());
    if objects.is_empty() {
        return Err(unreadable());
    }

    // 2. Images, reduced where every placement shows them finer than needed.
    if let Some(dpi) = options.image_dpi {
        let resolutions = image_resolutions(&file);
        for (number, resolution) in resolutions {
            let Some((Object::Dict(dict), Some(data))) = objects.get(&number).cloned() else { continue };
            if resolution <= f64::from(dpi) * 1.5 {
                continue;
            }
            match reduce_image(&dict, &data, &file, resolution, dpi, options.jpeg_quality) {
                Some((dict, data)) => {
                    objects.insert(number, (Object::Dict(dict), Some(data)));
                    report.images_reduced += 1;
                }
                None => report.images_kept += 1,
            }
        }
    }

    // 3. Uncompressed streams compressed.
    for (object, data) in objects.values_mut() {
        let (Object::Dict(dict), Some(data)) = (object, data) else { continue };
        let metadata = dict.get(b"Type").and_then(Object::as_name) == Some(b"Metadata");
        if dict.get(b"Filter").is_some() || data.len() < 64 || metadata {
            continue;
        }
        *data = miniz_oxide::deflate::compress_to_vec_zlib(data, 9);
        dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
        dict.0.retain(|(key, _)| key != b"DecodeParms");
        report.streams_compressed += 1;
    }

    // 4. Written again, numbered from 1 in the order they were reached.
    let renumber: HashMap<u32, u32> =
        order.iter().enumerate().map(|(i, &old)| (old, u32::try_from(i + 1).unwrap_or(u32::MAX))).collect();
    let count =
        u32::try_from(order.len() + 1).map_err(|_| Error::EditUnsupported("it has too many objects"))?;
    let mut trailer = Dict::default();
    for key in [b"Root".as_slice(), b"Info", b"ID"] {
        if let Some(value) = file.trailer.get(key) {
            let mut value = value.clone();
            renumber_refs(&mut value, &renumber);
            trailer.0.push((key.to_vec(), value));
        }
    }

    let mut out: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    // Per new number: (kind, field 2, field 3) as in a cross-reference stream.
    let mut xref: Vec<(u8, u64, u16)> = vec![(0, 0, 65_535); count as usize];
    let mut packed: Vec<(u32, Vec<u8>)> = Vec::new();
    for &old in &order {
        let new = renumber[&old];
        let Some((mut object, data)) = objects.remove(&old) else { continue };
        renumber_refs(&mut object, &renumber);
        match data {
            Some(data) => {
                let Object::Dict(mut dict) = object else { continue };
                dict.0.push((b"Length".to_vec(), Object::Int(data.len() as i64)));
                xref[new as usize] = (1, out.len() as u64, 0);
                let _ = writeln!(out, "{new} 0 obj");
                write_dict(&mut out, &dict);
                out.extend_from_slice(b"\nstream\n");
                out.extend_from_slice(&data);
                out.extend_from_slice(b"\nendstream\nendobj\n");
            }
            None if options.object_streams => {
                let mut body = Vec::new();
                write_object(&mut body, &object);
                packed.push((new, body));
            }
            None => {
                xref[new as usize] = (1, out.len() as u64, 0);
                let _ = writeln!(out, "{new} 0 obj");
                write_object(&mut out, &object);
                out.extend_from_slice(b"\nendobj\n");
            }
        }
    }

    if options.object_streams {
        let mut next = count;
        for chunk in packed.chunks(PER_OBJECT_STREAM) {
            let stream = next;
            next += 1;
            xref.push((0, 0, 0));
            let mut header = String::new();
            let mut body = Vec::new();
            for (index, (number, bytes)) in chunk.iter().enumerate() {
                header.push_str(&format!("{number} {} ", body.len()));
                body.extend_from_slice(bytes);
                body.push(b'\n');
                xref[*number as usize] = (2, u64::from(stream), index as u16);
            }
            let mut data = header.into_bytes();
            let first = data.len();
            data.extend_from_slice(&body);
            let data = miniz_oxide::deflate::compress_to_vec_zlib(&data, 9);
            xref[stream as usize] = (1, out.len() as u64, 0);
            let _ = writeln!(out, "{stream} 0 obj");
            write_dict(
                &mut out,
                &Dict(vec![
                    (b"Type".to_vec(), Object::Name(b"ObjStm".to_vec())),
                    (b"N".to_vec(), Object::Int(chunk.len() as i64)),
                    (b"First".to_vec(), Object::Int(first as i64)),
                    (b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())),
                    (b"Length".to_vec(), Object::Int(data.len() as i64)),
                ]),
            );
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&data);
            out.extend_from_slice(b"\nendstream\nendobj\n");
        }
        // The cross-reference stream takes the last number.
        let own = next;
        let start = out.len() as u64;
        xref.push((1, start, 0));
        let mut rows = Vec::with_capacity(xref.len() * 11);
        for (kind, second, third) in &xref {
            rows.push(*kind);
            rows.extend_from_slice(&second.to_be_bytes());
            rows.extend_from_slice(&third.to_be_bytes());
        }
        let rows = miniz_oxide::deflate::compress_to_vec_zlib(&rows, 9);
        trailer.0.extend([
            (b"Type".to_vec(), Object::Name(b"XRef".to_vec())),
            (b"Size".to_vec(), Object::Int(i64::from(own) + 1)),
            (b"W".to_vec(), Object::Array(vec![Object::Int(1), Object::Int(8), Object::Int(2)])),
            (b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())),
            (b"Length".to_vec(), Object::Int(rows.len() as i64)),
        ]);
        let _ = writeln!(out, "{own} 0 obj");
        write_dict(&mut out, &trailer);
        out.extend_from_slice(b"\nstream\n");
        out.extend_from_slice(&rows);
        out.extend_from_slice(b"\nendstream\nendobj\n");
        let _ = write!(out, "startxref\n{start}\n%%EOF\n");
    } else {
        let start = out.len();
        let _ = write!(out, "xref\n0 {count}\n");
        for (kind, offset, generation) in &xref {
            let tag = if *kind == 1 { 'n' } else { 'f' };
            let offset = if *kind == 1 { *offset } else { 0 };
            let _ = write!(out, "{offset:010} {generation:05} {tag}\r\n");
        }
        trailer.0.push((b"Size".to_vec(), Object::Int(i64::from(count))));
        out.extend_from_slice(b"trailer\n");
        write_dict(&mut out, &trailer);
        let _ = write!(out, "\nstartxref\n{start}\n%%EOF\n");
    }
    report.after = out.len() as u64;
    Ok((out, report))
}

fn for_each_ref(object: &Object, found: &mut impl FnMut(u32)) {
    match object {
        Object::Ref(number, _) => found(*number),
        Object::Array(items) => items.iter().for_each(|item| for_each_ref(item, found)),
        Object::Dict(dict) => dict.0.iter().for_each(|(_, value)| for_each_ref(value, found)),
        _ => {}
    }
}

/// References to objects that are not written become `null` (ISO 32000-1, 7.3.10).
fn renumber_refs(object: &mut Object, renumber: &HashMap<u32, u32>) {
    match object {
        Object::Ref(number, generation) => match renumber.get(number) {
            Some(&new) => {
                *number = new;
                *generation = 0;
            }
            None => *object = Object::Null,
        },
        Object::Array(items) => items.iter_mut().for_each(|item| renumber_refs(item, renumber)),
        Object::Dict(dict) => dict.0.iter_mut().for_each(|(_, value)| renumber_refs(value, renumber)),
        _ => {}
    }
}

type Matrix = [f64; 6];

fn multiply(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Int(i) => Some(*i as f64),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

/// The lowest resolution (pixels per inch, along either axis) at which each
/// image XObject is drawn on any page. Images found nowhere are absent.
fn image_resolutions(file: &File<'_>) -> HashMap<u32, f64> {
    let mut found: HashMap<u32, f64> = HashMap::new();
    let Ok(tree) = PageTree::read(file) else { return found };
    for leaf in &tree.leaves {
        let resources = tree
            .attribute(leaf, b"Resources")
            .and_then(|value| file.resolve(&value))
            .and_then(|value| value.as_dict().cloned())
            .unwrap_or_default();
        let Ok(content) = crate::redact::page_content(file, &leaf.dict) else { continue };
        walk(file, &content, &resources, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0, &mut found);
    }
    found
}

fn walk(
    file: &File<'_>,
    content: &[u8],
    resources: &Dict,
    base: Matrix,
    depth: usize,
    found: &mut HashMap<u32, f64>,
) {
    let Some(ops) = tokenize(content) else { return };
    let xobjects = resources
        .get(b"XObject")
        .and_then(|value| file.resolve(value))
        .and_then(|value| value.as_dict().cloned())
        .unwrap_or_default();
    let mut ctm = base;
    let mut stack: Vec<Matrix> = Vec::new();
    for op in ops {
        match op.name.as_slice() {
            b"q" => stack.push(ctm),
            b"Q" => ctm = stack.pop().unwrap_or(base),
            b"cm" => {
                let values: Vec<f64> = op
                    .operands
                    .iter()
                    .filter_map(|o| match o {
                        Operand::Num(n) => Some(*n),
                        _ => None,
                    })
                    .collect();
                if let [a, b, c, d, e, f] = values[..] {
                    ctm = multiply(&[a, b, c, d, e, f], &ctm);
                }
            }
            b"Do" => {
                let Some(Operand::Name(name)) = op.operands.first() else { continue };
                let Some(Object::Ref(xobject, _)) = xobjects.get(name) else { continue };
                let Some(Entry::Offset(offset)) = file.entries.get(xobject).copied() else { continue };
                let Some((dict, data)) = file.indirect_at(offset) else { continue };
                match dict.get(b"Subtype").and_then(Object::as_name) {
                    Some(b"Image") => {
                        let width = dict.get(b"Width").and_then(number).unwrap_or(0.0);
                        let height = dict.get(b"Height").and_then(number).unwrap_or(0.0);
                        // The unit square's sides, in points, then inches.
                        let across = ctm[0].hypot(ctm[1]) / 72.0;
                        let down = ctm[2].hypot(ctm[3]) / 72.0;
                        if across <= 0.0 || down <= 0.0 {
                            continue;
                        }
                        let resolution = (width / across).min(height / down);
                        let entry = found.entry(*xobject).or_insert(f64::INFINITY);
                        *entry = entry.min(resolution);
                    }
                    Some(b"Form") if depth < MAX_FORM_DEPTH => {
                        let Some(data) = data.and_then(|data| decode_stream(&dict, data, file)) else {
                            continue;
                        };
                        let matrix = dict
                            .get(b"Matrix")
                            .and_then(Object::as_array)
                            .map(|values| values.iter().filter_map(number).collect::<Vec<_>>())
                            .and_then(|values| <[f64; 6]>::try_from(values).ok())
                            .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                        let own = dict
                            .get(b"Resources")
                            .and_then(|value| file.resolve(value))
                            .and_then(|value| value.as_dict().cloned())
                            .unwrap_or_else(|| resources.clone());
                        walk(file, &data, &own, multiply(&matrix, &ctm), depth + 1, found);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// An image reduced from `resolution` to `dpi`: its new dictionary and data,
/// or `None` when its kind is not supported.
fn reduce_image(
    dict: &Dict,
    data: &[u8],
    file: &File<'_>,
    resolution: f64,
    dpi: u32,
    jpeg_quality: Option<u8>,
) -> Option<(Dict, Vec<u8>)> {
    if dict.get(b"ImageMask").is_some_and(|v| *v == Object::Bool(true)) || dict.get(b"Mask").is_some() {
        return None;
    }
    if dict.get(b"BitsPerComponent").and_then(Object::as_int) != Some(8) {
        return None;
    }
    let components = match dict.get(b"ColorSpace").map(|space| file.resolve(space))? {
        Some(Object::Name(name)) if name == b"DeviceGray" => 1,
        Some(Object::Name(name)) if name == b"DeviceRGB" => 3,
        Some(Object::Array(items)) if items.first().and_then(Object::as_name) == Some(b"ICCBased") => {
            let Some(Entry::Offset(offset)) = items.get(1).and_then(|r| match r {
                Object::Ref(n, _) => file.entries.get(n).copied(),
                _ => None,
            }) else {
                return None;
            };
            match file.indirect_at(offset)?.0.get(b"N").and_then(Object::as_int)? {
                1 => 1,
                3 => 3,
                _ => return None,
            }
        }
        _ => return None,
    };
    // Unfiltered, or Flate (with or without a predictor).
    let pixels = decode_stream(dict, data, file)?;
    let width = usize::try_from(dict.get(b"Width").and_then(Object::as_int)?).ok()?;
    let height = usize::try_from(dict.get(b"Height").and_then(Object::as_int)?).ok()?;
    if width == 0 || height == 0 || pixels.len() < width * height * components {
        return None;
    }
    let factor = f64::from(dpi) / resolution;
    let new_width = ((width as f64 * factor).ceil() as usize).clamp(1, width);
    let new_height = ((height as f64 * factor).ceil() as usize).clamp(1, height);
    let reduced = resample(&pixels, width, height, components, new_width, new_height);

    let mut dict = dict.clone();
    dict.0.retain(|(key, _)| !matches!(key.as_slice(), b"Width" | b"Height" | b"Filter" | b"DecodeParms"));
    dict.0.push((b"Width".to_vec(), Object::Int(new_width as i64)));
    dict.0.push((b"Height".to_vec(), Object::Int(new_height as i64)));
    let encoded = match jpeg_quality {
        Some(quality) if components == 3 => {
            dict.0.push((b"Filter".to_vec(), Object::Name(b"DCTDecode".to_vec())));
            crate::jpeg::encode_rgb(&reduced, new_width as u32, new_height as u32, quality)?
        }
        _ => {
            dict.0.push((b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec())));
            miniz_oxide::deflate::compress_to_vec_zlib(&reduced, 9)
        }
    };
    // Kept only when it is smaller than what the file holds now.
    (encoded.len() < data.len()).then_some((dict, encoded))
}

/// Area-average resampling: each new pixel is the mean of the source pixels
/// it covers.
fn resample(pixels: &[u8], width: usize, height: usize, channels: usize, nw: usize, nh: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(nw * nh * channels);
    for y in 0..nh {
        let (y0, y1) = (y * height / nh, ((y + 1) * height / nh).max(y * height / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * width / nw, ((x + 1) * width / nw).max(x * width / nw + 1));
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            for c in 0..channels {
                let mut sum = 0u32;
                for sy in y0..y1.min(height) {
                    let row = sy * width;
                    for sx in x0..x1.min(width) {
                        sum += u32::from(pixels[(row + sx) * channels + c]);
                    }
                }
                out.push(((sum + count / 2) / count.max(1)) as u8);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_averages_each_area() {
        // 4 x 2 grey: two 2 x 2 blocks, one dark, one light.
        let pixels = [0, 0, 200, 200, 0, 0, 200, 200];
        assert_eq!(resample(&pixels, 4, 2, 1, 2, 1), vec![0, 200]);
        let rgb = [10, 20, 30, 30, 40, 50];
        assert_eq!(resample(&rgb, 2, 1, 3, 1, 1), vec![20, 30, 40]);
    }

    #[test]
    fn missing_objects_become_null() {
        let mut object = Object::Array(vec![Object::Ref(4, 0), Object::Ref(9, 2)]);
        let renumber = HashMap::from([(9, 1)]);
        renumber_refs(&mut object, &renumber);
        assert_eq!(object, Object::Array(vec![Object::Null, Object::Ref(1, 0)]));
    }
}
