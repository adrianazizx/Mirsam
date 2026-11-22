//! Editing a paragraph of page text in place (ADR 0023), on real files: the
//! old text goes by byte range and nothing else in the page's content
//! changes, the new text lands where the old was, in the same z-order and
//! under the same clip, PDFium renders it, our extractor reads it back, and
//! `qpdf --check` passes. Spec §7's acceptance tests, in English and Arabic.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::manifest;
use safha_core::block_edit::{BlockEdit, EditedBlock, TextBlock};
use safha_core::fonts::FontFamily;
use safha_core::save::save;
use safha_core::text_edit::NewText;
use safha_core::typeset::{Align, BaseDirection};
use safha_core::{Bitmap, Document, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn fixture(name: &str) -> (Vec<u8>, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|f| f.name() == name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (std::fs::read(fixture.file()).expect("fixture readable"), fixture.password.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-block-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(bytes: &[u8], name: &str) {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write");
    let output = Command::new("qpdf").arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

fn is_dark(pixel: [u8; 4]) -> bool {
    pixel[0] < 100 && pixel[1] < 100 && pixel[2] < 100
}

/// Pixels that differ between two renders outside the view rectangle
/// (x0, y0, x1, y1), and dark pixels of `after` inside it.
fn changes_outside_and_ink_inside(
    before: &Bitmap,
    after: &Bitmap,
    (x0, y0, x1, y1): (f32, f32, f32, f32),
) -> (usize, usize) {
    let (mut outside, mut inside) = (0, 0);
    for y in 0..after.height() {
        for x in 0..after.width() {
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            let within = vx >= x0 && vx <= x1 && vy >= y0 && vy <= y1;
            let pixel = after.pixel(x, y).expect("pixel");
            if within {
                inside += usize::from(is_dark(pixel));
            } else if before.pixel(x, y) != Some(pixel) {
                outside += 1;
            }
        }
    }
    (outside, inside)
}

fn block_with(document: &Document, page: usize, needle: &str) -> TextBlock {
    let blocks = document.text_blocks(page).expect("blocks");
    blocks
        .iter()
        .find(|b| normalize(&b.text).contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "no block holds {needle:?}; blocks: {:#?}",
                blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
            )
        })
        .clone()
}

fn edit_to(
    document: &Document,
    block: &TextBlock,
    page: usize,
    text: &str,
) -> (Document, EditedBlock, Vec<u8>) {
    let edit = BlockEdit {
        page,
        index: block.index,
        fingerprint: block.fingerprint,
        text: text.into(),
        family: None,
    };
    let (edited, update, outcome) = document.with_block_edit(&edit).unwrap_or_else(|e| panic!("{e}"));
    (edited, outcome, update)
}

fn page_text(document: &Document, page: usize) -> String {
    normalize(&document.page_text(page).expect("text").text())
}

/// Embedded font programs in a document, after PDFium rewrote it without dead objects.
fn embedded_fonts(document: &Document) -> usize {
    let bytes = document.to_pdf_bytes().expect("rewrites");
    String::from_utf8_lossy(&bytes).matches("/FontFile").count()
}

fn add_text(document: &Document, text: &str, family: FontFamily, width: f32, y: f32) -> Document {
    add_text_on(document, 0, text, family, width, y)
}

fn add_text_on(
    document: &Document,
    page: usize,
    text: &str,
    family: FontFamily,
    width: f32,
    y: f32,
) -> Document {
    let new = NewText {
        page,
        x: 72.0,
        y,
        width,
        text: text.into(),
        family,
        size: 14.0,
        color: [0.0, 0.0, 0.0],
        align: Align::Start,
        direction: BaseDirection::Auto,
    };
    document.with_new_text(&new).expect("adds").0
}

// ------------------------------------------------------------------ English

#[test]
fn fixing_a_typo_in_english_keeps_the_font_and_moves_nothing_else() {
    let (bytes, _) = fixture("coretext/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let before = render(&document, 0);
    let blocks_before = document.text_blocks(0).expect("blocks");
    let block = block_with(&document, 0, "Questions? Email");
    assert!(block.editable && block.font_reusable, "{block:?}");

    let new = block.text.replace("call", "phone");
    let (edited, outcome, update) = edit_to(&document, &block, 0, &new);
    assert_eq!(outcome.substituted, None, "the paragraph's own font draws every letter");
    assert!(!String::from_utf8_lossy(&update).contains("FontFile"), "no font was embedded");

    // Only this paragraph's box changed; the page's other lines are where they were.
    let area = (block.x - 1.0, block.y - 1.0, 612.0, block.y + block.height + 1.0);
    let (outside, inside) = changes_outside_and_ink_inside(&before, &render(&edited, 0), area);
    assert_eq!(outside, 0, "pixels changed outside the paragraph's own line");
    assert!(inside > 100);
    let blocks_after = edited.text_blocks(0).expect("blocks");
    assert_eq!(blocks_after.len(), blocks_before.len());
    for (a, b) in blocks_before.iter().zip(&blocks_after) {
        if a.index != block.index {
            assert_eq!((a.x, a.y, a.width, &a.text), (b.x, b.y, b.width, &b.text));
        }
    }
    let read = page_text(&edited, 0);
    assert!(read.contains("or phone +1 (555) 010-0199") && !read.contains("or call"), "{read}");
    // The line starts where it did.
    let after = block_with(&edited, 0, "Questions? Email");
    assert!((after.x - block.x).abs() < 0.5 && (after.y - block.y).abs() < 0.5, "{block:?} -> {after:?}");
    qpdf_check(&edited.bytes(), "typo.pdf");
}

#[test]
fn a_two_line_paragraph_rewraps_inside_its_own_width() {
    let (bytes, _) = fixture("coretext/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let block = block_with(&document, 0, "The office staff");
    assert_eq!(block.lines, 2);
    // The same text again: nothing moves (the wrap is stable).
    let (same, _, _) = edit_to(&document, &block, 0, &block.text);
    let again = block_with(&same, 0, "The office staff");
    assert_eq!(again.lines, 2);
    assert!((again.width - block.width).abs() < 3.0, "{} vs {}", again.width, block.width);
    // A longer text grows downward inside the same width.
    let longer = format!(
        "{0} The client also asked for a second copy of the report, and for the appendix to be bound in the same cover as the rest. {0}",
        block.text
    );
    let (grown, _, _) = edit_to(&document, &block, 0, &longer);
    let after = block_with(&grown, 0, "The office staff");
    assert!(after.lines >= 3 && after.width <= block.width + 1.5, "{after:?}");
    assert!(page_text(&grown, 0).contains("bound in the same cover"));
}

#[test]
fn removing_a_paragraph_takes_its_text_away_and_only_its_text() {
    let (bytes, _) = fixture("coretext/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let before = page_text(&document, 0);
    let block = block_with(&document, 0, "Revenue rose");
    let (edited, ..) = edit_to(&document, &block, 0, "");
    let after = page_text(&edited, 0);
    assert!(before.contains("Revenue rose") && !after.contains("Revenue rose"), "{after}");
    assert!(after.contains("Questions? Email") && after.contains("Quarterly Report"));
    assert_eq!(
        edited.text_blocks(0).expect("blocks").len(),
        document.text_blocks(0).expect("blocks").len() - 1
    );
    qpdf_check(&edited.bytes(), "removed.pdf");
}

// ------------------------------------------------------------------- Arabic

const ARABIC: &str = "يستحق السداد خلال ثلاثين يوما من تاريخ الفاتورة، وتترتب على المتأخرات فائدة بالنسبة المحددة في العقد، ويحق للمورد إيقاف العمل إلى حين تسوية الرصيد كاملا.";

#[test]
fn inserting_a_word_in_the_middle_of_an_arabic_paragraph_rewraps_it_right_to_left() {
    let (bytes, _) = fixture("coretext/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let document = add_text(&original, ARABIC, FontFamily::Sans, 260.0, 560.0);
    let block = block_with(&document, 0, "يستحق السداد");
    // The paragraph is read back as it was typed: logical order, right to left, several lines.
    assert_eq!(normalize(&block.text), normalize(ARABIC));
    assert!(block.rtl && block.lines >= 3 && block.editable, "{block:?}");

    let new = ARABIC.replace("المتأخرات", "المتأخرات القانونية");
    let (edited, outcome, _) = edit_to(&document, &block, 0, &new);
    assert_eq!(outcome.substituted.as_ref().map(|s| s.to), Some(FontFamily::Sans));
    let after = block_with(&edited, 0, "القانونية");
    assert_eq!(normalize(&after.text), normalize(&new));
    assert!(after.rtl && after.lines >= block.lines);
    // The first line stays where it was, and the paragraph keeps its right edge.
    assert!((after.y - block.y).abs() < 1.5, "{} vs {}", after.y, block.y);
    assert!(((after.x + after.width) - (block.x + block.width)).abs() < 2.5, "{after:?} vs {block:?}");
    // Another viewer's extraction (our extractor, over PDFium) gives the logical text.
    assert!(page_text(&edited, 0).contains(&normalize(&new)), "{}", page_text(&edited, 0));
    // The paragraph's old subset is gone from the page, not kept beside the new one.
    assert!(embedded_fonts(&edited) <= embedded_fonts(&document), "duplicate fonts");
    qpdf_check(&edited.bytes(), "arabic.pdf");
}

#[test]
fn a_mixed_sentence_keeps_its_order() {
    let (bytes, _) = fixture("coretext/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let sentence = "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال للجهاز.";
    let document = add_text(&original, sentence, FontFamily::Sans, 400.0, 560.0);
    let block = block_with(&document, 0, "iPhone");
    assert_eq!(normalize(&block.text), sentence, "read back in logical order");
    let new = sentence.replace("3 أجهزة", "5 أجهزة").replace("4,500", "4,250");
    let (edited, ..) = edit_to(&document, &block, 0, &new);
    assert!(page_text(&edited, 0).contains(&new), "{}", page_text(&edited, 0));
    assert_eq!(normalize(&block_with(&edited, 0, "iPhone").text), new);
}

#[test]
fn open_edit_save_reopen_edit_again_without_drift_or_duplicate_fonts() {
    let (bytes, _) = fixture("coretext/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let document = add_text(&original, ARABIC, FontFamily::Naskh, 260.0, 560.0);
    let first = block_with(&document, 0, "يستحق السداد");

    let text1 = ARABIC.replace("ثلاثين", "ستين");
    let (edited, ..) = edit_to(&document, &first, 0, &text1);
    let one = block_with(&edited, 0, "ستين");
    let fonts_after_one = embedded_fonts(&edited);

    let target = scratch("roundtrip.pdf");
    save(&edited, &target).expect("saves");
    let reopened = engine().open_file(&target, None).expect("reopens");
    let two = block_with(&reopened, 0, "ستين");
    assert_eq!(normalize(&two.text), normalize(&text1));
    assert!(
        (two.x - one.x).abs() < 0.6 && (two.y - one.y).abs() < 0.6 && (two.width - one.width).abs() < 0.6,
        "drift after save and reopen: {one:?} -> {two:?}"
    );

    let text2 = text1.replace("ستين", "تسعين");
    let (again, ..) = edit_to(&reopened, &two, 0, &text2);
    let three = block_with(&again, 0, "تسعين");
    assert_eq!(normalize(&three.text), normalize(&text2));
    assert!((three.y - one.y).abs() < 1.5, "{three:?} vs {one:?}");
    assert!(embedded_fonts(&again) <= fonts_after_one, "duplicate fonts after the second edit");
    assert!(page_text(&again, 0).contains(&normalize(&text2)));
    qpdf_check(&again.bytes(), "roundtrip2.pdf");
}

#[test]
fn turned_pages_are_edited_as_they_are_displayed() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    let text = "تمت الموافقة على العقد بتاريخ ١٢ مارس ٢٠٢٦ من الإدارة المالية";
    for page in 0..4 {
        document = add_text_on(&document, page, text, FontFamily::Sans, 200.0, 40.0);
        let block = block_with(&document, page, "تمت الموافقة");
        assert_eq!(normalize(&block.text), text, "page {page}");
        let new = text.replace("الإدارة المالية", "الإدارة العامة");
        let before = render(&document, page);
        let (edited, ..) = edit_to(&document, &block, page, &new);
        let after = block_with(&edited, page, "العامة");
        assert_eq!(normalize(&after.text), new, "page {page}");
        assert!(
            (after.y - block.y).abs() < 1.5
                && ((after.x + after.width) - (block.x + block.width)).abs() < 2.5,
            "page {page}: {block:?} -> {after:?}"
        );
        // Upright and in its box on the displayed page: the old ink is gone, the new is where it was.
        let area = (block.x - 2.0, block.y - 2.0, block.x + block.width + 2.0, block.y + block.height + 2.0);
        let (outside, inside) = changes_outside_and_ink_inside(&before, &render(&edited, page), area);
        assert_eq!(outside, 0, "page {page}: pixels changed outside the paragraph");
        assert!(inside > 200, "page {page}");
        document = edited;
    }
    qpdf_check(&document.bytes(), "turned.pdf");
}

// ---------------------------------------------------- z-order, clip, state

/// A one-page file with the given content and a Helvetica with metrics.
fn pdf(content: &str) -> Vec<u8> {
    let widths = vec!["556"; 95].join(" ");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 400] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths [{widths}] >>"),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    pdf
}

#[test]
fn new_text_keeps_the_old_text_z_order_and_clip_on_a_page_that_leaves_its_state_changed() {
    // The page scales by 2 and never restores it. The text is clipped to a
    // box and a blue rectangle is drawn over it afterwards.
    let content = "2 0 0 2 0 0 cm\nq 10 140 80 35 re W n\nBT /F1 12 Tf 20 150 Td (Hello world) Tj ET\nQ\n0 0 1 rg 60 150 40 12 re f\n";
    let document = engine().open_bytes(pdf(content), None).expect("opens");
    let block = block_with(&document, 0, "Hello world");
    assert!(block.editable && block.font_reusable, "{block:?}");
    assert!((block.size - 24.0).abs() < 0.5, "size as displayed: {}", block.size);

    let (edited, outcome, _) = edit_to(&document, &block, 0, "Hello wonderful world today");
    assert_eq!(outcome.substituted, None);
    let image = render(&edited, 0);
    // z-order: the blue rectangle (page 120..200 x 300..324, displayed y 76..100) is still on top.
    let blue =
        |x: f32, y: f32| image.pixel((x * SCALE) as u32, (y * SCALE) as u32).map(|p| (p[0], p[1], p[2]));
    assert_eq!(blue(160.0, 90.0), Some((0, 0, 255)), "the rectangle still covers the text");
    // Clip: nothing dark right of the clip box (page x 180, displayed 180) in the text's band.
    let mut ink_right_of_clip = 0;
    let mut ink_inside_clip = 0;
    for y in (60.0 * SCALE) as u32..(110.0 * SCALE) as u32 {
        for x in 0..image.width() {
            if image.pixel(x, y).is_some_and(is_dark) {
                if x as f32 / SCALE > 181.0 {
                    ink_right_of_clip += 1;
                } else {
                    ink_inside_clip += 1;
                }
            }
        }
    }
    assert_eq!(ink_right_of_clip, 0, "the new text is clipped like the old");
    assert!(ink_inside_clip > 100, "and drawn where the old was");
    assert!(page_text(&edited, 0).contains("Hello wonderful world today"));
    qpdf_check(&edited.bytes(), "clip.pdf");
}

#[test]
fn text_drawn_with_kerning_and_line_operators_is_read_and_replaced_cleanly() {
    // TJ with adjustments, `'` and a second line, then a later show whose
    // position depends on the first's advance (no repositioning).
    let content = "BT /F1 10 Tf 12 TL 50 300 Td [(Hel) -80 (lo) 120 ( there)] TJ (friend of mine) ' ET\nBT /F1 10 Tf 50 200 Td (kept) Tj ( along) Tj ET\n";
    let document = engine().open_bytes(pdf(content), None).expect("opens");
    let blocks = document.text_blocks(0).expect("blocks");
    assert_eq!(blocks.len(), 2, "{:#?}", blocks.iter().map(|b| &b.text).collect::<Vec<_>>());
    let block = block_with(&document, 0, "Hello there");
    assert_eq!(normalize(&block.text), "Hello there friend of mine");
    let other = block_with(&document, 0, "kept along");
    let (edited, ..) = edit_to(&document, &block, 0, "Goodbye");
    let after = edited.text_blocks(0).expect("blocks");
    let kept = after.iter().find(|b| b.text.contains("kept")).expect("the other paragraph");
    assert_eq!((kept.x, kept.y, kept.width), (other.x, other.y, other.width));
    assert!(page_text(&edited, 0).contains("Goodbye") && !page_text(&edited, 0).contains("Hello"));
}

// ------------------------------------------------------------------ refusals

#[test]
fn an_edit_of_text_that_changed_is_refused() {
    let (bytes, _) = fixture("coretext/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let block = block_with(&document, 0, "Questions? Email");
    let stale = BlockEdit {
        page: 0,
        index: block.index,
        fingerprint: block.fingerprint ^ 1,
        text: "x".into(),
        family: None,
    };
    assert!(matches!(document.with_block_edit(&stale), Err(Error::InvalidEdit(_))));
    let missing = BlockEdit { index: 99, ..stale };
    assert!(matches!(document.with_block_edit(&missing), Err(Error::InvalidEdit(_))));
}

#[test]
fn encrypted_files_and_unreadable_fonts_are_not_offered_for_editing() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    assert!(matches!(document.text_blocks(0), Err(Error::EditUnsupported(_))));

    // CoreText's Arabic draws dotless letters with the dots as separate shapes
    // that carry no text: not read, so not offered.
    let (bytes, _) = fixture("coretext/ar-plain");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let blocks = document.text_blocks(0).expect("blocks");
    assert!(
        !blocks.is_empty() && blocks.iter().all(|b| !b.editable),
        "{:?}",
        blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    );
}
