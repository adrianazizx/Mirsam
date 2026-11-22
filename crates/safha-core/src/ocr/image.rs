//! Page images for recognition: RGB pixels, PNG in and out, and the resize the
//! models were measured with.
//!
//! The resize reproduces Pillow's Lanczos filter exactly (horizontal pass, then
//! vertical, through 8-bit intermediates with 22-bit fixed-point weights). ADR 0030's
//! accuracy was measured on images Pillow resized, so the same pixels reach the model.

use super::OcrError;
use crate::Bitmap;

/// The input size Nuqta-Arabic-OCR was trained on, in pixels (its model card).
pub const NUQTA_PIXELS: u64 = 2_400_000;

/// An 8-bit RGB image, rows top to bottom, no padding.
#[derive(Clone, PartialEq, Eq)]
pub struct OcrImage {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl std::fmt::Debug for OcrImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OcrImage({}x{})", self.width, self.height)
    }
}

impl OcrImage {
    /// Wraps RGB bytes; `rgb` must hold exactly `width * height * 3` bytes.
    pub fn from_rgb(width: u32, height: u32, rgb: Vec<u8>) -> Result<Self, OcrError> {
        if width == 0 || height == 0 {
            return Err(OcrError::Image("empty image"));
        }
        if rgb.len() as u64 != u64::from(width) * u64::from(height) * 3 {
            return Err(OcrError::Image("pixel buffer does not match the image size"));
        }
        Ok(Self { width, height, rgb })
    }

    /// A rendered page (BGRA, opaque) as RGB.
    pub fn from_bitmap(bitmap: &Bitmap) -> Self {
        let rgb = bitmap.as_bgra().as_chunks::<4>().0.iter().flat_map(|px| [px[2], px[1], px[0]]).collect();
        Self { width: bitmap.width(), height: bitmap.height(), rgb }
    }

    /// Decodes a PNG of any colour type and bit depth to 8-bit RGB. Alpha is
    /// dropped, not composited, as Pillow's `convert("RGB")` does.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, OcrError> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(|_| OcrError::Image("not a readable PNG"))?;
        let mut buffer = vec![0; reader.output_buffer_size().ok_or(OcrError::Image("PNG too large"))?];
        let frame = reader.next_frame(&mut buffer).map_err(|_| OcrError::Image("not a readable PNG"))?;
        let data = &buffer[..frame.buffer_size()];
        let rgb = match frame.color_type {
            png::ColorType::Rgb => data.to_vec(),
            png::ColorType::Rgba => {
                data.as_chunks::<4>().0.iter().flat_map(|px| [px[0], px[1], px[2]]).collect()
            }
            png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g]).collect(),
            png::ColorType::GrayscaleAlpha => {
                data.as_chunks::<2>().0.iter().flat_map(|px| [px[0], px[0], px[0]]).collect()
            }
            png::ColorType::Indexed => return Err(OcrError::Image("palette PNG was not expanded")),
        };
        Self::from_rgb(frame.width, frame.height, rgb)
    }

    /// Encodes as an 8-bit RGB PNG (what is sent to the model).
    pub fn encode_png(&self) -> Result<Vec<u8>, OcrError> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|_| OcrError::Image("PNG encoding failed"))?;
        writer.write_image_data(&self.rgb).map_err(|_| OcrError::Image("PNG encoding failed"))?;
        writer.finish().map_err(|_| OcrError::Image("PNG encoding failed"))?;
        Ok(out)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    /// Resizes with Pillow's Lanczos filter (`Image.resize(size, Image.LANCZOS)`).
    pub fn resize(&self, width: u32, height: u32) -> Result<Self, OcrError> {
        if width == 0 || height == 0 {
            return Err(OcrError::Image("empty target size"));
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let horizontal = Coefficients::new(w, width as usize);
        let vertical = Coefficients::new(h, height as usize);
        // Pillow resamples only the source rows the vertical pass reads.
        let first_row = vertical.bounds.first().map_or(0, |b| b.0);
        let last_row = vertical.bounds.last().map_or(h, |b| b.0 + b.1);
        let rows = last_row - first_row;

        let mut temp = if width as usize == w {
            self.rgb[first_row * w * 3..last_row * w * 3].to_vec()
        } else {
            let mut temp = vec![0u8; rows * width as usize * 3];
            for y in 0..rows {
                let src = &self.rgb[(first_row + y) * w * 3..(first_row + y + 1) * w * 3];
                let dst = &mut temp[y * width as usize * 3..(y + 1) * width as usize * 3];
                for (x, (start, len)) in horizontal.bounds.iter().enumerate() {
                    let k = &horizontal.weights[x * horizontal.size..x * horizontal.size + len];
                    for c in 0..3 {
                        let mut ss = 1i64 << (PRECISION_BITS - 1);
                        for (i, &kk) in k.iter().enumerate() {
                            ss += i64::from(src[(start + i) * 3 + c]) * i64::from(kk);
                        }
                        dst[x * 3 + c] = clip8(ss);
                    }
                }
            }
            temp
        };
        if height as usize == h && first_row == 0 {
            return Self::from_rgb(width, height, temp);
        }
        let row = width as usize * 3;
        let mut out = vec![0u8; height as usize * row];
        for (y, (start, len)) in vertical.bounds.iter().enumerate() {
            let k = &vertical.weights[y * vertical.size..y * vertical.size + len];
            let start = start - first_row;
            for x in 0..row {
                let mut ss = 1i64 << (PRECISION_BITS - 1);
                for (i, &kk) in k.iter().enumerate() {
                    ss += i64::from(temp[(start + i) * row + x]) * i64::from(kk);
                }
                out[y * row + x] = clip8(ss);
            }
        }
        temp.clear();
        Self::from_rgb(width, height, out)
    }
}

