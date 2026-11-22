use std::sync::Arc;

use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfColor, PdfDocument, PdfPage, PdfPageAnnotationCommon,
    PdfPageAnnotationType, PdfPageIndex, PdfRenderConfig,
};
use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::error::{Error, Result};
use crate::view::ViewTransform;

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
    hints: crate::catalog::CatalogHints,
    /// The file's bytes as PDFium reads them; an edit appends to a copy.
    bytes: Arc<[u8]>,
    /// Kept in memory while the document is open, so an edited revision of an
    /// encrypted file can be opened too (ADR 0014). Never written anywhere.
    password: Option<String>,
    engine: &'static crate::Engine,
}

impl Document {
    pub(crate) fn new(
        inner: PdfDocument<'static>,
        hints: crate::catalog::CatalogHints,
        bytes: Arc<[u8]>,
        password: Option<String>,
        engine: &'static crate::Engine,
    ) -> Self {
        Self { inner, hints, bytes, password, engine }
    }

    /// The document's bytes: the file as opened plus any edits.
    pub fn bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    /// Another revision of this document (after an edit, an undo or a redo),
    /// opened with the same password.
    pub fn revision(&self, bytes: Arc<[u8]>) -> Result<Document> {
        self.engine.open_shared(bytes, self.password.as_deref())
    }

    /// This document with `edit` applied, and the update that applies it.
    pub fn with_page_edit(&self, edit: &crate::edit::PageEdit) -> Result<(Document, Vec<u8>)> {
        let update = crate::edit::page_edit_update(&self.bytes, edit)?;
        let mut bytes = Vec::with_capacity(self.bytes.len() + update.len());
        bytes.extend_from_slice(&self.bytes);
        bytes.extend_from_slice(&update);
        Ok((self.revision(Arc::from(bytes))?, update))
    }

    /// This document with an annotation added or removed, and the update that
    /// does it (ADR 0017).
    pub fn with_annotation_edit(
        &self,
        edit: &crate::annotate::AnnotationEdit,
    ) -> Result<(Document, Vec<u8>)> {
        self.with_annotation_edits(std::slice::from_ref(edit))
    }

    /// This document with `edits` applied in order (marking a selection that
    /// spans pages adds one annotation per page), and the one update that
    /// does them all, so a single undo takes them back.
    pub fn with_annotation_edits(
        &self,
        edits: &[crate::annotate::AnnotationEdit],
    ) -> Result<(Document, Vec<u8>)> {
        if edits.is_empty() {
            return Err(Error::InvalidEdit("no annotation edits given"));
        }
        let mut bytes = self.bytes.to_vec();
        let start = bytes.len();
        for edit in edits {
            // Each update builds on the last: appended together they are one.
            let update = crate::annotate::annotation_update(&bytes, self.password.as_deref(), edit)?;
            bytes.extend_from_slice(&update);
        }
        let update = bytes[start..].to_vec();
        Ok((self.revision(Arc::from(bytes))?, update))
    }

    /// The page's annotations of the kinds this app writes (markup, ink,
    /// notes), and the map from page space to the page as displayed.
    pub fn page_annotations(
        &self,
        index: usize,
    ) -> Result<(Vec<crate::annotate::AnnotationInfo>, ViewTransform)> {
        use crate::annotate::{AnnotationInfo, AnnotationKind};
        let page = self.page(index)?;
        let annotations = page.annotations();
        let mut out = Vec::new();
        // By index, not by iterator: the iterator stops at the first entry
        // PDFium cannot read, and the index is the annotation's position in
        // `/Annots`, which changing or removing it needs.
        for position in 0..annotations.len() {
            let Ok(annotation) = annotations.get(position) else { continue };
            let kind = match annotation.annotation_type() {
                PdfPageAnnotationType::Highlight => AnnotationKind::Highlight,
                PdfPageAnnotationType::Underline => AnnotationKind::Underline,
                PdfPageAnnotationType::Strikeout => AnnotationKind::StrikeOut,
                PdfPageAnnotationType::Squiggly => AnnotationKind::Squiggly,
                PdfPageAnnotationType::Ink => AnnotationKind::Ink,
                PdfPageAnnotationType::Text => AnnotationKind::Note,
                _ => continue,
            };
            let rect =
                |left: f32, bottom: f32, right: f32, top: f32| crate::text::Rect { left, bottom, right, top };
            let points = annotation.attachment_points();
            let mut rects: Vec<_> = (0..points.len())
                .filter_map(|q| points.get(q).ok())
                .map(|quad| {
                    rect(quad.left().value, quad.bottom().value, quad.right().value, quad.top().value)
                })
                .filter(|r| r.width() > 0.0 && r.height() > 0.0)
                .collect();
            if rects.is_empty()
                && let Ok(bounds) = annotation.bounds()
            {
                rects.push(rect(
                    bounds.left().value,
                    bounds.bottom().value,
                    bounds.right().value,
                    bounds.top().value,
                ));
            }
            let contents = (kind == AnnotationKind::Note).then(|| annotation.contents().unwrap_or_default());
            out.push(AnnotationInfo { index: position, kind, rects, contents, strokes: Vec::new() });
        }
        if out.iter().any(|info| info.kind == AnnotationKind::Ink) {
            let mut strokes = crate::annotate::ink_strokes(&self.bytes, index);
            for info in &mut out {
                if let Some(found) = strokes.remove(&info.index) {
                    info.strokes = found;
                }
            }
        }
        Ok((out, ViewTransform::of_page(&page)?))
    }

