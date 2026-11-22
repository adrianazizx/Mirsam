//! Raster images written into PDF (ADR 0045): image signatures.
//!
//! A JPEG goes in as it is (`/DCTDecode`): PDF readers decode it themselves,
//! so nothing is lost or re-encoded. A PNG is decoded (the `png` crate) and
//! written as 8-bit samples under `/FlateDecode`, with its transparency as a
//! soft mask (`/SMask`), so a signature scanned on white with its background
//! removed lies on the page as ink only.

use super::crypt::Crypt;
use super::write::Update;
use super::{Dict, Object};
use crate::error::{Error, Result};

/// The largest side of an image signature, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 4096;

/// One image to embed as an XObject.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageToEmbed {
    /// Its resource name (`Im0`, …).
    pub(crate) resource: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// `DeviceGray` or `DeviceRGB`.
    pub(crate) color_space: &'static [u8],
    /// `DCTDecode` (the JPEG as it was) or `FlateDecode` (samples, compressed).
    pub(crate) filter: &'static [u8],
    pub(crate) data: Vec<u8>,
    /// Alpha as 8-bit gray samples, compressed, when the image is not opaque.
    pub(crate) mask: Option<Vec<u8>>,
}

/// What kind of image `data` is, by its signature bytes.
fn is_png(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn is_jpeg(data: &[u8]) -> bool {
    data.starts_with(&[0xFF, 0xD8, 0xFF])
}

fn unsupported() -> Error {
    Error::InvalidEdit("only PNG and JPEG images can be used")
}

/// A PNG or JPEG decoded far enough to embed it, named `resource`.
pub(crate) fn image_to_embed(data: &[u8], resource: &str) -> Result<ImageToEmbed> {
    let image = if is_png(data) {
        png_image(data)?
    } else if is_jpeg(data) {
        jpeg_image(data)?
    } else {
        return Err(unsupported());
    };
    Ok(ImageToEmbed { resource: resource.into(), ..image })
}

/// An image's size in pixels, without decoding its samples.
pub(crate) fn image_size(data: &[u8]) -> Result<(u32, u32)> {
    if is_png(data) {
        let decoder = png::Decoder::new(std::io::Cursor::new(data));
        let reader = decoder.read_info().map_err(|_| unsupported())?;
        let info = reader.info();
        return checked_size(info.width, info.height);
    }
    if is_jpeg(data) {
        let (width, height, _) = jpeg_header(data).ok_or_else(unsupported)?;
        return checked_size(width, height);
    }
    Err(unsupported())
}

fn checked_size(width: u32, height: u32) -> Result<(u32, u32)> {
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(Error::InvalidEdit("an image is 1 to 4,096 pixels on each side"));
    }
    Ok((width, height))
}

fn png_image(data: &[u8]) -> Result<ImageToEmbed> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    // Palettes expanded, 16-bit samples cut to 8, transparency as alpha.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|_| unsupported())?;
    let (width, height) = checked_size(reader.info().width, reader.info().height)?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or_else(unsupported)?];
    let frame = reader.next_frame(&mut buffer).map_err(|_| unsupported())?;
    let samples = &buffer[..frame.buffer_size()];
    let pixels = (width as usize) * (height as usize);
    let (channels, gray) = match frame.color_type {
        png::ColorType::Grayscale => (1, true),
        png::ColorType::GrayscaleAlpha => (2, true),
        png::ColorType::Rgb => (3, false),
        png::ColorType::Rgba => (4, false),
        png::ColorType::Indexed => return Err(unsupported()),
    };
    if frame.bit_depth != png::BitDepth::Eight || samples.len() < pixels * channels {
        return Err(unsupported());
    }
    let colors = if gray { 1 } else { 3 };
    let mut color = Vec::with_capacity(pixels * colors);
    let mut alpha = Vec::with_capacity(pixels);
    for pixel in samples.chunks_exact(channels).take(pixels) {
        color.extend_from_slice(&pixel[..colors]);
        if channels == 2 || channels == 4 {
            alpha.push(pixel[channels - 1]);
        }
    }
    let opaque = alpha.iter().all(|a| *a == 255);
    Ok(ImageToEmbed {
        resource: String::new(),
        width,
        height,
        color_space: if gray { b"DeviceGray" } else { b"DeviceRGB" },
        filter: b"FlateDecode",
        data: miniz_oxide::deflate::compress_to_vec_zlib(&color, 6),
        mask: (!opaque).then(|| miniz_oxide::deflate::compress_to_vec_zlib(&alpha, 6)),
    })
}

fn jpeg_image(data: &[u8]) -> Result<ImageToEmbed> {
    let (width, height, components) = jpeg_header(data).ok_or_else(unsupported)?;
    let (width, height) = checked_size(width, height)?;
    let color_space: &'static [u8] = match components {
        1 => b"DeviceGray",
        3 => b"DeviceRGB",
        // CMYK JPEGs disagree on whether their samples are inverted.
        _ => return Err(Error::InvalidEdit("CMYK JPEG images cannot be used; save it as RGB")),
    };
    Ok(ImageToEmbed {
        resource: String::new(),
        width,
        height,
        color_space,
        filter: b"DCTDecode",
        data: data.to_vec(),
        mask: None,
    })
}

