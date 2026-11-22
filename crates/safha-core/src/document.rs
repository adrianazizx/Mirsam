use std::sync::Arc;

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

    /// Sizes of every page, in order, read without loading any page's content,
    /// so a viewer can lay out a long document before rendering anything.
    pub fn page_sizes(&self) -> Result<Vec<PageSize>> {
        Ok(self
            .inner
            .pages()
            .page_sizes()?
            .iter()
            .map(|rect| PageSize { width: rect.width().value, height: rect.height().value })
            .collect())
    }

    /// Parses one page and keeps it open, so several tiles can be rendered from
    /// it without re-parsing its content each time (parsing costs more than
    /// rasterizing a 512 px tile of a typical text page). The page holds a
    /// reference to the document, which therefore stays open at least as long.
    pub fn load_page(self: &Arc<Self>, index: usize) -> Result<LoadedPage> {
        let page = self.page(index)?;
        let size = PageSize { width: page.width().value, height: page.height().value };
        Ok(LoadedPage { page, size, index, _document: Arc::clone(self) })
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
        validate_tile_size(&tile)?;
        let page = self.page(index)?;
        let size = PageSize { width: page.width().value, height: page.height().value };
        render_tile_of(&page, size, tile)
    }

    /// The page's text exactly as PDFium reports it.
    ///
    /// This is *not* reading order: PDFs frequently store Arabic in visual order.
    /// The bidi-aware extraction required by spec section 6.1 is Milestone 1 and
    /// will be a separate API; this one exists for diagnostics and fixtures.
    pub fn page_text_raw(&self, index: usize) -> Result<String> {
        Ok(self.page(index)?.text()?.all())
    }

    /// The document outline (bookmarks), as a tree.
    pub fn outline(&self) -> Vec<crate::outline::OutlineItem> {
        crate::outline::read(&self.inner)
    }

    /// The page's label (e.g. "iv", "A-3") when the document defines labels.
    /// Loads the page, so ask for visible pages, not all of them.
    pub fn page_label(&self, index: usize) -> Result<Option<String>> {
        Ok(self.page(index)?.label().map(str::to_string))
    }

    /// The page's text in logical reading order, with per-character geometry
    /// for selection (spec section 6.1). See [`crate::text`].
    pub fn page_text(&self, index: usize) -> Result<crate::text::PageText> {
        let page = self.page(index)?;
        let text = page.text()?;
        let glyphs = crate::text::collect_glyphs(&text);
        let spaces = crate::text::collect_space_marks(&page, &text);
        Ok(crate::text::analyze(&glyphs, &spaces))
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

/// One page of a [`Document`], parsed once and kept open for rendering
/// (see [`Document::load_page`]).
pub struct LoadedPage {
    // Declared before `_document` so the page is closed before the document.
    page: PdfPage<'static>,
    size: PageSize,
    index: usize,
    _document: Arc<Document>,
}

impl LoadedPage {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn size(&self) -> PageSize {
        self.size
    }

    /// Same contract as [`Document::render_tile`].
    pub fn render_tile(&self, tile: TileRequest) -> Result<Bitmap> {
        validate_tile_size(&tile)?;
        render_tile_of(&self.page, self.size, tile)
    }
}

impl std::fmt::Debug for LoadedPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPage").field("index", &self.index).field("size", &self.size).finish()
    }
}

fn validate_tile_size(tile: &TileRequest) -> Result<()> {
    if tile.width == 0 || tile.height == 0 {
        return Err(Error::InvalidRenderRequest("tile has zero area"));
    }
    if tile.width > MAX_BITMAP_SIDE || tile.height > MAX_BITMAP_SIDE {
        return Err(Error::InvalidRenderRequest("tile exceeds the maximum bitmap size"));
    }
    Ok(())
}

fn render_tile_of(page: &PdfPage<'_>, size: PageSize, tile: TileRequest) -> Result<Bitmap> {
    let (page_w, page_h) = scaled_page_pixels(size, tile.scale)?;

    if tile.x.checked_add(tile.width).is_none_or(|right| right > page_w)
        || tile.y.checked_add(tile.height).is_none_or(|bottom| bottom > page_h)
    {
        return Err(Error::InvalidRenderRequest("tile extends past the page"));
    }

    // PDFium renders straight into our buffer: no engine-side allocation and no
    // copy per tile. A 4-byte format's rows need no padding, so the buffer is
    // exactly the `Bitmap` layout.
    let byte_len = (tile.width as usize)
        .checked_mul(tile.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Error::InvalidRenderRequest("tile exceeds the maximum bitmap size"))?;
    let mut pixels = vec![0u8; byte_len];
    {
        let mut bitmap = PdfBitmap::from_bytes(
            to_i32(tile.width)?,
            to_i32(tile.height)?,
            PdfBitmapFormat::BGRA,
            &mut pixels,
        )?;
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
    }

    // `PdfRenderConfig` renders with reversed byte order by default, so the
    // buffer is already RGBA.
    Bitmap::from_rgba(tile.width, tile.height, pixels)
        .ok_or_else(|| Error::Pdfium("PDFium returned a bitmap of unexpected size".into()))
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
