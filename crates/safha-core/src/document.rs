use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfColor, PdfDocument, PdfPage, PdfPageIndex, PdfRenderConfig,
};
use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::error::{Error, Result};

/// Upper bound on either side of a single rendered bitmap. Larger views are
/// assembled from tiles; this guards against a hostile or buggy request
/// allocating gigabytes.
pub const MAX_BITMAP_SIDE: u32 = 8192;

/// Page dimensions in PDF points (1/72 inch), after the page's own `/Rotate`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// A rectangle of a page rendered at a given zoom.
///
/// `scale` is device pixels per PDF point, so the whole page is
/// `ceil(width * scale) x ceil(height * scale)` pixels and the tile is the
/// `width x height` pixel window whose top-left corner sits at (`x`, `y`) in
/// that space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TileRequest {
    pub scale: f32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// An open PDF document.
pub struct Document {
    inner: PdfDocument<'static>,
}

impl Document {
    pub(crate) fn new(inner: PdfDocument<'static>) -> Self {
        Self { inner }
    }

    pub fn page_count(&self) -> usize {
        usize::try_from(self.inner.pages().len()).unwrap_or(0)
    }

    /// Size of one page, read without loading the page's content.
    pub fn page_size(&self, index: usize) -> Result<PageSize> {
        let rect = self.inner.pages().page_size(self.page_index(index)?)?;
        Ok(PageSize { width: rect.width().value, height: rect.height().value })
    }

    /// Renders the whole page at `scale` device pixels per point.
    pub fn render_page(&self, index: usize, scale: f32) -> Result<Bitmap> {
        let size = self.page_size(index)?;
        let (width, height) = scaled_page_pixels(size, scale)?;
        self.render_tile(index, TileRequest { scale, x: 0, y: 0, width, height })
    }

    /// Renders one tile of a page. The tile must lie entirely inside the scaled
    /// page; callers clamp edge tiles to the page bounds.
    pub fn render_tile(&self, index: usize, tile: TileRequest) -> Result<Bitmap> {
        if tile.width == 0 || tile.height == 0 {
            return Err(Error::InvalidRenderRequest("tile has zero area"));
        }
        if tile.width > MAX_BITMAP_SIDE || tile.height > MAX_BITMAP_SIDE {
            return Err(Error::InvalidRenderRequest("tile exceeds the maximum bitmap size"));
        }

        let page = self.page(index)?;
        let size = PageSize { width: page.width().value, height: page.height().value };
        let (page_w, page_h) = scaled_page_pixels(size, tile.scale)?;

        if tile.x.checked_add(tile.width).is_none_or(|right| right > page_w)
            || tile.y.checked_add(tile.height).is_none_or(|bottom| bottom > page_h)
        {
            return Err(Error::InvalidRenderRequest("tile extends past the page"));
        }

        let mut bitmap = PdfBitmap::empty(to_i32(tile.width)?, to_i32(tile.height)?, PdfBitmapFormat::BGRA)?;
        let config = PdfRenderConfig::new()
            .set_fixed_size(to_i32(page_w)?, to_i32(page_h)?)
            // A negative origin slides the full-size page under the tile-sized bitmap.
            // pdfium-render only honours the origin on its form-data render path
            // (FPDF_RenderPageBitmap); the matrix path ignores it, so form data
            // rendering must stay enabled here.
            .set_origin(-to_i32(tile.x)?, -to_i32(tile.y)?)
            .render_form_data(true)
            .render_annotations(true)
            .clear_before_rendering(true)
            .set_clear_color(PdfColor::WHITE);
        page.render_into_bitmap_with_config(&mut bitmap, &config)?;

        // `PdfRenderConfig` renders with reversed byte order by default, so the
        // raw buffer is already RGBA.
        Bitmap::from_rgba(tile.width, tile.height, bitmap.as_raw_bytes())
            .ok_or_else(|| Error::Pdfium("PDFium returned a bitmap of unexpected size".into()))
    }

    /// The page's text exactly as PDFium reports it.
    ///
    /// This is *not* reading order: PDFs frequently store Arabic in visual order.
    /// The bidi-aware extraction required by spec section 6.1 is Milestone 1 and
    /// will be a separate API; this one exists for diagnostics and fixtures.
    pub fn page_text_raw(&self, index: usize) -> Result<String> {
        Ok(self.page(index)?.text()?.all())
    }

    fn page(&self, index: usize) -> Result<PdfPage<'static>> {
        Ok(self.inner.pages().get(self.page_index(index)?)?)
    }

    fn page_index(&self, index: usize) -> Result<PdfPageIndex> {
        let count = self.page_count();
        if index >= count {
            return Err(Error::PageOutOfRange { index, count });
        }
        PdfPageIndex::try_from(index).map_err(|_| Error::PageOutOfRange { index, count })
    }
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document").field("page_count", &self.page_count()).finish()
    }
}

fn scaled_page_pixels(size: PageSize, scale: f32) -> Result<(u32, u32)> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(Error::InvalidRenderRequest("scale must be a positive number"));
    }
    let width = (size.width * scale).ceil();
    let height = (size.height * scale).ceil();
    // A page can legitimately be far larger than one bitmap once zoomed in (that is
    // what tiles are for), but it must still fit PDFium's signed 32-bit coordinates.
    if !(1.0..=i32::MAX as f32).contains(&width) || !(1.0..=i32::MAX as f32).contains(&height) {
        return Err(Error::InvalidRenderRequest("scaled page size is out of range"));
    }
    Ok((width as u32, height as u32))
}

fn to_i32(value: u32) -> Result<i32> {
    i32::try_from(value).map_err(|_| Error::InvalidRenderRequest("coordinate is out of range"))
}
