//! Text boxes (ADR 0019) on real files. Each is rendered by PDFium, and then
//! flattened into its page by `qpdf --flatten-annotations`, where this
//! crate's own extractor must read back exactly the text that was typed, in
//! logical order: that checks the shaping, the embedded font subsets, and
//! their ToUnicode maps together, in English, Arabic, and both mixed.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use safha_core::annotate::{AnnotationEdit, AnnotationKind, TextBox};
use safha_core::edit::EditHistory;
use safha_core::save::save;
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
    let dir = std::env::temp_dir().join(format!("safha-text-boxes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf(args: &[&str]) {
    let output = Command::new("qpdf").args(args).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The document with its annotations merged into its pages' content.
fn flattened(document: &Document, password: Option<&str>, name: &str) -> Document {
    let input = scratch(&format!("{name}-in.pdf"));
    let output = scratch(&format!("{name}-flat.pdf"));
    std::fs::write(&input, document.bytes()).expect("write");
    let password = format!("--password={}", password.unwrap_or(""));
    let (input, output) = (input.to_string_lossy().into_owned(), output.to_string_lossy().into_owned());
    qpdf(&[&password, "--check", &input]);
    qpdf(&[&password, "--decrypt", "--flatten-annotations=all", &input, &output]);
    engine().open_file(std::path::Path::new(&output), None).expect("the flattened file opens")
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

/// Dark pixels inside the view rectangle, and changed pixels outside it.
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

/// A text box at view (`x`, `y`), `width` wide, its height fitted to `text`;
/// and its displayed rectangle.
fn text_box(
    document: &Document,
    page: usize,
    (x, y, width): (f32, f32, f32),
    text: &str,
) -> (AnnotationEdit, (f32, f32, f32, f32)) {
    let view = document.view_transform(page).expect("view");
    let height = TextBox::height_for(text, width, TextBox::DEFAULT_SIZE).expect("fits");
    let rect = TextBox::rect_at(x, y, width, height, &view);
    let edit = AnnotationEdit::AddTextBox {
        page,
        text_box: TextBox {
            rect,
            contents: text.into(),
            size: TextBox::DEFAULT_SIZE,
            color: TextBox::DEFAULT_COLOR,
        },
    };
    (edit, (x, y, x + width, y + height))
}

fn apply(document: &Document, edit: &AnnotationEdit) -> (Document, Vec<u8>) {
    document.with_annotation_edit(edit).expect("the edit applies")
}

#[test]
fn english_arabic_and_mixed_text_is_drawn_and_reads_back_in_logical_order() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let original = engine().open_bytes(bytes, None).expect("opens");
    let page_text = normalize(&original.page_text(0).expect("text").text());
    let texts = [
        "Approved by the board on 12 March 2026.",
        "تمت الموافقة على العقد بتاريخ ١٢ مارس ٢٠٢٦",
        "Signed: محمد العلي, Director of Finance (ID 4471)",
    ];
    let mut document = original;
    for (i, text) in texts.iter().enumerate() {
        let before = render(&document, 0);
        let (edit, area) = text_box(&document, 0, (60.0, 500.0 + 60.0 * i as f32, 260.0), text);
        document = apply(&document, &edit).0;
        let (inside, outside) = ink_inside_and_changes_outside(&before, &render(&document, 0), area);
        assert!(inside > 200, "{text}: {inside} dark pixels in the box");
        assert_eq!(outside, 0, "{text}: drawn outside its box");
    }
    let (annotations, _) = document.page_annotations(0).expect("listed");
    assert_eq!(annotations.iter().map(|a| a.kind).collect::<Vec<_>>(), [AnnotationKind::TextBox; 3]);
    assert_eq!(annotations[1].contents.as_deref(), Some(texts[1]));

    // Merged into the page: the page's own text, then each box's, as typed.
    let flat = flattened(&document, None, "three");
    let read = normalize(&flat.page_text(0).expect("text").text());
    assert!(read.starts_with(&page_text), "the page's own text comes first:\n{read}");
    let boxes = read[page_text.len()..].trim();
    assert_eq!(boxes, normalize(&texts.join(" ")), "the boxes' text, in logical order");
}

#[test]
fn a_long_text_wraps_inside_its_width() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let text = "هذا نص طويل يلتف داخل المربع على عدة أسطر دون أن يتجاوز عرضه المحدد. ".repeat(3);
    let before = render(&document, 0);
    let (edit, area) = text_box(&document, 0, (100.0, 560.0, 200.0), &text);
    let (edited, _) = apply(&document, &edit);
    assert!(area.3 - area.1 > 60.0, "several lines: {area:?}");
    let (inside, outside) = ink_inside_and_changes_outside(&before, &render(&edited, 0), area);
    assert!(inside > 500 && outside == 0, "{inside} inside, {outside} outside");
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
    // Pages turned 0, 90, 180 and 270 degrees; the box in the top margin as displayed.
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    let text = "Received 3 March — استلم";
    let mut masks = Vec::new();
    for page in 0..4 {
        let before = render(&document, page);
        let (edit, area) = text_box(&document, page, (4.0, 4.0, 180.0), text);
        document = apply(&document, &edit).0;
        let after = render(&document, page);
        let (inside, outside) = ink_inside_and_changes_outside(&before, &after, area);
        assert!(inside > 100 && outside == 0, "page {page}: {inside} inside, {outside} outside");
        let (annotations, view) = document.page_annotations(page).expect("listed");
        let shown = view.rect(annotations[0].rects[0]);
        assert!((shown.x - area.0).abs() < 0.01 && (shown.y - area.1).abs() < 0.01, "page {page}: {shown:?}");
        masks.push(changed_mask(&before, &after, area));
    }
    // Upright as displayed: each turned page shows the box as the unturned one does.
    let ink = masks[0].iter().filter(|changed| **changed).count();
    for (page, mask) in masks.iter().enumerate().skip(1) {
        assert_eq!(mask.len(), masks[0].len());
        let differ = masks[0].iter().zip(mask).filter(|(a, b)| a != b).count();
        assert!(differ * 10 < ink, "page {page}: {differ} of {ink} pixels differ from the unturned page");
    }
    // The text extractor reads page space, where only the unturned page's box runs across.
    let flat = flattened(&document, None, "turned");
    let read = normalize(&flat.page_text(0).expect("text").text());
    assert!(read.contains(&normalize(text)), "{read}");
}

#[test]
fn changing_the_text_sets_it_again_and_grows_the_box_downward() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes.clone(), None).expect("opens");
    let mut history = EditHistory::new(Arc::from(bytes));
    let (edit, area) = text_box(&document, 0, (60.0, 520.0, 200.0), "Short.");
    let (added, update) = apply(&document, &edit);
    history.push_annotations(update);
    let longer = "Now a much longer note that has to wrap onto several lines inside the same width.";
    let (changed, update) =
        apply(&added, &AnnotationEdit::SetContents { page: 0, index: 0, contents: longer.into() });
    history.push_annotations(update);

    let (annotations, view) = changed.page_annotations(0).expect("listed");
    assert_eq!(annotations.len(), 1, "the same annotation, rewritten");
    assert_eq!(annotations[0].contents.as_deref(), Some(longer));
    let shown = view.rect(annotations[0].rects[0]);
    assert!((shown.y - area.1).abs() < 0.01, "the top stays: {} vs {}", shown.y, area.1);
    assert!((shown.width - 200.0).abs() < 0.01, "the width stays: {}", shown.width);
    assert!(shown.height > (area.3 - area.1) * 2.0, "taller: {}", shown.height);
    let flat = flattened(&changed, None, "changed");
    assert!(normalize(&flat.page_text(0).expect("text").text()).ends_with(&normalize(longer)));

    let undone = document.revision(history.undo().expect("undo")).expect("opens");
    assert_eq!(undone.page_annotations(0).expect("listed").0[0].contents.as_deref(), Some("Short."));
}

