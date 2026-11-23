//! Raster images written into PDF: image signatures (ADR 0045) and image
//! watermarks (ADR 0050).
//!
//! A JPEG goes in as it is (`/DCTDecode`): PDF readers decode it themselves,
//! so nothing is lost or re-encoded. Only its frame header is read
//! (`jpeg::frame`: baseline or progressive, 8-bit, gray or RGB). A PNG is
//! decoded (the `png` crate) and written as 8-bit samples under
//! `/FlateDecode`, with its transparency as a soft mask (`/SMask`) when it has
//! any, so a signature scanned on white with its background removed lies on
//! the page as ink only.
//!
//! Each caller passes its own [`ImageLimits`]: how large an image it takes,
//! and what it says when it refuses one.

use super::crypt::Crypt;
use super::write::Update;
use super::{Dict, Object};
use crate::error::{Error, Result};

/// What one caller takes, and how it refuses the rest (as
/// `Error::InvalidEdit`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImageLimits {
    /// The most pixels along either side.
    pub(crate) max_side: u32,
    /// The most pixels in all. The PNG decoder gets 4 bytes for each.
    pub(crate) max_pixels: u64,
    /// Why an image over either limit is refused.
    pub(crate) too_large: &'static str,
    /// Why a file that is not a PNG or JPEG that can be read is refused.
    pub(crate) unreadable: &'static str,
}

impl ImageLimits {
    fn check(&self, width: u32, height: u32) -> Result<(u32, u32)> {
        let pixels = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || width.max(height) > self.max_side || pixels > self.max_pixels {
            return Err(Error::InvalidEdit(self.too_large));
        }
        Ok((width, height))
    }

    fn unreadable(&self) -> Error {
        Error::InvalidEdit(self.unreadable)
    }
}

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

fn is_png(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// A PNG or JPEG within `limits`, decoded far enough to embed it, named
/// `resource`.
pub(crate) fn image_to_embed(data: &[u8], resource: &str, limits: &ImageLimits) -> Result<ImageToEmbed> {
    let image = if is_png(data) { png_image(data, limits)? } else { jpeg_image(data, limits)? };
    Ok(ImageToEmbed { resource: resource.into(), ..image })
}

/// The size in pixels of a PNG or JPEG within `limits`, without decoding its
/// samples.
pub(crate) fn image_size(data: &[u8], limits: &ImageLimits) -> Result<(u32, u32)> {
    let (width, height) = if is_png(data) {
        let reader = png_reader(data, limits)?;
        (reader.info().width, reader.info().height)
    } else {
        let (width, height, _) = crate::jpeg::frame(data).ok_or_else(|| limits.unreadable())?;
        (width, height)
    };
    limits.check(width, height)
}

/// A PNG's header read, its decoder held to `limits`: palettes expanded,
/// 16-bit samples cut to 8, transparency as alpha.
fn png_reader<'a>(data: &'a [u8], limits: &ImageLimits) -> Result<png::Reader<std::io::Cursor<&'a [u8]>>> {
    let bytes = usize::try_from(limits.max_pixels.saturating_mul(4)).unwrap_or(usize::MAX);
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(data), png::Limits { bytes });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    decoder.read_info().map_err(|_| limits.unreadable())
}

