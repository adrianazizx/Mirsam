//! Route A: everything through PDFium's own edit API (fpdf_edit.h).
//!
//! Most of this file uses pdfium-render's raw bindings, because the content-mark
//! calls (`FPDFPageObj_AddMark`, `FPDFPageObjMark_Set*Param`) have no safe wrapper
//! in 0.9.6 and the crate keeps its FPDF_* handles private, so raw and safe calls
//! cannot be mixed on one object. `write_safe` at the end repeats the A4 recipe
//! with the safe wrappers alone (`PdfFonts::load_cid_type2_from_bytes`,
//! `PdfPageTextObject::set_charcodes` / `set_positions`) to show it needs no `unsafe`.

use std::os::raw::{c_int, c_ulong, c_void};

use pdfium_render::prelude::*;

use crate::Layout;
use crate::fontinfo::FontInfo;
use crate::pdfparts::{ActualText, Embed, ToUnicodePolicy, plan_segments};
use crate::shape::ShapedLine;

/// fpdf_edit.h; pdfium-render re-exports the FPDF_* types but not this constant.
const FPDF_FONT_TRUETYPE: c_int = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActualTextEncoding {
    /// `FPDFPageObjMark_SetBlobParam` with UTF-16BE + BOM bytes.
    Utf16Blob,
    /// `FPDFPageObjMark_SetStringParam`; the binding takes a C string, so the only
    /// thing it can carry is UTF-8 (here with a UTF-8 BOM, a PDF 2.0 text string).
    Utf8String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A1: `FPDFText_LoadFont(cid)` + `FPDFText_SetText(logical string)`.
    SetText,
    /// A2: `FPDFText_LoadFont(cid)` + `FPDFText_SetCharcodes(shaped glyph IDs)`,
    /// one text object per line, PDFium's own advances.
    Charcodes,
    /// A3: as A2 plus `FPDFText_SetPositions` for x and one text object per
    /// stretch of equal y offset.
    CharcodesPositioned,
    /// A4: `FPDFText_LoadCidType2Font(our subset, our ToUnicode, our CIDToGIDMap)`,
    /// positioned like A3, plus `/Span` content marks carrying /ActualText.
    CidType2 { actual_text: ActualText, encoding: ActualTextEncoding },
}

#[derive(Debug, Default)]
pub struct Stats {
    /// Text objects created, per page.
    pub objects: Vec<usize>,
    /// API calls that returned failure.
    pub failures: Vec<String>,
}

/// FPDF_FILEWRITE with the output buffer behind it (PDFium hands the struct back).
#[repr(C)]
struct Sink {
    base: FPDF_FILEWRITE,
    buf: Vec<u8>,
}

unsafe extern "C" fn write_block(this: *mut FPDF_FILEWRITE, data: *const c_void, size: c_ulong) -> c_int {
    // SAFETY: PDFium passes back the pointer we gave it, which is the first field
    // of a live `Sink`, and `data` is valid for `size` bytes for this call.
    unsafe {
        let sink = &mut *(this as *mut Sink);
        sink.buf.extend_from_slice(std::slice::from_raw_parts(data as *const u8, size as usize));
    }
    1
}

