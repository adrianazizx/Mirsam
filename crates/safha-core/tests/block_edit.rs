//! Editing a paragraph of page text in place (ADR 0023), on real files: the
//! old text goes by byte range and nothing else in the page's content
//! changes, the new text lands where the old was, in the same z-order and
//! under the same clip, PDFium renders it, our extractor reads it back, and
//! `qpdf --check` passes. Spec §7's acceptance tests, in English and Arabic.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::manifest;
use safha_core::block_edit::{BlockEdit, EditedBlock, FontSource, Mark, Refusal, TextBlock};
use safha_core::fonts::FontFamily;
use safha_core::save::save;
use safha_core::text_edit::NewText;
use safha_core::typeset::{Align, BaseDirection};
use safha_core::{Bitmap, Document, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

/// A fixture's bytes and password. `None` only for a producer the corpus
/// records as unavailable here (CoreText exists only on macOS), which is
/// reported; any other missing fixture fails the test.
fn fixture(name: &str) -> Option<(Vec<u8>, Option<String>)> {
    let manifest = manifest();
    let fixture = manifest.fixture_or_unavailable(name)?;
    Some((std::fs::read(fixture.file()).expect("fixture readable"), fixture.password.clone()))
}

/// A fixture every platform's corpus has.
fn fixture_everywhere(name: &str) -> (Vec<u8>, Option<String>) {
    fixture(name).unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"))
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
        marks: None,
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
    // CoreText English: its paragraphs are set in an embedded font with a ToUnicode map.
    let Some((bytes, _)) = fixture("coretext/en-paragraphs") else { return };
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
    // CoreText English: its paragraphs are set in an embedded font with a ToUnicode map.
    let Some((bytes, _)) = fixture("coretext/en-paragraphs") else { return };
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
    // CoreText English: its paragraphs are set in an embedded font with a ToUnicode map.
    let Some((bytes, _)) = fixture("coretext/en-paragraphs") else { return };
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
    let (bytes, _) = fixture_everywhere("chrome/en-paragraphs");
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
    let (bytes, _) = fixture_everywhere("chrome/en-paragraphs");
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
    let (bytes, _) = fixture_everywhere("chrome/en-paragraphs");
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
    let (bytes, _) = fixture_everywhere("qpdf/rotated");
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

// ------------------------------------------------------------- styled runs

/// A one-page file with Helvetica (F1), Helvetica-Bold (F2) and
/// Helvetica-Oblique (F3), each with metrics.
fn styled_pdf(content: &str) -> Vec<u8> {
    let widths = |w: &str| vec![w; 95].join(" ");
    let font = |name: &str, w: &str| {
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths [{}] >>",
            widths(w)
        )
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 500 400] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 6 0 R /F3 7 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        font("Helvetica", "556"),
        font("Helvetica-Bold", "611"),
        font("Helvetica-Oblique", "556"),
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

/// Two lines: "due" bold, "first" red, "without delay" italic.
const STYLED: &str = "BT /F1 12 Tf 14 TL 50 300 Td (The rent is ) Tj /F2 12 Tf (due) Tj /F1 12 Tf ( on the ) Tj 1 0 0 rg (first) Tj 0 0 0 rg ( day of each) Tj T* (month, ) Tj /F3 12 Tf (without delay) Tj /F1 12 Tf (, by transfer.) Tj ET\n";

/// A block's runs: (text, font, bold, italic, red).
fn runs_of(block: &TextBlock) -> Vec<(String, String, bool, bool, bool)> {
    let units: Vec<u16> = block.text.encode_utf16().collect();
    block
        .runs
        .iter()
        .map(|r| {
            (
                String::from_utf16_lossy(&units[r.start..r.end]),
                r.font.clone(),
                r.bold,
                r.italic,
                r.color[0] > 0.5,
            )
        })
        .collect()
}

/// The run holding `needle`, as (font, bold, italic, red).
fn run_with(block: &TextBlock, needle: &str) -> (String, bool, bool, bool) {
    let runs = runs_of(block);
    runs.iter()
        .find(|(text, ..)| text.contains(needle))
        .map(|(_, font, bold, italic, red)| (font.clone(), *bold, *italic, *red))
        .unwrap_or_else(|| panic!("no run holds {needle:?}: {runs:#?}"))
}

#[test]
fn a_paragraphs_bold_italic_and_coloured_words_are_read_as_runs() {
    let document = engine().open_bytes(styled_pdf(STYLED), None).expect("opens");
    let block = block_with(&document, 0, "The rent is due");
    assert_eq!(block.text, "The rent is due on the first day of each month, without delay, by transfer.");
    assert!(block.styled && block.lines == 2, "{block:?}");
    let plain = |text: &str| (text.to_string(), "Helvetica".to_string(), false, false, false);
    assert_eq!(
        runs_of(&block),
        vec![
            plain("The rent is "),
            ("due ".into(), "Helvetica-Bold".into(), true, false, false),
            plain("on the "),
            ("first ".into(), "Helvetica".into(), false, false, true),
            plain("day of each month, "),
            ("without delay".into(), "Helvetica-Oblique".into(), false, true, false),
            plain(", by transfer."),
        ]
    );
}

#[test]
fn editing_a_styled_paragraph_in_its_own_fonts_keeps_every_run() {
    let document = engine().open_bytes(styled_pdf(STYLED), None).expect("opens");
    let block = block_with(&document, 0, "The rent is due");
    // Typed over the bold word, and after the italic one.
    let new = block.text.replace("due", "payable").replace("delay", "delay or excuse");
    let (edited, outcome, update) = edit_to(&document, &block, 0, &new);
    assert_eq!(outcome.substituted, None, "each run in its own font");
    assert!(!String::from_utf8_lossy(&update).contains("FontFile"), "no font was embedded");
    let after = block_with(&edited, 0, "The rent is payable");
    // Still one paragraph, though a line of it is now mostly italic.
    assert_eq!(normalize(&after.text), normalize(&new));
    assert_eq!(after.lines, 3);
    assert_eq!(run_with(&after, "payable"), ("Helvetica-Bold".into(), true, false, false));
    assert_eq!(run_with(&after, "first"), ("Helvetica".into(), false, false, true));
    assert_eq!(run_with(&after, "or excuse"), ("Helvetica-Oblique".into(), false, true, false));
    assert_eq!(run_with(&after, "transfer"), ("Helvetica".into(), false, false, false));
    assert!(page_text(&edited, 0).contains(&normalize(&new)));
    qpdf_check(&edited.bytes(), "styled-own.pdf");
}

#[test]
fn a_substituted_paragraph_keeps_bold_italic_size_and_colour_and_keeps_them_on_the_next_edit() {
    let document = engine().open_bytes(styled_pdf(STYLED), None).expect("opens");
    let block = block_with(&document, 0, "The rent is due");
    let new = block.text.replace("first", "fifth");
    let edit = BlockEdit {
        page: 0,
        index: block.index,
        fingerprint: block.fingerprint,
        text: new.clone(),
        family: Some(FontFamily::Sans),
        marks: None,
    };
    let (edited, _, outcome) = document.with_block_edit(&edit).expect("edits");
    assert_eq!(outcome.substituted.as_ref().map(|s| s.font.as_str()), Some("IBMPlexSans"));
    let after = block_with(&edited, 0, "The rent is due");
    assert_eq!(normalize(&after.text), normalize(&new));
    assert_eq!(run_with(&after, "due"), ("IBMPlexSans-Bold".into(), true, false, false));
    assert_eq!(run_with(&after, "fifth"), ("IBMPlexSans".into(), false, false, true));
    // No italic face is bundled: the run is slanted, under a name that says so.
    assert_eq!(run_with(&after, "without delay"), ("IBMPlexSans-Oblique".into(), false, true, false));
    assert!(page_text(&edited, 0).contains(&normalize(&new)));

    // Edited again (by name now, the bundled faces themselves): nothing is lost.
    let again_text = after.text.replace("transfer", "bank transfer");
    let (again, ..) = edit_to(&edited, &after, 0, &again_text);
    let third = block_with(&again, 0, "The rent is due");
    assert_eq!(normalize(&third.text), normalize(&again_text));
    assert_eq!(run_with(&third, "due"), ("IBMPlexSans-Bold".into(), true, false, false));
    assert_eq!(run_with(&third, "fifth"), ("IBMPlexSans".into(), false, false, true));
    assert_eq!(run_with(&third, "without delay"), ("IBMPlexSans-Oblique".into(), false, true, false));
    qpdf_check(&again.bytes(), "styled-substitute.pdf");
}

#[test]
fn arabic_typed_over_a_bold_word_is_set_in_a_bold_arabic_face() {
    let document = engine().open_bytes(styled_pdf(STYLED), None).expect("opens");
    let block = block_with(&document, 0, "The rent is due");
    let new = block.text.replace("due", "مستحق");
    let (edited, outcome, _) = edit_to(&document, &block, 0, &new);
    assert!(outcome.substituted.is_some(), "Arabic needs a bundled face");
    let after = block_with(&edited, 0, "مستحق");
    assert_eq!(normalize(&after.text), normalize(&new));
    let (font, bold, ..) = run_with(&after, "مستحق");
    assert!(bold && font.contains("Arabic"), "{font}");
    let (font, bold, ..) = run_with(&after, "The rent is");
    assert!(!bold, "{font}");
    assert!(run_with(&after, "first").3, "still red");
}

/// The editor's marks for a paragraph as it reads: its runs' bold and italic.
fn marks_of(block: &TextBlock) -> Vec<Mark> {
    block.runs.iter().map(|r| Mark { start: r.start, end: r.end, bold: r.bold, italic: r.italic }).collect()
}

/// `marks` with `needle` (in `text`) set to `bold` and `italic`, as the editor's buttons would.
fn styled_as(mut marks: Vec<Mark>, text: &str, needle: &str, bold: bool, italic: bool) -> Vec<Mark> {
    let at = text.find(needle).expect("needle");
    let start = text[..at].encode_utf16().count();
    let end = start + needle.encode_utf16().count();
    let mut out = Vec::new();
    for m in marks.drain(..) {
        if m.end <= start || m.start >= end {
            out.push(m);
            continue;
        }
        if m.start < start {
            out.push(Mark { end: start, ..m });
        }
        if m.end > end {
            out.push(Mark { start: end, ..m });
        }
    }
    out.push(Mark { start, end, bold, italic });
    out
}

fn edit_marked(
    document: &Document,
    block: &TextBlock,
    text: &str,
    marks: Vec<Mark>,
) -> (Document, EditedBlock) {
    let edit = BlockEdit {
        page: 0,
        index: block.index,
        fingerprint: block.fingerprint,
        text: text.into(),
        family: None,
        marks: Some(marks),
    };
    let (edited, _, outcome) = document.with_block_edit(&edit).unwrap_or_else(|e| panic!("{e}"));
    (edited, outcome)
}

#[test]
fn bold_and_italic_set_in_the_editor_use_the_pages_own_bold_and_italic() {
    let document = engine().open_bytes(styled_pdf(STYLED), None).expect("opens");
    let block = block_with(&document, 0, "The rent is due");
    let text = block.text.clone();
    // "first" made bold (and stays red), "due" no longer bold, "rent" italic.
    let marks = styled_as(marks_of(&block), &text, "first", true, false);
    let marks = styled_as(marks, &text, "due", false, false);
    let marks = styled_as(marks, &text, "rent", false, true);
    let (edited, outcome) = edit_marked(&document, &block, &text, marks);
    assert_eq!(outcome.substituted, None, "the page has Helvetica's bold and oblique");
    let after = block_with(&edited, 0, "The rent is due");
    assert_eq!(normalize(&after.text), normalize(&text));
    assert_eq!(run_with(&after, "first"), ("Helvetica-Bold".into(), true, false, true));
    assert_eq!(run_with(&after, "due"), ("Helvetica".into(), false, false, false));
    assert_eq!(run_with(&after, "rent"), ("Helvetica-Oblique".into(), false, true, false));
    assert_eq!(run_with(&after, "without delay"), ("Helvetica-Oblique".into(), false, true, false));
    qpdf_check(&edited.bytes(), "marked-own.pdf");
}

#[test]
fn bold_italic_the_page_lacks_sets_the_paragraph_in_bundled_faces() {
    let document = engine().open_bytes(styled_pdf(STYLED), None).expect("opens");
    let block = block_with(&document, 0, "The rent is due");
    let text = block.text.clone();
    // No Helvetica-BoldOblique on the page.
    let marks = styled_as(marks_of(&block), &text, "month", true, true);
    let (edited, outcome) = edit_marked(&document, &block, &text, marks);
    assert!(outcome.substituted.is_some());
    let after = block_with(&edited, 0, "The rent is due");
    assert_eq!(normalize(&after.text), normalize(&text));
    let (font, bold, italic, _) = run_with(&after, "month");
    assert!(bold && italic && font.ends_with("BoldOblique"), "{font}");
    // The rest keeps its own look in the bundled faces.
    let (_, bold, italic, red) = run_with(&after, "due");
    assert!(bold && !italic && !red);
    let (_, bold, italic, red) = run_with(&after, "first");
    assert!(!bold && !italic && red);
    assert!(run_with(&after, "without delay").2, "still italic");
}

// ------------------------------------------------------------------ tashkeel

/// Chrome's vocalised Arabic: every paragraph reads back as the fixture's
/// ground truth, with no spaces around marks and letters in order.
#[test]
fn vocalised_arabic_reads_as_written_with_no_gaps_around_its_marks() {
    let manifest = manifest();
    let fixture = manifest.fixture_or_unavailable("chrome/ar-vocalized").expect("a Chrome fixture");
    let document = engine().open_file(&fixture.file(), None).expect("opens");
    let blocks = document.text_blocks(0).expect("blocks");
    let read: Vec<String> = blocks.iter().filter(|b| b.editable).map(|b| normalize(&b.text)).collect();
    for expected in &fixture.expected {
        assert!(read.contains(&normalize(&expected.text)), "{:?} not read; read: {read:#?}", expected.text);
    }
}

/// Ink left in a block's box (with a margin) on page 0.
fn ink_in(document: &Document, block: &TextBlock) -> usize {
    let image = render(document, 0);
    let area = (block.x - 1.0, block.y - 1.0, block.x + block.width + 1.0, block.y + block.height + 1.0);
    changes_outside_and_ink_inside(&image, &image, area).1
}

#[test]
fn a_vocalised_paragraph_removed_takes_its_marks_with_it() {
    // Chrome draws each mark with its own operator.
    let manifest = manifest();
    let fixture = manifest.fixture_or_unavailable("chrome/ar-vocalized").expect("a Chrome fixture");
    let document = engine().open_file(&fixture.file(), None).expect("opens");
    let block = block_with(&document, 0, "ذَهَبَ");
    assert!(ink_in(&document, &block) > 200);
    let (edited, ..) = edit_to(&document, &block, 0, "");
    assert_eq!(ink_in(&edited, &block), 0, "letters and marks all gone");

    // So does this app: a mark is a glyph of its own in its own `Tj`.
    let (bytes, _) = fixture_everywhere("chrome/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let vocalised = "بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ";
    let document = add_text(&original, vocalised, FontFamily::Naskh, 300.0, 560.0);
    let block = block_with(&document, 0, "بِسْمِ");
    assert_eq!(normalize(&block.text), vocalised);
    assert!(ink_in(&document, &block) > 200);
    let (edited, ..) = edit_to(&document, &block, 0, "");
    assert_eq!(ink_in(&edited, &block), 0, "its marks went with it");
}

// ----------------------------------------------------------- embedded fonts

/// The `/BaseFont`s of fonts embedded by an update.
fn embedded_names(update: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(update);
    text.match_indices("/BaseFont /")
        .map(|(at, m)| {
            text[at + m.len()..]
                .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

#[test]
fn arabic_is_set_again_in_the_pdfs_own_embedded_font_when_it_has_the_forms() {
    // Chrome embeds IBM Plex Sans Arabic as TrueType subsets: a cmap with the
    // presentation forms it drew, no GSUB.
    let (bytes, _) = fixture_everywhere("chrome/ar-sans");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let heading = block_with(&document, 0, "إشعار للموظفين");
    let before = render(&document, 0);

    // The same text again: its own program, its own glyphs.
    let (same, outcome, update) = edit_to(&document, &heading, 0, &heading.text);
    assert_eq!(outcome.source, FontSource::Embedded);
    assert_eq!(outcome.substituted, None);
    let names = embedded_names(&update);
    assert!(names.iter().any(|n| n.ends_with("+IBMPlexSansArabic-SemiBold")), "{names:?}");
    let after = block_with(&same, 0, "إشعار للموظفين");
    assert_eq!(normalize(&after.text), normalize(&heading.text), "read back as letters, not forms");
    assert!(page_text(&same, 0).contains("إشعار للموظفين"));
    // Drawn as it was: the same ink, where it was.
    let area =
        (heading.x - 2.0, heading.y - 2.0, heading.x + heading.width + 2.0, heading.y + heading.height + 2.0);
    let (outside, inside) = changes_outside_and_ink_inside(&before, &render(&same, 0), area);
    let (_, ink_before) = changes_outside_and_ink_inside(&before, &before, area);
    assert_eq!(outside, 0);
    assert!(inside.abs_diff(ink_before) * 10 < ink_before, "{inside} vs {ink_before} dark pixels");

    // A word taken away: the rest keeps its forms, in its own font.
    let (shorter, outcome, _) = edit_to(&document, &heading, 0, "إشعار");
    assert_eq!(outcome.source, FontSource::Embedded);
    assert_eq!(normalize(&block_with(&shorter, 0, "إشعار").text), "إشعار");
    qpdf_check(&same.bytes(), "embedded-arabic.pdf");
}

#[test]
fn a_form_the_subset_lacks_or_marks_it_cannot_place_fall_back_to_bundled_faces() {
    let (bytes, _) = fixture_everywhere("chrome/ar-sans");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let heading = block_with(&document, 0, "إشعار للموظفين");
    // Beh standing alone: a form the heading never drew.
    let (edited, outcome, _) = edit_to(&document, &heading, 0, "إشعار للموظفين ب");
    assert_eq!(outcome.source, FontSource::Bundled);
    assert!(outcome.substituted.is_some());
    assert_eq!(normalize(&block_with(&edited, 0, "إشعار").text), "إشعار للموظفين ب");
    // A paragraph with tashkeel: the subset has no GPOS to place the marks.
    let marked = block_with(&document, 0, "رجى من جميع");
    let (_, outcome, _) = edit_to(&document, &marked, 0, &marked.text.replace("نهاية", "آخر"));
    assert_eq!(outcome.source, FontSource::Bundled);
}

#[test]
fn latin_in_a_pages_own_codes_still_embeds_nothing() {
    let document = engine().open_bytes(styled_pdf(STYLED), None).expect("opens");
    let block = block_with(&document, 0, "The rent is due");
    let (_, outcome, update) = edit_to(&document, &block, 0, &block.text.replace("first", "second"));
    assert_eq!(outcome.source, FontSource::Page);
    assert!(embedded_names(&update).is_empty());
}

// ------------------------------------------------------------------ refusals

#[test]
fn an_edit_of_text_that_changed_is_refused() {
    let document = engine()
        .open_bytes(pdf("BT /F1 12 Tf 50 300 Td (Questions? Email us) Tj ET\n"), None)
        .expect("opens");
    let block = block_with(&document, 0, "Questions? Email");
    let stale = BlockEdit {
        page: 0,
        index: block.index,
        fingerprint: block.fingerprint ^ 1,
        text: "x".into(),
        family: None,
        marks: None,
    };
    assert!(matches!(document.with_block_edit(&stale), Err(Error::InvalidEdit(_))));
    let missing = BlockEdit { index: 99, ..stale };
    assert!(matches!(document.with_block_edit(&missing), Err(Error::InvalidEdit(_))));
}

#[test]
fn encrypted_files_and_unreadable_fonts_are_not_offered_for_editing() {
    // Encrypted: the content is not read, but where its text is, a press says why.
    let (bytes, password) = fixture_everywhere("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let blocks = document.text_blocks(0).expect("blocks");
    assert!(!blocks.is_empty(), "the page's text is listed");
    assert!(blocks.iter().all(|b| !b.editable && b.reason == Some(Refusal::Encrypted)));
    let edit = BlockEdit { page: 0, index: 0, fingerprint: 0, text: "x".into(), family: None, marks: None };
    assert!(document.with_block_edit(&edit).is_err());

    // Chrome draws these fixtures' variable fonts as Type 3, which is not
    // read: refused as one area per run of text, not one per glyph.
    let (bytes, _) = fixture_everywhere("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let blocks = document.text_blocks(0).expect("blocks");
    let fonts: Vec<&TextBlock> = blocks.iter().filter(|b| b.reason == Some(Refusal::Font)).collect();
    assert!(!fonts.is_empty() && fonts.len() < 40, "{} areas", fonts.len());
    let text = document.page_text(0).expect("text");
    let view = document.view_transform(0).expect("view");
    // Every line of the page's text lies in some refused area.
    for line in text.blocks.iter().flat_map(|b| &b.lines) {
        let shown = view.rect(line.bounds);
        let (x, y) = (shown.x + shown.width / 2.0, shown.y + shown.height / 2.0);
        assert!(
            fonts.iter().any(|b| x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height),
            "{:?} is in no refused area",
            line.text
        );
    }

    // CoreText's Arabic draws dotless letters with the dots as separate shapes
    // that carry no text: not read, so not offered, and said to need OCR.
    let Some((bytes, _)) = fixture("coretext/ar-plain") else { return };
    let document = engine().open_bytes(bytes, None).expect("opens");
    let blocks = document.text_blocks(0).expect("blocks");
    assert!(
        !blocks.is_empty() && blocks.iter().all(|b| !b.editable),
        "{:?}",
        blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    );
    assert!(blocks.iter().any(|b| b.reason == Some(Refusal::Unreadable)));
}
