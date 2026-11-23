//! Text extraction against the fixture corpus' ground truth (docs/SPEC.md 6.1).
//!
//! Each fixture's content file lists its blocks in logical order. A block
//! counts as extracted when it appears verbatim (NFC, whitespace collapsed) in
//! `Document::page_text`. The numbers below are floors measured when the
//! extractor was written: raising them is progress, falling below is a
//! regression. Run `cargo run -p mirsam-core --example fixture_report --
//! --verbose` to see every miss.

mod common;

use common::manifest;
use mirsam_core::text::{Direction, TextQuality};
use mirsam_core::{Engine, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

/// Fixtures whose producer wrote a broken glyph-to-Unicode mapping (CoreText
/// with Noto Naskh draws each dot as its own glyph and maps it to `#`, `%`, …;
/// with Amiri, marks land on ASCII codes). No extractor can recover their
/// Arabic text; the requirement is that we notice.
const BROKEN_MAPPING: &[&str] = &[
    "coretext/ar-lam-alef",
    "coretext/ar-plain",
    "coretext/ar-vocalized",
    "coretext/contract-two-column",
    "coretext/mixed-inline",
    "coretext/table-bilingual",
];

/// (fixture, blocks that must be extracted verbatim)
const FLOORS: &[(&str, usize)] = &[
    ("chrome/ar-lam-alef", 5),
    ("chrome/ar-plain", 4),
    ("chrome/ar-sans", 3),
    ("chrome/ar-vocalized", 4),
    ("chrome/contract-two-column", 10),
    ("chrome/en-paragraphs", 4),
    // "(64-bit)" inside an Arabic line comes out "(bit-64)": the two are drawn
    // identically, and "INV-2026" in the same file needs the other reading (ADR 0008).
    ("chrome/mixed-inline", 7),
    ("chrome/table-bilingual", 17),
    ("coretext/ar-sans", 3),
    ("coretext/en-paragraphs", 4),
    ("qpdf/linearized", 4),
    ("qpdf/encrypted", 4),
];

#[test]
fn extraction_meets_the_recorded_floors() {
    let manifest = manifest();
    let mut failures = Vec::new();
    for &(name, floor) in FLOORS {
        // CoreText fixtures exist only where they could be made (macOS).
        let Some(fixture) = manifest.fixture_or_unavailable(name) else { continue };
        let document = engine().open_file(&fixture.file(), fixture.password.as_deref()).expect("opens");
        let page = document.page_text(0).expect("extracts");
        let text = common::comparable(&page.text());
        let found =
            fixture.expected.iter().filter(|block| text.contains(&common::comparable(&block.text))).count();
        if found < floor {
            failures.push(format!("{name}: {found}/{} blocks, floor {floor}", fixture.expected.len()));
        }
        assert_eq!(page.quality, TextQuality::Good, "{name} must not be flagged");
    }
    assert!(failures.is_empty(), "extraction regressed:\n  {}", failures.join("\n  "));
}

#[test]
fn broken_mappings_are_flagged_for_ocr() {
    let manifest = manifest();
    for &name in BROKEN_MAPPING {
        let Some(fixture) = manifest.fixture_or_unavailable(name) else { continue };
        let document = engine().open_file(&fixture.file(), None).expect("opens");
        let quality = document.page_text(0).expect("extracts").quality;
        assert!(matches!(quality, TextQuality::Suspect { .. }), "{name}: {quality:?}");
    }
}

#[test]
fn page_and_block_directions_follow_the_content() {
    let manifest = manifest();
    let open = |name: &str| {
        let fixture = manifest.fixtures.iter().find(|f| f.name() == name).expect("fixture exists");
        engine().open_file(&fixture.file(), None).expect("opens").page_text(0).expect("extracts")
    };
    assert_eq!(open("chrome/ar-plain").direction, Direction::Rtl);
    assert_eq!(open("chrome/en-paragraphs").direction, Direction::Ltr);

    // The mixed page has right-to-left paragraphs and left-to-right ones.
    let mixed = open("chrome/mixed-inline");
    let directions: Vec<Direction> = mixed.blocks.iter().map(|b| b.direction).collect();
    assert!(directions.contains(&Direction::Rtl) && directions.contains(&Direction::Ltr), "{directions:?}");
}

#[test]
fn spans_cover_each_line_and_map_to_glyph_boxes() {
    let manifest = manifest();
    let fixture =
        manifest.fixtures.iter().find(|f| f.name() == "chrome/mixed-inline").expect("fixture exists");
    let page = engine().open_file(&fixture.file(), None).expect("opens").page_text(0).expect("extracts");
    for line in page.blocks.iter().flat_map(|block| &block.lines) {
        let mut end = 0;
        for span in &line.spans {
            assert_eq!(span.start, end, "spans are contiguous in {:?}", line.text);
            assert!(span.end > span.start);
            end = span.end;
            // Every span is a glyph with a box, except inferred spaces.
            assert_eq!(span.rect.is_none(), &line.text[span.start..span.end] == " ");
        }
        assert_eq!(end, line.text.len());
    }
}

#[test]
fn image_only_and_damaged_pages_do_not_panic() {
    let manifest = manifest();
    for fixture in manifest.fixtures.iter().filter(|f| f.damaged) {
        if let Ok(document) = engine().open_file(&fixture.file(), None) {
            for page in 0..document.page_count() {
                let _ = document.page_text(page);
            }
        }
    }
}

#[test]
fn a_thousand_page_document_extracts_page_by_page_quickly() {
    let manifest = manifest();
    let fixture = manifest.fixtures.iter().find(|f| f.id == "bilingual-1000-pages").expect("fixture exists");
    let document = engine().open_file(&fixture.file(), None).expect("opens");
    let started = std::time::Instant::now();
    for page in [0, 1, 500, 999] {
        assert!(!document.page_text(page).expect("extracts").blocks.is_empty());
    }
    let per_page = started.elapsed() / 4;
    println!("page_text: {per_page:?} per page (debug build)");
    // Generous ceiling for a debug build on CI; release is much faster.
    assert!(per_page.as_millis() < 500, "{per_page:?} per page");
}