/// A JPEG's width, height, and number of components, from its frame header.
fn jpeg_header(data: &[u8]) -> Option<(u32, u32, u8)> {
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return None;
        }
        let marker = data[at + 1];
        if marker == 0xFF {
            at += 1;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
        // Start-of-frame markers, except DHT (C4), JPG (C8), and DAC (CC).
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let header = data.get(at + 4..at + 2 + length)?;
            let height = u32::from(u16::from_be_bytes([*header.get(1)?, *header.get(2)?]));
            let width = u32::from(u16::from_be_bytes([*header.get(3)?, *header.get(4)?]));
            return Some((width, height, *header.get(5)?));
        }
        if marker == 0xDA || length < 2 {
            return None;
        }
        at += 2 + length;
    }
    None
}

/// Writes an image (and its soft mask) into `update`, encrypted where the
/// file is, numbering them from `next`. Returns the image's number.
pub(crate) fn write_image(
    update: &mut Update,
    crypt: Option<&Crypt>,
    next: &mut u32,
    image: &ImageToEmbed,
) -> Result<u32> {
    let mut take = || -> Result<u32> {
        let number = *next;
        *next = number.checked_add(1).ok_or(Error::EditUnsupported("it has too many objects"))?;
        Ok(number)
    };
    let name = |n: &[u8]| Object::Name(n.to_vec());
    let header = |color_space: &[u8], filter: &[u8]| {
        Dict(vec![
            (b"Type".to_vec(), name(b"XObject")),
            (b"Subtype".to_vec(), name(b"Image")),
            (b"Width".to_vec(), Object::Int(i64::from(image.width))),
            (b"Height".to_vec(), Object::Int(i64::from(image.height))),
            (b"ColorSpace".to_vec(), name(color_space)),
            (b"BitsPerComponent".to_vec(), Object::Int(8)),
            (b"Filter".to_vec(), name(filter)),
        ])
    };
    let stream = |update: &mut Update, number: u32, dict: Dict, data: &[u8]| {
        let data = match crypt {
            Some(crypt) => crypt.encrypt_stream(number, 0, data),
            None => data.to_vec(),
        };
        update.stream(number, 0, dict, &data);
    };
    let mask = match &image.mask {
        Some(alpha) => {
            let number = take()?;
            stream(update, number, header(b"DeviceGray", b"FlateDecode"), alpha);
            Some(number)
        }
        None => None,
    };
    let number = take()?;
    let mut dict = header(image.color_space, image.filter);
    if let Some(mask) = mask {
        dict.0.push((b"SMask".to_vec(), Object::Ref(mask, 0)));
    }
    stream(update, number, dict, &image.data);
    Ok(number)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A `width` × `height` RGBA PNG: opaque dark ink on a transparent ground.
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            let pixels: Vec<u8> = (0..width * height)
                .flat_map(|i| if i % 3 == 0 { [10, 20, 90, 255] } else { [255, 255, 255, 0] })
                .collect();
            writer.write_image_data(&pixels).expect("pixels");
        }
        out
    }

    #[test]
    fn a_png_with_transparency_gets_a_soft_mask() {
        let image = image_to_embed(&png(6, 4), "Im0").expect("decodes");
        assert_eq!((image.width, image.height, image.color_space), (6, 4, &b"DeviceRGB"[..]));
        assert_eq!(image.filter, b"FlateDecode");
        let alpha =
            miniz_oxide::inflate::decompress_to_vec_zlib(image.mask.as_ref().expect("mask")).expect("zlib");
        assert_eq!(alpha.len(), 24);
        assert_eq!((alpha[0], alpha[1]), (255, 0));
        assert_eq!(image_size(&png(6, 4)).expect("size"), (6, 4));
    }

    #[test]
    fn a_jpeg_goes_in_as_it_is() {
        // SOI, an APP0 segment, and a baseline frame header: 3 components, 40 × 30.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 30, 0x00, 40, 3]);
        jpeg.extend_from_slice(&[0; 9]);
        let image = image_to_embed(&jpeg, "Im0").expect("reads");
        assert_eq!((image.width, image.height, image.filter), (40, 30, &b"DCTDecode"[..]));
        assert_eq!(image.data, jpeg);
        assert!(image.mask.is_none());
    }

    #[test]
    fn other_files_and_huge_images_are_refused() {
        assert!(matches!(image_to_embed(b"GIF89a", "Im0"), Err(Error::InvalidEdit(_))));
        assert!(matches!(image_size(&png(MAX_IMAGE_SIDE + 1, 1)), Err(Error::InvalidEdit(_))));
    }
}