/// The size Nuqta should see an image at (its card's rule, as the benchmark ran it):
/// page-shaped images (long side at most twice the short one, short side ≥ 450 px)
/// are scaled up or down to about 2.4 MP; anything over 2.4 MP is scaled down; other
/// crops (lines, words) stay as they are.
pub fn nuqta_size(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (f64::from(width), f64::from(height));
    let page = w.max(h) / w.min(h) <= 2.0 && w.min(h) >= 450.0;
    if page || u64::from(width) * u64::from(height) > NUQTA_PIXELS {
        let k = (NUQTA_PIXELS as f64 / (w * h)).sqrt();
        // Python's int() truncates; the floor below is the same for positive values.
        let scaled = |v: f64| ((v * k).floor() as u32).max(32);
        (scaled(w), scaled(h))
    } else {
        (width, height)
    }
}

/// `image` at the size Nuqta should see it ([`nuqta_size`]).
pub fn prepare_nuqta(image: &OcrImage) -> Result<OcrImage, OcrError> {
    let (w, h) = nuqta_size(image.width(), image.height());
    if (w, h) == (image.width(), image.height()) { Ok(image.clone()) } else { image.resize(w, h) }
}

const PRECISION_BITS: u32 = 32 - 8 - 2;

fn clip8(ss: i64) -> u8 {
    (ss >> PRECISION_BITS).clamp(0, 255) as u8
}

fn lanczos(x: f64) -> f64 {
    fn sinc(x: f64) -> f64 {
        if x == 0.0 {
            1.0
        } else {
            let px = x * std::f64::consts::PI;
            px.sin() / px
        }
    }
    if (-3.0..3.0).contains(&x) { sinc(x) * sinc(x / 3.0) } else { 0.0 }
}

/// Pillow's `precompute_coeffs` + `normalize_coeffs_8bpc` for one axis.
struct Coefficients {
    /// Per output pixel: first source pixel and how many it reads.
    bounds: Vec<(usize, usize)>,
    /// `size` weights per output pixel, in 22-bit fixed point.
    weights: Vec<i32>,
    size: usize,
}

