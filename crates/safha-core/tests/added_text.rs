//! Text written into page content (ADR 0021), on real files: PDFium renders
//! each result (the text lands in its box, upright on turned pages, and
//! nothing else moves), our extractor reads it back in logical order as page
//! text, and `qpdf --check` passes. Also: pages that leave their graphics
//! state changed, resources inherited from the page tree, encrypted files,
//! adding twice, undo, and saving.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use safha_core::edit::EditHistory;
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
    let dir = std::env::temp_dir().join(format!("safha-added-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(bytes: &[u8], password: Option<&str>, name: &str) -> String {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write");
    let mut command = Command::new("qpdf");
    if let Some(password) = password {
        command.arg(format!("--password={password}"));
    }
    let output = command.arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    path.to_string_lossy().into_owned()
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

/// Dark pixels inside the view rectangle (x0, y0, x1, y1), and changed pixels outside it.
fn ink_inside_and_changes_outside(
    before: &Bitmap,
    after: &Bitmap,
    (x0, y0, x1, y1): (f32, f32, f32, f32),
) -> (usize, usize) {
    let (mut inside, mut outside) = (0, 0);
    for y in 0..after.height() {
        for x in 0..after.width() {
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            let within = vx >= x0 - 1.0 && vx <= x1 + 1.0 && vy >= y0 - 1.0 && vy <= y1 + 1.0;
            let pixel = after.pixel(x, y).expect("pixel");
            if within {
                if pixel[0] < 100 && pixel[1] < 100 && pixel[2] < 100 {
                    inside += 1;
                }
            } else if before.pixel(x, y) != Some(pixel) {
                outside += 1;
            }
        }
    }
    (inside, outside)
}

fn new_text(page: usize, (x, y, width): (f32, f32, f32), text: &str) -> NewText {
    NewText {
        page,
        x,
        y,
        width,
        text: text.into(),
        family: FontFamily::Sans,
        size: NewText::DEFAULT_SIZE,
        color: [0.0, 0.0, 0.0],
        align: Align::Start,
        direction: BaseDirection::Auto,
    }
}

/// Adds `text`; the edited document, the update, and the text's displayed box.
fn add(document: &Document, text: &NewText) -> (Document, Vec<u8>, (f32, f32, f32, f32)) {
    let (edited, update, height) = document.with_new_text(text).unwrap_or_else(|e| panic!("{e}"));
    (edited, update, (text.x, text.y, text.x + text.width, text.y + height))
}

#[test]
fn english_arabic_and_mixed_text_becomes_page_text_in_logical_order() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let page_text = normalize(&original.page_text(0).expect("text").text());
    let texts = [
        ("Approved by the board on 12 March 2026.", FontFamily::Sans),
        ("تمت الموافقة على العقد بتاريخ ١٢ مارس ٢٠٢٦", FontFamily::Naskh),
        ("Signed: محمد العلي, Director of Finance (ID 4471)", FontFamily::Sans),
    ];
    let mut document = original;
    for (i, (text, family)) in texts.iter().enumerate() {
        let before = render(&document, 0);
        let mut new = new_text(0, (60.0, 560.0 + 60.0 * i as f32, 300.0), text);
        new.family = *family;
        let (edited, _, area) = add(&document, &new);
        document = edited;
        let (inside, outside) = ink_inside_and_changes_outside(&before, &render(&document, 0), area);
        assert!(inside > 200, "{text}: {inside} dark pixels in the box");
        assert_eq!(outside, 0, "{text}: drawn outside its box");
    }
    // Page text now: the page's own, then each addition as typed, no flattening needed.
    let read = normalize(&document.page_text(0).expect("text").text());
    assert!(read.starts_with(&page_text), "the page's own text comes first:\n{read}");
    let added = read[page_text.len()..].trim();
    assert_eq!(added, normalize(&texts.map(|(text, _)| text).join(" ")), "the added text, in logical order");
    qpdf_check(&document.bytes(), None, "three.pdf");
}

#[test]
fn right_to_left_text_wraps_and_aligns_inside_its_width() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let text = "هذا نص طويل يلتف داخل المربع على عدة أسطر دون أن يتجاوز عرضه المحدد. ".repeat(3);
    let before = render(&document, 0);
    let mut new = new_text(0, (100.0, 560.0, 220.0), &text);
    new.align = Align::Justify;
    let (edited, _, area) = add(&document, &new);
    assert!(area.3 - area.1 > 50.0, "several lines: {area:?}");
    let (inside, outside) = ink_inside_and_changes_outside(&before, &render(&edited, 0), area);
    assert!(inside > 500 && outside == 0, "{inside} inside, {outside} outside");
    let read = normalize(&edited.page_text(0).expect("text").text());
    assert!(read.contains(&normalize(&text)), "{read}");
}