pub fn write(
    pdfium: &Pdfium,
    font: &FontInfo,
    lines: &[ShapedLine],
    mode: Mode,
    layout: &Layout,
) -> Result<(Vec<u8>, Stats), Box<dyn std::error::Error>> {
    let b = pdfium.bindings();
    let embed = Embed::build(font, lines, ToUnicodePolicy::Source)?;
    let mut stats = Stats::default();
    let size = layout.font_size;
    let pt = |units: f64| units / font.upem as f64 * size as f64;

    // SAFETY: plain FFI. Every handle is checked for null before use, every pointer
    // passed in outlives its call, and handles are closed exactly once below.
    unsafe {
        let doc = b.FPDF_CreateNewDocument();
        if doc.is_null() {
            return Err("FPDF_CreateNewDocument failed".into());
        }
        let font_handle = match mode {
            Mode::CidType2 { .. } => {
                // CID == glyph ID of the subset: an explicit identity table.
                let cid_to_gid: Vec<u8> = (0..embed.n_cids).flat_map(u16::to_be_bytes).collect();
                b.FPDFText_LoadCidType2Font(
                    doc,
                    embed.subset.as_ptr(),
                    embed.subset.len() as u32,
                    &embed.to_unicode_cmap(),
                    cid_to_gid.as_ptr(),
                    cid_to_gid.len() as u32,
                )
            }
            _ => b.FPDFText_LoadFont(doc, font.data.as_ptr(), font.data.len() as u32, FPDF_FONT_TRUETYPE, 1),
        };
        if font_handle.is_null() {
            b.FPDF_CloseDocument(doc);
            return Err(format!("font load failed for {mode:?}").into());
        }
        // In A1-A3 the whole font is embedded, so the charcode is the original glyph ID.
        let code = |gid: u16| match mode {
            Mode::CidType2 { .. } => embed.cid(gid) as u32,
            _ => gid as u32,
        };

        for (page_index, line) in lines.iter().enumerate() {
            let page = b.FPDFPage_New(doc, page_index as c_int, layout.page_w as f64, layout.page_h as f64);
            if page.is_null() {
                return Err("FPDFPage_New failed".into());
            }
            let mut objects = 0;
            match mode {
                Mode::SetText => {
                    let obj = b.FPDFPageObj_CreateTextObj(doc, font_handle, size);
                    if b.FPDFText_SetText_str(obj, &line.text) == 0 {
                        stats.failures.push(format!("page {page_index}: FPDFText_SetText"));
                    }
                    // Right-align using whatever width PDFium laid the string out to.
                    let (mut l, mut bt, mut r, mut t) = (0.0, 0.0, 0.0, 0.0);
                    b.FPDFPageObj_GetBounds(obj, &mut l, &mut bt, &mut r, &mut t);
                    let x = if line.base_rtl { layout.page_w - layout.margin - r } else { layout.margin };
                    b.FPDFPageObj_Transform(obj, 1.0, 0.0, 0.0, 1.0, x as f64, layout.baseline as f64);
                    b.FPDFPage_InsertObject(page, obj);
                    objects += 1;
                }
                Mode::Charcodes => {
                    let codes: Vec<u32> = line.glyphs.iter().map(|g| code(g.gid)).collect();
                    let obj = b.FPDFPageObj_CreateTextObj(doc, font_handle, size);
                    if b.FPDFText_SetCharcodes(obj, codes.as_ptr(), codes.len()) == 0 {
                        stats.failures.push(format!("page {page_index}: FPDFText_SetCharcodes"));
                    }
                    // Without positions the viewer uses hmtx advances, not the shaper's.
                    let natural: f64 = line.glyphs.iter().map(|g| font.advance(g.gid) as f64).sum();
                    let x = if line.base_rtl {
                        (layout.page_w - layout.margin) as f64 - pt(natural)
                    } else {
                        layout.margin as f64
                    };
                    b.FPDFPageObj_Transform(obj, 1.0, 0.0, 0.0, 1.0, x, layout.baseline as f64);
                    b.FPDFPage_InsertObject(page, obj);
                    objects += 1;
                }
                Mode::CharcodesPositioned | Mode::CidType2 { .. } => {
                    let (at_mode, encoding) = match mode {
                        Mode::CidType2 { actual_text, encoding } => (actual_text, encoding),
                        _ => (ActualText::None, ActualTextEncoding::Utf16Blob),
                    };
                    let target = Target { doc, page, font: font_handle };
                    let at = (layout.line_x(font, line), layout.baseline as f64);
                    let codes: Vec<u32> = line.glyphs.iter().map(|g| code(g.gid)).collect();
                    objects += place_positioned(
                        b, &target, font, &embed, line, &codes, at_mode, encoding, size, at, &mut stats,
                    );
                }
            }
            if b.FPDFPage_GenerateContent(page) == 0 {
                stats.failures.push(format!("page {page_index}: FPDFPage_GenerateContent"));
            }
            b.FPDF_ClosePage(page);
            stats.objects.push(objects);
        }

        let mut sink =
            Sink { base: FPDF_FILEWRITE { version: 1, WriteBlock: Some(write_block) }, buf: Vec::new() };
        let saved = b.FPDF_SaveAsCopy(doc, &mut sink.base, 0);
        b.FPDFFont_Close(font_handle);
        b.FPDF_CloseDocument(doc);
        if saved == 0 {
            return Err("FPDF_SaveAsCopy failed".into());
        }
        Ok((sink.buf, stats))
    }
}

struct Target {
    doc: FPDF_DOCUMENT,
    page: FPDF_PAGE,
    font: FPDF_FONT,
}