impl Coefficients {
    fn new(in_size: usize, out_size: usize) -> Self {
        let scale = in_size as f64 / out_size as f64;
        let filterscale = scale.max(1.0);
        let support = 3.0 * filterscale;
        let size = support.ceil() as usize * 2 + 1;
        let mut bounds = Vec::with_capacity(out_size);
        let mut weights = vec![0i32; out_size * size];
        let mut k = vec![0f64; size];
        for xx in 0..out_size {
            let center = (xx as f64 + 0.5) * scale;
            let ss = 1.0 / filterscale;
            let xmin = ((center - support + 0.5) as i64).max(0) as usize;
            let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize;
            let len = xmax.saturating_sub(xmin);
            let mut ww = 0.0;
            for (x, slot) in k.iter_mut().enumerate().take(len) {
                let w = lanczos((x as f64 + xmin as f64 - center + 0.5) * ss);
                *slot = w;
                ww += w;
            }
            for x in 0..len {
                let v = if ww != 0.0 { k[x] / ww } else { k[x] };
                let fixed = v * f64::from(1u32 << PRECISION_BITS);
                weights[xx * size + x] = if v < 0.0 { (fixed - 0.5) as i32 } else { (fixed + 0.5) as i32 };
            }
            bounds.push((xmin, len));
        }
        Self { bounds, weights, size }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> OcrImage {
        let rgb = (0..h)
            .flat_map(|y| {
                (0..w).flat_map(move |x| [(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 128])
            })
            .collect();
        OcrImage::from_rgb(w, h, rgb).expect("valid size")
    }

    #[test]
    fn nuqta_size_follows_the_card() {
        // A 300 dpi A4 page (2481 x 3508, 8.7 MP) goes down to ~2.4 MP, aspect kept.
        let (w, h) = nuqta_size(2481, 3508);
        assert!(u64::from(w) * u64::from(h) <= NUQTA_PIXELS);
        assert!(u64::from(w) * u64::from(h) > NUQTA_PIXELS * 99 / 100);
        assert_eq!((w, h), (1302, 1842));
        // A 100 dpi fax page is page-shaped, so it goes up.
        let (w, h) = nuqta_size(827, 1169);
        assert!(w > 827 && h > 1169);
        // A text line stays as it is; a huge strip is capped.
        assert_eq!(nuqta_size(1800, 90), (1800, 90));
        let (w, h) = nuqta_size(20000, 400);
        assert!(u64::from(w) * u64::from(h) <= NUQTA_PIXELS);
    }

    #[test]
    fn resize_keeps_a_flat_image_flat() {
        let image = OcrImage::from_rgb(40, 30, [200u8, 10, 77].repeat(40 * 30)).expect("valid");
        for (w, h) in [(13, 11), (40, 30), (97, 61)] {
            let out = image.resize(w, h).expect("resized");
            assert_eq!((out.width(), out.height()), (w, h));
            assert!(out.rgb().as_chunks::<3>().0.iter().all(|px| *px == [200, 10, 77]), "{w}x{h}");
        }
    }

    #[test]
    fn resize_matches_pillow() {
        // Pillow 13.0 on the same gradient: `resize((7, 5), Image.LANCZOS)` and
        // `resize((23, 17), …)`; the first three pixels and the sum of all channels.
        let sum = |image: &OcrImage| image.rgb().iter().map(|&v| u64::from(v)).sum::<u64>();
        let image = gradient(16, 12);
        let small = image.resize(7, 5).expect("resized");
        assert_eq!(&small.rgb()[..9], &[10, 15, 128, 45, 15, 128, 83, 15, 128]);
        assert_eq!(sum(&small), 12731);
        let large = image.resize(23, 17).expect("resized");
        assert_eq!(&large.rgb()[..9], &[0, 0, 128, 6, 0, 128, 19, 0, 128]);
        assert_eq!(sum(&large), 142180);
    }

    #[test]
    fn png_round_trip() {
        let image = gradient(31, 17);
        let bytes = image.encode_png().expect("encoded");
        assert_eq!(OcrImage::decode_png(&bytes).expect("decoded"), image);
    }

    #[test]
    fn rejects_a_buffer_of_the_wrong_size() {
        assert!(OcrImage::from_rgb(2, 2, vec![0; 11]).is_err());
        assert!(OcrImage::from_rgb(0, 2, vec![]).is_err());
    }
}