fn png_image(data: &[u8], limits: &ImageLimits) -> Result<ImageToEmbed> {
    let mut reader = png_reader(data, limits)?;
    let (width, height) = limits.check(reader.info().width, reader.info().height)?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or_else(|| limits.unreadable())?];
    let frame = reader.next_frame(&mut buffer).map_err(|_| limits.unreadable())?;
    let samples = &buffer[..frame.buffer_size()];
    let pixels = (width as usize) * (height as usize);
    let (channels, gray) = match frame.color_type {
        png::ColorType::Grayscale => (1, true),
        png::ColorType::GrayscaleAlpha => (2, true),
        png::ColorType::Rgb => (3, false),
        png::ColorType::Rgba => (4, false),
        png::ColorType::Indexed => return Err(limits.unreadable()),
    };
    if frame.bit_depth != png::BitDepth::Eight || samples.len() < pixels * channels {
        return Err(limits.unreadable());
    }
    let colors = if gray { 1 } else { 3 };
    let has_alpha = channels > colors;
    let mut color = Vec::with_capacity(pixels * colors);
    let mut alpha = Vec::with_capacity(if has_alpha { pixels } else { 0 });
    for pixel in samples.chunks_exact(channels).take(pixels) {
        color.extend_from_slice(&pixel[..colors]);
        if has_alpha {
            alpha.push(pixel[colors]);
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

fn jpeg_image(data: &[u8], limits: &ImageLimits) -> Result<ImageToEmbed> {
    // `frame` refuses CMYK, which JPEGs disagree on inverting.
    let (width, height, components) = crate::jpeg::frame(data).ok_or_else(|| limits.unreadable())?;
    let (width, height) = limits.check(width, height)?;
    Ok(ImageToEmbed {
        resource: String::new(),
        width,
        height,
        color_space: if components == 1 { b"DeviceGray" } else { b"DeviceRGB" },
        filter: b"DCTDecode",
        data: data.to_vec(),
        mask: None,
    })
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
    let stream = |update: &mut Update, number: u32, dict: Dict, data: &[u8]| match crypt {
        Some(crypt) => update.stream(number, 0, dict, &crypt.encrypt_stream(number, 0, data)),
        None => update.stream(number, 0, dict, data),
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

    /// A signature's limits: each side.
    const SIDES: ImageLimits =
        ImageLimits { max_side: 64, max_pixels: 64 * 64, too_large: "too large", unreadable: "unreadable" };
    /// A watermark's limits: pixels in all, any shape.
    const PIXELS: ImageLimits =
        ImageLimits { max_side: u32::MAX, max_pixels: 10_000, too_large: "too many", unreadable: "not read" };

    /// A `width` × `height` RGBA PNG: opaque dark ink on a transparent ground.
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        png_of(width, height, |i| if i % 3 == 0 { [10, 20, 90, 255] } else { [255, 255, 255, 0] })
    }

    fn png_of(width: u32, height: u32, pixel: impl Fn(u32) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            let pixels: Vec<u8> = (0..width * height).flat_map(pixel).collect();
            writer.write_image_data(&pixels).expect("pixels");
        }
        out
    }

    #[test]
    fn a_png_with_transparency_gets_a_soft_mask() {
        let image = image_to_embed(&png(6, 4), "Im0", &SIDES).expect("decodes");
        assert_eq!((image.width, image.height, image.color_space), (6, 4, &b"DeviceRGB"[..]));
        assert_eq!(image.filter, b"FlateDecode");
        let alpha =
            miniz_oxide::inflate::decompress_to_vec_zlib(image.mask.as_ref().expect("mask")).expect("zlib");
        assert_eq!(alpha.len(), 24);
        assert_eq!((alpha[0], alpha[1]), (255, 0));
        assert_eq!(image_size(&png(6, 4), &SIDES).expect("size"), (6, 4));
    }

    #[test]
    fn an_opaque_png_gets_no_soft_mask() {
        let image = image_to_embed(&png_of(3, 2, |_| [0, 0, 255, 255]), "Im0", &PIXELS).expect("decodes");
        assert!(image.mask.is_none());
        let color = miniz_oxide::inflate::decompress_to_vec_zlib(&image.data).expect("zlib");
        assert_eq!(color, [0, 0, 255].repeat(6));
    }

    #[test]
    fn a_jpeg_goes_in_as_it_is() {
        // SOI, an APP0 segment, and a baseline frame header: 8-bit, 40 × 30, 3 components.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 30, 0x00, 40, 3]);
        jpeg.extend_from_slice(&[0; 9]);
        let image = image_to_embed(&jpeg, "Im0", &SIDES).expect("reads");
        assert_eq!((image.width, image.height, image.filter), (40, 30, &b"DCTDecode"[..]));
        assert_eq!(image.color_space, b"DeviceRGB");
        assert_eq!(image.data, jpeg);
        assert!(image.mask.is_none());
        assert_eq!(image_size(&jpeg, &SIDES).expect("size"), (40, 30));
        // The same frame with four components (CMYK) is refused.
        jpeg[17] = 4;
        assert!(matches!(image_to_embed(&jpeg, "Im0", &SIDES), Err(Error::InvalidEdit("unreadable"))));
    }

    #[test]
    fn other_files_and_images_over_the_limits_are_refused() {
        assert!(matches!(image_to_embed(b"GIF89a", "Im0", &SIDES), Err(Error::InvalidEdit("unreadable"))));
        assert!(matches!(image_size(b"GIF89a", &PIXELS), Err(Error::InvalidEdit("not read"))));
        // A side over a signature's limit; the same pixels in all are within a watermark's.
        assert!(matches!(image_size(&png(65, 1), &SIDES), Err(Error::InvalidEdit("too large"))));
        assert!(image_to_embed(&png(65, 1), "Im0", &PIXELS).is_ok());
        // Pixels over a watermark's limit, in any shape.
        assert!(matches!(
            image_to_embed(&png(101, 100), "Im0", &PIXELS),
            Err(Error::InvalidEdit("too many"))
        ));
        assert!(matches!(image_size(&png(200, 51), &PIXELS), Err(Error::InvalidEdit("too many"))));
    }

    #[test]
    fn the_soft_mask_is_written_first_and_referenced() {
        let image = image_to_embed(&png(6, 4), "Im0", &SIDES).expect("decodes");
        let mut update = Update::new(b"%PDF-1.7\n");
        let mut next = 10;
        let number = write_image(&mut update, None, &mut next, &image).expect("written");
        assert_eq!((number, next), (11, 12));
        let opaque = image_to_embed(&png_of(2, 2, |_| [0, 0, 0, 255]), "Im0", &SIDES).expect("decodes");
        let number = write_image(&mut update, None, &mut next, &opaque).expect("written");
        assert_eq!((number, next), (12, 13), "no mask, one object");
    }
}