    /// The map from page space to the page as displayed.
    pub fn view_transform(&self, index: usize) -> Result<ViewTransform> {
        ViewTransform::of_page(&self.page(index)?)
    }

    /// `pages` (in that order) as a PDF of their own, copied by PDFium with
    /// everything they use (fonts, images, annotations). The new file has no
    /// outline and no encryption: it is a fresh document (ADR 0016).
    pub fn extract_pages(&self, pages: &[usize]) -> Result<Vec<u8>> {
        if pages.is_empty() {
            return Err(Error::InvalidEdit("no pages given"));
        }
        let mut out = self.engine.new_pdf()?;
        for (position, &page) in pages.iter().enumerate() {
            let index = self.page_index(page)?;
            let at = PdfPageIndex::try_from(position).map_err(|_| Error::InvalidEdit("too many pages"))?;
            out.pages_mut().copy_page_range_from_document(&self.inner, index..=index, at)?;
        }
        Ok(out.save_to_bytes()?)
    }

    /// The document in parts of at most `size` pages each, in order.
    pub fn split_every(&self, size: usize) -> Result<Vec<Vec<u8>>> {
        if size == 0 {
            return Err(Error::InvalidEdit("parts need at least one page"));
        }
        let pages: Vec<usize> = (0..self.page_count()).collect();
        pages.chunks(size).map(|part| self.extract_pages(part)).collect()
    }

    /// This document with every page of `others` inserted before page `at`
    /// (`at` = page count appends), in order; and the new revision's bytes.
    ///
    /// PDFium copies the pages with everything they use, then rewrites the
    /// whole file: copying objects across files is beyond the incremental
    /// writer (ADR 0016). A signed document is refused, since a rewrite would
    /// invalidate its signatures.
    pub fn with_pages_from(&self, others: &[&Document], at: usize) -> Result<(Document, Arc<[u8]>)> {
        if self.is_signed() {
            return Err(Error::EditUnsupported(
                "it is signed, and inserting pages from another file would invalidate its signatures",
            ));
        }
        let count = self.page_count();
        if at > count {
            return Err(Error::PageOutOfRange { index: at, count });
        }
        let mut working = self.revision(self.bytes())?;
        let mut position = PdfPageIndex::try_from(at).map_err(|_| Error::InvalidEdit("too many pages"))?;
        for other in others {
            let pages = PdfPageIndex::try_from(other.page_count())
                .map_err(|_| Error::InvalidEdit("too many pages"))?;
            if pages == 0 {
                continue;
            }
            working.inner.pages_mut().copy_page_range_from_document(&other.inner, 0..=pages - 1, position)?;
            position += pages;
        }
        let bytes: Arc<[u8]> = Arc::from(working.inner.save_to_bytes()?);
        Ok((self.revision(Arc::clone(&bytes))?, bytes))
    }

    /// Whether the document carries digital signatures, which only an
    /// incremental save keeps valid.
    pub fn is_signed(&self) -> bool {
        !self.inner.signatures().is_empty()
    }

    /// The document's own reading preferences: `/ViewerPreferences /Direction`
    /// and `/PageLayout`, where it states them (see [`crate::catalog`]).
    pub fn catalog_hints(&self) -> crate::catalog::CatalogHints {
        self.hints
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

    /// The document as PDF bytes, as PDFium would save it (with any edits).
    pub fn to_pdf_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.inner.save_to_bytes()?)
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

    /// Every page's label, or `None` when the document defines no labels.
    ///
    /// A `/PageLabels` tree must cover page 0, so a document whose first page
    /// has no label has none at all and costs one page load. Otherwise every
    /// page is loaded (PDFium reads labels only through a loaded page): call
    /// this off the UI thread.
    pub fn page_labels(&self) -> Result<Option<Vec<Option<String>>>> {
        if self.page_count() == 0 || self.page_label(0)?.is_none() {
            return Ok(None);
        }
        (0..self.page_count()).map(|index| self.page_label(index)).collect::<Result<Vec<_>>>().map(Some)
    }

    /// The page's text in logical reading order, with per-character geometry
    /// for selection (spec section 6.1). See [`crate::text`].
    pub fn page_text(&self, index: usize) -> Result<crate::text::PageText> {
        analyze_page(&self.page(index)?)
    }

    /// [`page_text`](Self::page_text) plus the mapping from its page-space
    /// geometry to the page as displayed (rotation, crop-box origin), from
    /// one page load.
    pub fn page_text_view(&self, index: usize) -> Result<(crate::text::PageText, ViewTransform)> {
        let page = self.page(index)?;
        Ok((analyze_page(&page)?, ViewTransform::of_page(&page)?))
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

fn analyze_page(page: &PdfPage<'_>) -> Result<crate::text::PageText> {
    let text = page.text()?;
    let glyphs = crate::text::collect_glyphs(&text);
    let spaces = crate::text::collect_space_marks(page, &text);
    Ok(crate::text::analyze(&glyphs, &spaces))
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
            .set_clear_color(PdfColor::WHITE)
            // PDFium's native order (BGRA): no swap, and what a BMP holds.
            .set_reverse_byte_order(false);
        page.render_into_bitmap_with_config(&mut bitmap, &config)?;
    }

    Bitmap::from_bgra(tile.width, tile.height, pixels)
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
