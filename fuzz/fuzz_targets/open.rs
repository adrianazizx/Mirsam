//! The app's open path: PDFium loads the bytes (after mirsam-core's own catalog
//! reader), then what the shell asks of a newly opened document — page sizes,
//! outline, labels, comments, form fields — and of its first pages: text in reading order,
//! annotations, text blocks, a small render.
//!
//! Needs the PDFium library: `scripts/fetch-pdfium.sh` (found through the
//! debug-assertions fallback, which cargo-fuzz builds keep) or `MIRSAM_PDFIUM_DIR`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mirsam_core::{Engine, PdfiumSource};

/// Whole-document reads load every page; skip them on files that claim many.
const MAX_PAGES_FOR_WHOLE_DOCUMENT: usize = 64;

fuzz_target!(|data: &[u8]| {
    let engine = Engine::global(&PdfiumSource::Auto)
        .expect("PDFium is available (run scripts/fetch-pdfium.sh or set MIRSAM_PDFIUM_DIR)");
    let Ok(document) = engine.open_bytes(data.to_vec(), None) else { return };
    let _ = document.catalog_hints();
    let _ = document.is_signed();
    let _ = document.outline();
    let count = document.page_count();
    if count <= MAX_PAGES_FOR_WHOLE_DOCUMENT {
        let _ = document.page_sizes();
        let _ = document.page_labels();
        let _ = document.comments();
        let _ = document.form();
    }
    for index in 0..count.min(2) {
        let _ = document.page_text_view(index);
        let _ = document.page_annotations(index);
        let _ = document.text_blocks(index);
        let _ = document.render_page(index, 0.05);
    }
});