/// Places one shaped line as text objects: x through `FPDFText_SetPositions`, and a
/// new object at every change of y offset and at every /ActualText span boundary.
/// Returns the number of objects inserted.
///
/// SAFETY: the handles in `target` must be live and belong together.
#[allow(clippy::too_many_arguments)]
unsafe fn place_positioned(
    b: &dyn PdfiumLibraryBindings,
    target: &Target,
    font: &FontInfo,
    embed: &Embed,
    line: &ShapedLine,
    codes: &[u32],
    at_mode: ActualText,
    encoding: ActualTextEncoding,
    size: f32,
    (x0, baseline): (f64, f64),
    stats: &mut Stats,
) -> usize {
    let pt = |units: f64| units / font.upem as f64 * size as f64;
    let pens = line.pen_positions();
    let draw_x = |i: usize| (pens[i] + line.glyphs[i].x_offset) as f64;
    let mut objects = 0;

    for seg in plan_segments(line, embed, at_mode) {
        let mut span_mark: Option<FPDF_PAGEOBJECTMARK> = None;
        // PDFium positions are x-only, so every change of y offset (GPOS mark
        // attachment) starts a new text object.
        let mut start = seg.glyphs.start;
        while start < seg.glyphs.end {
            let y = line.glyphs[start].y_offset;
            let mut end = start + 1;
            while end < seg.glyphs.end && line.glyphs[end].y_offset == y {
                end += 1;
            }
            // SAFETY: see the function contract; slices outlive the calls.
            unsafe {
                let obj = b.FPDFPageObj_CreateTextObj(target.doc, target.font, size);
                if b.FPDFText_SetCharcodes(obj, codes[start..end].as_ptr(), end - start) == 0 {
                    stats.failures.push("FPDFText_SetCharcodes".into());
                }
                if end - start > 1 {
                    let positions: Vec<f32> =
                        (start + 1..end).map(|i| pt(draw_x(i) - draw_x(start)) as f32).collect();
                    if b.FPDFText_SetPositions(obj, positions.as_ptr(), positions.len()) == 0 {
                        stats.failures.push("FPDFText_SetPositions".into());
                    }
                }
                if let Some(text) = &seg.actual_text {
                    match span_mark {
                        Some(mark) => {
                            b.FPDFPageObj_AddExistingMark(obj, mark);
                        }
                        None => {
                            let mark = b.FPDFPageObj_AddMark(obj, "Span");
                            let ok = match encoding {
                                ActualTextEncoding::Utf16Blob => {
                                    let mut bytes = vec![0xFE, 0xFF];
                                    bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
                                    b.FPDFPageObjMark_SetBlobParam(
                                        target.doc,
                                        obj,
                                        mark,
                                        "ActualText",
                                        bytes.as_ptr(),
                                        bytes.len() as c_ulong,
                                    )
                                }
                                ActualTextEncoding::Utf8String => b.FPDFPageObjMark_SetStringParam(
                                    target.doc,
                                    obj,
                                    mark,
                                    "ActualText",
                                    &format!("\u{FEFF}{text}"),
                                ),
                            };
                            if ok == 0 {
                                stats.failures.push("set ActualText".into());
                            }
                            span_mark = Some(mark);
                        }
                    }
                }
                b.FPDFPageObj_Transform(
                    obj,
                    1.0,
                    0.0,
                    0.0,
                    1.0,
                    x0 + pt(draw_x(start)),
                    baseline + pt(y as f64),
                );
                b.FPDFPage_InsertObject(target.page, obj);
            }
            objects += 1;
            start = end;
        }
    }
    objects
}

/// Width and height of the first page, as PDFium sees it.
pub fn first_page_size(
    pdfium: &Pdfium,
    existing: &[u8],
    password: Option<&str>,
) -> Result<(f32, f32), Box<dyn std::error::Error>> {
    let b = pdfium.bindings();
    // SAFETY: plain FFI; `existing` outlives the document handle.
    unsafe {
        let doc = b.FPDF_LoadMemDocument(existing, password);
        if doc.is_null() {
            return Err(format!("FPDF_LoadMemDocument failed (error {})", b.FPDF_GetLastError()).into());
        }
        let page = b.FPDF_LoadPage(doc, 0);
        let size = if page.is_null() {
            None
        } else {
            Some((b.FPDF_GetPageWidthF(page), b.FPDF_GetPageHeightF(page)))
        };
        if !page.is_null() {
            b.FPDF_ClosePage(page);
        }
        b.FPDF_CloseDocument(doc);
        size.ok_or_else(|| "FPDF_LoadPage failed".into())
    }
}

