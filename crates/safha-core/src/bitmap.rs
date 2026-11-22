/// A rendered raster in PDFium's native byte order: 8-bit B, G, R, A per
/// pixel, row-major, top-left origin, no row padding. Pages render opaque.
///
/// BGRA is what PDFium writes without a conversion pass, and what a 32-bit
/// BMP holds, which is how tiles reach the webview (ADR 0011): WebKit decodes
/// a plain BMP natively, so the pixels never become a JavaScript buffer.
/// [`to_rgba`](Self::to_rgba) converts for PNG export and tests.
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

/// Bytes before the pixels in [`Bitmap::encode_bmp`]: file header and BITMAPINFOHEADER.
pub const BMP_HEADER_LEN: usize = 14 + 40;

impl Bitmap {
    pub(crate) fn from_bgra(width: u32, height: u32, bgra: Vec<u8>) -> Option<Self> {
        let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        (bgra.len() == expected).then_some(Self { width, height, bgra })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels as stored: B, G, R, A.
    pub fn as_bgra(&self) -> &[u8] {
        &self.bgra
    }

    /// A copy with each pixel's bytes in R, G, B, A order (web `ImageData`, PNG).
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = self.bgra.clone();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        rgba
    }

    /// RGBA of the pixel at (`x`, `y`), or `None` when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let [b, g, r, a]: [u8; 4] = self.bgra.get(i..i + 4)?.try_into().ok()?;
        Some([r, g, b, a])
    }

    /// The bitmap as a top-down, uncompressed 32-bit BMP (`BI_RGB`): a
    /// [`BMP_HEADER_LEN`]-byte header, then the pixels unchanged. The alpha
    /// byte is ignored by readers; pages are opaque.
    ///
    /// The plainest form there is, and PDFium's native byte order, so encoding
    /// is one copy. (WebKit rejects the `BITMAPV4` RGBA-bitfields variant.)
    pub fn encode_bmp(&self) -> Vec<u8> {
        let pixels = u32::try_from(self.bgra.len()).unwrap_or(u32::MAX);
        let header = BMP_HEADER_LEN as u32;
        let mut out = Vec::with_capacity(BMP_HEADER_LEN + self.bgra.len());
        // BITMAPFILEHEADER
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&header.saturating_add(pixels).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&header.to_le_bytes());
        // BITMAPINFOHEADER
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&i32::try_from(self.width).unwrap_or(i32::MAX).to_le_bytes());
        // Negative height: rows run top to bottom, as stored.
        out.extend_from_slice(&(-i32::try_from(self.height).unwrap_or(i32::MAX)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        out.extend_from_slice(&pixels.to_le_bytes());
        out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi, in pixels per metre
        out.extend_from_slice(&2835i32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // palette size
        out.extend_from_slice(&0u32.to_le_bytes()); // important colours
        out.extend_from_slice(&self.bgra);
        out
    }
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bitmap")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bmp_is_a_fixed_header_then_the_pixels_top_down() {
        // 2 x 1: a red pixel, then a blue one.
        let bitmap = Bitmap::from_bgra(2, 1, vec![0, 0, 255, 255, 255, 0, 0, 255]).expect("sized");
        assert_eq!(bitmap.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(bitmap.to_rgba(), [255, 0, 0, 255, 0, 0, 255, 255]);
        let bmp = bitmap.encode_bmp();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().expect("4 bytes")), 62);
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().expect("4 bytes")), BMP_HEADER_LEN as u32);
        assert_eq!(i32::from_le_bytes(bmp[18..22].try_into().expect("4 bytes")), 2);
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().expect("4 bytes")), -1);
        assert_eq!(u16::from_le_bytes(bmp[28..30].try_into().expect("2 bytes")), 32);
        assert_eq!(&bmp[BMP_HEADER_LEN..], bitmap.as_bgra());
    }
}