#[test]
fn text_boxes_in_encrypted_files() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    // Mostly Arabic, so a reader takes it for a right-to-left paragraph: PDF
    // keeps no paragraph direction, and a line with more Latin than Arabic
    // letters reads as left to right.
    let text = "سري للغاية ولا يجوز نسخه أو نشره — Confidential";
    let (edit, _) = text_box(&document, 0, (60.0, 600.0, 240.0), text);
    let (added, _) = apply(&document, &edit);
    // Its `/DA` is decrypted to set it again; the new text and fonts are encrypted.
    let (changed, _) =
        apply(&added, &AnnotationEdit::SetContents { page: 0, index: 0, contents: format!("{text} ٢") });
    assert_eq!(changed.page_annotations(0).expect("listed").0[0].contents, Some(format!("{text} ٢")));
    let flat = flattened(&changed, password.as_deref(), "encrypted");
    let read = normalize(&flat.page_text(0).expect("text").text());
    assert!(read.ends_with(&normalize(&format!("{text} ٢"))), "{read}");
}

#[test]
fn saving_keeps_text_boxes_and_their_fonts() {
    let (bytes, _) = fixture("chrome/mixed-inline");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let text = "Checked — تمت المراجعة";
    let (edit, area) = text_box(&document, 0, (60.0, 600.0, 220.0), text);
    let (edited, _) = apply(&document, &edit);
    let target = scratch("saved.pdf");
    save(&edited, &target).expect("saves");
    let reopened = engine().open_file(&target, None).expect("reopens");
    assert_eq!(reopened.page_annotations(0).expect("listed").0[0].contents.as_deref(), Some(text));
    let (inside, _) = ink_inside_and_changes_outside(&render(&document, 0), &render(&reopened, 0), area);
    assert!(inside > 100, "{inside}");
}

#[test]
fn empty_text_is_refused() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    assert!(matches!(TextBox::height_for("   ", 200.0, 12.0), Err(Error::InvalidEdit(_))));
    let (edit, _) = text_box(&document, 0, (60.0, 600.0, 200.0), "x");
    let (added, _) = apply(&document, &edit);
    assert!(matches!(
        added.with_annotation_edit(&AnnotationEdit::SetContents { page: 0, index: 0, contents: " ".into() }),
        Err(Error::InvalidEdit(_))
    ));
}