/// The A4 recipe applied to the first page of an existing PDF: load it with PDFium,
/// insert the text objects, regenerate the page content, save.
#[allow(clippy::too_many_arguments)]
pub fn insert_into(
    pdfium: &Pdfium,
    existing: &[u8],
    password: Option<&str>,
    font: &FontInfo,
    line: &ShapedLine,
    size: f32,
    at: (f64, f64),
    incremental: bool,
) -> Result<(Vec<u8>, Stats), Box<dyn std::error::Error>> {
    const FPDF_INCREMENTAL: FPDF_DWORD = 1;
    const FPDF_NO_INCREMENTAL: FPDF_DWORD = 2;
    let b = pdfium.bindings();
    let embed = Embed::build(font, std::slice::from_ref(line), ToUnicodePolicy::Source)?;
    let mut stats = Stats::default();

    // SAFETY: plain FFI. `existing` outlives the document (PDFium reads from it
    // lazily), handles are null-checked and closed once.
    unsafe {
        let doc = b.FPDF_LoadMemDocument(existing, password);
        if doc.is_null() {
            return Err(format!("FPDF_LoadMemDocument failed (error {})", b.FPDF_GetLastError()).into());
        }
        let page = b.FPDF_LoadPage(doc, 0);
        if page.is_null() {
            b.FPDF_CloseDocument(doc);
            return Err("FPDF_LoadPage failed".into());
        }
        let cid_to_gid: Vec<u8> = (0..embed.n_cids).flat_map(u16::to_be_bytes).collect();
        let font_handle = b.FPDFText_LoadCidType2Font(
            doc,
            embed.subset.as_ptr(),
            embed.subset.len() as u32,
            &embed.to_unicode_cmap(),
            cid_to_gid.as_ptr(),
            cid_to_gid.len() as u32,
        );
        if font_handle.is_null() {
            b.FPDF_ClosePage(page);
            b.FPDF_CloseDocument(doc);
            return Err("FPDFText_LoadCidType2Font failed".into());
        }
        let codes: Vec<u32> = line.glyphs.iter().map(|g| embed.cid(g.gid) as u32).collect();
        let target = Target { doc, page, font: font_handle };
        let objects = place_positioned(
            b,
            &target,
            font,
            &embed,
            line,
            &codes,
            ActualText::None,
            ActualTextEncoding::Utf16Blob,
            size,
            at,
            &mut stats,
        );
        stats.objects.push(objects);
        if b.FPDFPage_GenerateContent(page) == 0 {
            stats.failures.push("FPDFPage_GenerateContent".into());
        }
        b.FPDF_ClosePage(page);

        let mut sink =
            Sink { base: FPDF_FILEWRITE { version: 1, WriteBlock: Some(write_block) }, buf: Vec::new() };
        let flags = if incremental { FPDF_INCREMENTAL } else { FPDF_NO_INCREMENTAL };
        let saved = b.FPDF_SaveAsCopy(doc, &mut sink.base, flags);
        b.FPDFFont_Close(font_handle);
        b.FPDF_CloseDocument(doc);
        if saved == 0 {
            return Err("FPDF_SaveAsCopy failed".into());
        }
        Ok((sink.buf, stats))
    }
}

/// The A4 recipe again, without `/ActualText`, through pdfium-render's safe
/// wrappers only: no `unsafe`, no raw handles. One text object per bidi run and
/// per stretch of equal y offset, exactly like `Mode::CidType2`.
pub fn write_safe(
    pdfium: &Pdfium,
    font: &FontInfo,
    lines: &[ShapedLine],
    layout: &Layout,
) -> Result<(Vec<u8>, Stats), Box<dyn std::error::Error>> {
    let embed = Embed::build(font, lines, ToUnicodePolicy::Source)?;
    let mut stats = Stats::default();
    let size = layout.font_size;
    let pt = |units: f64| (units / font.upem as f64 * size as f64) as f32;

    let mut doc = pdfium.create_new_pdf()?;
    let cid_to_gid: Vec<u8> = (0..embed.n_cids).flat_map(u16::to_be_bytes).collect();
    let token =
        doc.fonts_mut().load_cid_type2_from_bytes(&embed.subset, &embed.to_unicode_cmap(), &cid_to_gid)?;

    for line in lines {
        let paper =
            PdfPagePaperSize::new_custom(PdfPoints::new(layout.page_w), PdfPoints::new(layout.page_h));
        let mut page = doc.pages_mut().create_page_at_end(paper)?;
        page.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
        let pens = line.pen_positions();
        let draw_x = |i: usize| (pens[i] + line.glyphs[i].x_offset) as f64;
        let x0 = layout.line_x(font, line) as f32;
        let mut objects = 0;

        for seg in plan_segments(line, &embed, ActualText::None) {
            let mut start = seg.glyphs.start;
            while start < seg.glyphs.end {
                let y = line.glyphs[start].y_offset;
                let mut end = start + 1;
                while end < seg.glyphs.end && line.glyphs[end].y_offset == y {
                    end += 1;
                }
                let codes: Vec<u32> = (start..end).map(|i| embed.cid(line.glyphs[i].gid) as u32).collect();
                let mut object = PdfPageTextObject::new(&doc, "", token, PdfPoints::new(size))?;
                object.set_charcodes(&codes)?;
                if codes.len() > 1 {
                    let positions: Vec<f32> =
                        (start + 1..end).map(|i| pt(draw_x(i) - draw_x(start))).collect();
                    object.set_positions(&positions)?;
                }
                object.translate(
                    PdfPoints::new(x0 + pt(draw_x(start))),
                    PdfPoints::new(layout.baseline + pt(y as f64)),
                )?;
                page.objects_mut().add_text_object(object)?;
                objects += 1;
                start = end;
            }
        }
        page.regenerate_content()?;
        stats.objects.push(objects);
    }
    Ok((doc.save_to_bytes()?, stats))
}