/// Which pixels of the view rectangle changed, row by row.
fn changed_mask(before: &Bitmap, after: &Bitmap, (x0, y0, x1, y1): (f32, f32, f32, f32)) -> Vec<bool> {
    let (left, top) = ((x0 * SCALE) as u32, (y0 * SCALE) as u32);
    let (width, height) = (((x1 - x0) * SCALE) as u32, ((y1 - y0) * SCALE) as u32);
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (left + x, top + y)))
        .map(|(x, y)| before.pixel(x, y) != after.pixel(x, y))
        .collect()
}

#[test]
fn on_turned_pages_the_text_stands_upright_where_it_was_placed() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    let text = "Received 3 March — استلم";
    let mut masks = Vec::new();
    for page in 0..4 {
        let before = render(&document, page);
        let (edited, _, area) = add(&document, &new_text(page, (4.0, 4.0, 180.0), text));
        document = edited;
        let after = render(&document, page);
        let (inside, outside) = ink_inside_and_changes_outside(&before, &after, area);
        assert!(inside > 100 && outside == 0, "page {page}: {inside} inside, {outside} outside");
        masks.push(changed_mask(&before, &after, area));
    }
    let ink = masks[0].iter().filter(|changed| **changed).count();
    for (page, mask) in masks.iter().enumerate().skip(1) {
        let differ = masks[0].iter().zip(mask).filter(|(a, b)| a != b).count();
        assert!(differ * 10 < ink, "page {page}: {differ} of {ink} pixels differ from the unturned page");
    }
}

/// A one-page file whose font comes from the page tree's `/Resources`, and
/// whose content leaves the graphics state scaled by 2 (no `q … Q`).
fn unbalanced_pdf() -> Vec<u8> {
    let content = "2 0 0 2 0 0 cm BT /F1 12 Tf 20 150 Td (Scaled page text) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 400 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
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
fn a_page_that_leaves_its_state_changed_cannot_move_new_text() {
    let document = engine().open_bytes(unbalanced_pdf(), None).expect("opens");
    let before = render(&document, 0);
    let (edited, _, area) = add(&document, &new_text(0, (20.0, 20.0, 200.0), "Added — مضاف"));
    let after = render(&edited, 0);
    // Exactly where it was put, at its own size; the page's text unmoved.
    let (inside, outside) = ink_inside_and_changes_outside(&before, &after, area);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");
    let read = normalize(&edited.page_text(0).expect("text").text());
    assert!(read.contains("Scaled page text") && read.contains("Added — مضاف"), "{read}");

    // A second addition reuses the wrapping, and both read back.
    let (twice, update, _) = add(&edited, &new_text(0, (20.0, 300.0, 200.0), "Second"));
    assert_eq!(String::from_utf8_lossy(&update).matches("MirsamWrap").count(), 0, "wrapped once only");
    let read = normalize(&twice.page_text(0).expect("text").text());
    assert!(read.contains("Added — مضاف") && read.contains("Second"), "{read}");
    // The inherited font still draws the page's own text: the page has its own resources now.
    qpdf_check(&twice.bytes(), None, "unbalanced.pdf");
}

#[test]
fn encrypted_files_get_encrypted_text() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let before = render(&document, 0);
    // Arabic-majority: a single line's paragraph direction is the reader's guess (ADR 0019).
    let text = "سري للغاية ولا ينشر خارج الإدارة — Confidential ٢٠٢٦";
    let (edited, _, area) = add(&document, &new_text(0, (40.0, 700.0, 300.0), text));
    let (inside, outside) = ink_inside_and_changes_outside(&before, &render(&edited, 0), area);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");
    let read = normalize(&edited.page_text(0).expect("text").text());
    assert!(read.contains(text), "{read}");
    qpdf_check(&edited.bytes(), password.as_deref(), "encrypted.pdf");
}

#[test]
fn undo_takes_the_text_away_and_saving_keeps_it() {
    let (bytes, _) = fixture("chrome/mixed-inline");
    let original = engine().open_bytes(bytes.clone(), None).expect("opens");
    let (edited, update, _) = add(&original, &new_text(0, (60.0, 600.0, 300.0), "Keep me — احفظني"));
    let mut history = EditHistory::new(Arc::from(bytes.clone()));
    history.push_content(update);
    assert!(history.can_undo());
    let undone = history.undo().expect("undoes");
    assert_eq!(&*undone, bytes.as_slice());

    let target = scratch("saved.pdf");
    save(&edited, &target).expect("saves");
    let reopened = engine().open_file(&target, None).expect("reopens");
    let read = normalize(&reopened.page_text(0).expect("text").text());
    assert!(read.contains("Keep me — احفظني"), "{read}");
}

#[test]
fn a_page_out_of_range_is_refused() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    assert!(matches!(
        document.with_new_text(&new_text(5, (10.0, 10.0, 100.0), "x")),
        Err(Error::PageOutOfRange { .. })
    ));
}
