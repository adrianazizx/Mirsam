//! End-to-end checks of the Milestone 0 engine surface against a PDF built in
//! memory, so the tests need no fixture files.

use safha_core::{Engine, Error, PdfiumSource, TileRequest};

/// A one-page 200x100 pt PDF: a red rectangle over the bottom-left quarter and
/// the word "Hello" in Helvetica near the top.
fn minimal_pdf() -> Vec<u8> {
    let content = "1 0 0 rg 0 0 100 50 re f\nBT /F1 12 Tf 0 g 110 70 Td (Hello) Tj ET\n";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];

    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref_at = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objects.len() + 1, xref_at)
            .as_bytes(),
    );
    pdf
}

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

const RED: [u8; 4] = [255, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn reports_page_geometry() {
    let doc = engine().open_bytes(minimal_pdf(), None).expect("opens");
    assert_eq!(doc.page_count(), 1);
    let size = doc.page_size(0).expect("page 0 exists");
    assert_eq!((size.width, size.height), (200.0, 100.0));
}

#[test]
fn renders_page_as_top_left_origin_rgba() {
    let doc = engine().open_bytes(minimal_pdf(), None).expect("opens");
    let bitmap = doc.render_page(0, 2.0).expect("renders");
    assert_eq!((bitmap.width(), bitmap.height()), (400, 200));
    assert_eq!(bitmap.as_rgba().len(), 400 * 200 * 4);
    // PDF's bottom-left quarter is the bitmap's bottom-left quarter. Red (not blue)
    // here also proves the channel order is RGBA rather than PDFium's native BGRA.
    assert_eq!(bitmap.pixel(20, 180), Some(RED));
    assert_eq!(bitmap.pixel(380, 180), Some(WHITE));
    assert_eq!(bitmap.pixel(20, 20), Some(WHITE));
}

#[test]
fn tile_matches_the_same_region_of_the_full_render() {
    let doc = engine().open_bytes(minimal_pdf(), None).expect("opens");
    let full = doc.render_page(0, 2.0).expect("renders");
    let tile = TileRequest { scale: 2.0, x: 150, y: 60, width: 120, height: 100 };
    let part = doc.render_tile(0, tile).expect("renders tile");
    assert_eq!((part.width(), part.height()), (120, 100));
    for y in 0..tile.height {
        for x in 0..tile.width {
            assert_eq!(part.pixel(x, y), full.pixel(tile.x + x, tile.y + y), "pixel ({x}, {y})");
        }
    }
    // The tile straddles the rectangle's edge, so it is not trivially uniform.
    assert_eq!(part.pixel(10, 90), Some(RED));
    assert_eq!(part.pixel(110, 90), Some(WHITE));
}

#[test]
fn tile_must_lie_inside_the_page() {
    let doc = engine().open_bytes(minimal_pdf(), None).expect("opens");
    let overhanging = TileRequest { scale: 1.0, x: 150, y: 50, width: 100, height: 50 };
    assert!(matches!(doc.render_tile(0, overhanging), Err(Error::InvalidRenderRequest(_))));
    let flush = TileRequest { scale: 1.0, x: 150, y: 50, width: 50, height: 50 };
    assert!(doc.render_tile(0, flush).is_ok());
}

#[test]
fn extracts_raw_text() {
    let doc = engine().open_bytes(minimal_pdf(), None).expect("opens");
    assert_eq!(doc.page_text_raw(0).expect("has text").trim(), "Hello");
}

#[test]
fn rejects_bad_requests_without_panicking() {
    let doc = engine().open_bytes(minimal_pdf(), None).expect("opens");
    assert!(matches!(doc.page_size(1), Err(Error::PageOutOfRange { index: 1, count: 1 })));
    assert!(matches!(doc.render_page(0, 0.0), Err(Error::InvalidRenderRequest(_))));
    assert!(matches!(doc.render_page(0, f32::NAN), Err(Error::InvalidRenderRequest(_))));
    let huge = TileRequest { scale: 1.0, x: 0, y: 0, width: 100_000, height: 10 };
    assert!(matches!(doc.render_tile(0, huge), Err(Error::InvalidRenderRequest(_))));
}

#[test]
fn rejects_non_pdf_bytes() {
    let result = engine().open_bytes(b"this is not a pdf".to_vec(), None);
    assert!(matches!(result, Err(Error::InvalidPdf)), "{result:?}");
}
