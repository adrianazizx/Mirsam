//! Page objects (ADR 0051) on real files: images, paths and text blocks
//! traced to their bytes, then moved, resized, rotated, cropped, removed,
//! reordered and replaced by byte-range edits, rendered by PDFium, read back
//! by our extractor, checked with `qpdf --check`, undone and saved. English
//! and Arabic documents, turned pages, an encrypted file.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use safha_core::edit::EditHistory;
use safha_core::objects::{Arrange, NewImage, ObjectChange, ObjectEdit, ObjectKind, ObjectRef, PageObject};
use safha_core::save::save;
use safha_core::view::ViewRect;
use safha_core::{Bitmap, Document, Engine, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn fixture(name: &str) -> (Vec<u8>, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixture_or_unavailable(name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (std::fs::read(fixture.file()).expect("fixture readable"), fixture.password.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-objects-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(bytes: &[u8], password: Option<&str>, name: &str) {
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
}

const SCALE: f32 = 1.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

fn pixel(bitmap: &Bitmap, x: f32, y: f32) -> [u8; 4] {
    bitmap.pixel((x * SCALE) as u32, (y * SCALE) as u32).expect("pixel")
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

fn is_blue(p: [u8; 4]) -> bool {
    p[2] > 200 && p[0] < 60 && p[1] < 60
}

fn is_white(p: [u8; 4]) -> bool {
    p[0] > 240 && p[1] > 240 && p[2] > 240
}

fn is_green(p: [u8; 4]) -> bool {
    p[1] > 120 && p[0] < 60 && p[2] < 60
}

/// Pixels that differ between two renders outside the displayed rectangles.
fn changes_outside(before: &Bitmap, after: &Bitmap, areas: &[(f32, f32, f32, f32)]) -> usize {
    let mut count = 0;
    for y in 0..after.height() {
        for x in 0..after.width() {
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            let within = areas.iter().any(|&(x0, y0, x1, y1)| {
                vx >= x0 - 1.0 && vx <= x1 + 1.0 && vy >= y0 - 1.0 && vy <= y1 + 1.0
            });
            if !within && before.pixel(x, y) != after.pixel(x, y) {
                count += 1;
            }
        }
    }
    count
}

fn area_of(rect: &ViewRect) -> (f32, f32, f32, f32) {
    (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height)
}

/// A one-page PDF (400 × 400) whose page content is `content`, with the
/// font `F1` (Helvetica) and the image `Im1` (2 × 2 green pixels).
fn pdf(content: &str) -> Vec<u8> {
    let image = [0u8, 180, 0, 0, 180, 0, 0, 180, 0, 0, 180, 0];
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 400 400] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >> >> >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    let mut image_object = format!(
        "<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
        image.len()
    )
    .into_bytes();
    image_object.extend_from_slice(&image);
    image_object.extend_from_slice(b"\nendstream");
    objects.push(image_object);
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}

/// A red square, a green image, two lines of text, and a blue square over the red one's corner.
const SCENE: &str = "q 1 0 0 rg 40 240 100 100 re f Q\n\
q 100 0 0 100 220 220 cm /Im1 Do Q\n\
BT /F1 24 Tf 40 120 Td (Hello ) Tj (there) Tj ET\n\
BT /F1 24 Tf 40 40 Td (Second line) Tj ET\n\
q 0 0 1 rg 100 200 80 80 re f Q";

fn open(bytes: Vec<u8>) -> Document {
    engine().open_bytes(bytes, None).expect("opens")
}

fn objects(document: &Document, page: usize) -> Vec<PageObject> {
    document.page_objects(page).expect("objects")
}

fn reference(object: &PageObject) -> ObjectRef {
    ObjectRef { index: object.index, fingerprint: object.fingerprint }
}

fn edit(
    document: &Document,
    page: usize,
    object: &PageObject,
    edit: ObjectEdit,
) -> (Document, Vec<u8>, Option<usize>) {
    document.with_object_edit(&ObjectChange { page, object: reference(object), edit }).expect("edits")
}

fn translate(dx: f64, dy: f64) -> ObjectEdit {
    ObjectEdit::Transform { matrix: [1.0, 0.0, 0.0, 1.0, dx, dy] }
}

fn text(document: &Document, page: usize) -> String {
    document.page_text(page).expect("text").text().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn a_page_is_traced_to_its_objects() {
    let document = open(pdf(SCENE));
    let found = objects(&document, 0);
    let kinds: Vec<ObjectKind> = found.iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        [ObjectKind::Path, ObjectKind::Image, ObjectKind::Text, ObjectKind::Text, ObjectKind::Path],
        "{found:#?}"
    );
    // The red square: page (40, 240)–(140, 340) is displayed at (40, 60)–(140, 160).
    let red = &found[0];
    assert!((red.rect.x - 40.0).abs() < 0.01 && (red.rect.y - 60.0).abs() < 0.01, "{:?}", red.rect);
    assert!((red.rect.width - 100.0).abs() < 0.01 && (red.rect.height - 100.0).abs() < 0.01);
    let image = &found[1];
    assert_eq!(image.image.map(|i| (i.width, i.height)), Some((2, 2)));
    // "Hello there" is one block of two shows; it sits on its baseline at y = 280 displayed.
    let hello = &found[2];
    assert!(hello.rect.y < 280.0 && hello.rect.y + hello.rect.height > 280.0, "{:?}", hello.rect);
    assert!(hello.rect.width > 100.0, "both shows in one block: {:?}", hello.rect);
}

#[test]
fn moving_a_path_changes_only_where_it_was_and_where_it_went() {
    let document = open(pdf(SCENE));
    let before = render(&document, 0);
    let red = objects(&document, 0)[0].clone();
    let (moved, update, index) = edit(&document, 0, &red, translate(0.0, 200.0));
    assert_eq!(index, Some(0), "the square keeps its place in the drawing order");
    let after = render(&moved, 0);
    // Gone from its old place (where nothing else is), drawn at the new one, under the text.
    assert!(is_white(pixel(&after, 50.0, 70.0)), "{:?}", pixel(&after, 50.0, 70.0));
    assert!(is_red(pixel(&after, 50.0, 300.0)));
    let old = area_of(&red.rect);
    let new = (old.0, old.1 + 200.0, old.2, old.3 + 200.0);
    assert_eq!(changes_outside(&before, &after, &[old, new]), 0);
    // The blue square still covers the red one's corner where they overlap… at its old place only.
    let objects_after = objects(&moved, 0);
    assert_eq!(objects_after.len(), 5);
    assert!((objects_after[0].rect.y - 260.0).abs() < 0.01);
    assert!(!update.is_empty());
    qpdf_check(&moved.bytes(), None, "moved-path.pdf");
}

#[test]
fn moving_again_changes_the_wrapper_instead_of_adding_one() {
    let document = open(pdf(SCENE));
    let red = objects(&document, 0)[0].clone();
    let (once, _, _) = edit(&document, 0, &red, translate(10.0, 0.0));
    let red = objects(&once, 0)[0].clone();
    let (twice, _, _) = edit(&once, 0, &red, translate(10.0, 0.0));
    let red = &objects(&twice, 0)[0];
    assert!((red.rect.x - 60.0).abs() < 0.01, "{:?}", red.rect);
    let after = render(&twice, 0);
    assert!(is_red(pixel(&after, 65.0, 70.0)) && !is_red(pixel(&after, 50.0, 70.0)));
}

#[test]
fn an_image_is_resized_rotated_cropped_and_replaced() {
    let document = open(pdf(SCENE));
    let image = objects(&document, 0)[1].clone();
    // Displayed at (220, 80)–(320, 180). Half the size about its top-left corner.
    let (x, y) = (f64::from(image.rect.x), f64::from(image.rect.y));
    let half = ObjectEdit::Transform { matrix: [0.5, 0.0, 0.0, 0.5, x * 0.5, y * 0.5] };
    let (smaller, _, _) = edit(&document, 0, &image, half);
    let image = objects(&smaller, 0)[1].clone();
    assert!(
        (image.rect.width - 50.0).abs() < 0.01 && (image.rect.x - 220.0).abs() < 0.01,
        "{:?}",
        image.rect
    );
    let rendered = render(&smaller, 0);
    assert!(is_green(pixel(&rendered, 240.0, 100.0)) && is_white(pixel(&rendered, 300.0, 160.0)));

    // A quarter turn about its centre keeps a square where it was.
    let (cx, cy) =
        (f64::from(image.rect.x + image.rect.width / 2.0), f64::from(image.rect.y + image.rect.height / 2.0));
    let turn = ObjectEdit::Transform { matrix: [0.0, 1.0, -1.0, 0.0, cx + cy, cy - cx] };
    let (turned, _, _) = edit(&smaller, 0, &image, turn);
    let image = objects(&turned, 0)[1].clone();
    assert!(
        (image.rect.x - 220.0).abs() < 0.01 && (image.rect.width - 50.0).abs() < 0.01,
        "{:?}",
        image.rect
    );
    assert!(
        (image.rect.y - 80.0).abs() < 0.01 && (image.rect.height - 50.0).abs() < 0.01,
        "{:?}",
        image.rect
    );

    // Cropped to its left half: the right half is gone; the crop moves with it.
    let crop = ViewRect { x: image.rect.x, y: image.rect.y, width: 25.0, height: 50.0 };
    let (cropped, _, _) = edit(&turned, 0, &image, ObjectEdit::Crop { rect: Some(crop) });
    let image = objects(&cropped, 0)[1].clone();
    assert!(image.cropped && (image.rect.width - 25.0).abs() < 0.01, "{:?}", image.rect);
    let rendered = render(&cropped, 0);
    assert!(is_green(pixel(&rendered, 230.0, 100.0)) && is_white(pixel(&rendered, 260.0, 100.0)));
    let (moved, _, _) = edit(&cropped, 0, &image, translate(0.0, 100.0));
    let rendered = render(&moved, 0);
    assert!(is_green(pixel(&rendered, 230.0, 200.0)) && is_white(pixel(&rendered, 260.0, 200.0)));
    let image = objects(&moved, 0)[1].clone();
    let (uncropped, _, _) = edit(&moved, 0, &image, ObjectEdit::Crop { rect: None });
    let image = objects(&uncropped, 0)[1].clone();
    assert!(!image.cropped && (image.rect.width - 50.0).abs() < 0.01, "{:?}", image.rect);

    // Replaced by a wide red PNG: fitted in the box, in proportion, centred,
    // and turned as the old one was.
    let png = png_of(40, 20, [220, 0, 0]);
    let (replaced, _, index) = edit(&uncropped, 0, &image, ObjectEdit::ReplaceImage { data: png });
    let image = &objects(&replaced, 0)[index.expect("still there")];
    assert_eq!(image.image.map(|i| (i.width, i.height)), Some((40, 20)));
    assert!(
        (image.rect.width - 25.0).abs() < 0.1 && (image.rect.height - 50.0).abs() < 0.1,
        "{:?}",
        image.rect
    );
    assert!((image.rect.x - 232.5).abs() < 0.1, "centred: {:?}", image.rect);
    let rendered = render(&replaced, 0);
    let (mx, my) = (image.rect.x + 12.5, image.rect.y + 25.0);
    assert!(is_red(pixel(&rendered, mx, my)), "{:?}", pixel(&rendered, mx, my));
    qpdf_check(&replaced.bytes(), None, "image.pdf");
}

fn png_of(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("header");
        let data: Vec<u8> = (0..width * height).flat_map(|_| rgb).collect();
        writer.write_image_data(&data).expect("data");
    }
    out
}

#[test]
fn text_moves_and_goes_and_the_text_after_it_stays() {
    let document = open(pdf(SCENE));
    let before = render(&document, 0);
    let found = objects(&document, 0);
    let hello = found[2].clone();
    let second = found[3].clone();
    let (moved, _, _) = edit(&document, 0, &hello, translate(100.0, -40.0));
    let after = render(&moved, 0);
    let old = area_of(&hello.rect);
    let new = (old.0 + 100.0, old.1 - 40.0, old.2 + 100.0, old.3 - 40.0);
    assert_eq!(changes_outside(&before, &after, &[old, new]), 0, "nothing else moved");
    assert!(text(&moved, 0).contains("Hello there"), "{}", text(&moved, 0));
    let moved_hello = &objects(&moved, 0)[2];
    assert!((moved_hello.rect.x - hello.rect.x - 100.0).abs() < 0.5, "{:?}", moved_hello.rect);
    assert!((objects(&moved, 0)[3].rect.y - second.rect.y).abs() < 0.01);

    // Removing it leaves the second line where it was.
    let hello = objects(&moved, 0)[2].clone();
    let (gone, _, index) = edit(&moved, 0, &hello, ObjectEdit::Delete);
    assert_eq!(index, None);
    let read = text(&gone, 0);
    assert!(!read.contains("Hello") && read.contains("Second line"), "{read}");
    assert_eq!(changes_outside(&before, &render(&gone, 0), &[old, new]), 0);
    qpdf_check(&gone.bytes(), None, "text.pdf");
}

#[test]
fn one_show_of_a_text_object_moves_and_the_next_keeps_its_place() {
    // Three shows in one text object, far apart: three blocks.
    let content = "BT /F1 20 Tf 40 300 Td (Left) Tj 200 0 Td (Right) Tj 0 -40 Td (Below) Tj ET";
    let document = open(pdf(content));
    let found = objects(&document, 0);
    assert_eq!(found.len(), 3, "{found:#?}");
    let before = render(&document, 0);
    let left = found[0].clone();
    let (moved, _, _) = edit(&document, 0, &left, translate(0.0, 200.0));
    let after = render(&moved, 0);
    let old = area_of(&left.rect);
    let new = (old.0, old.1 + 200.0, old.2, old.3 + 200.0);
    assert_eq!(changes_outside(&before, &after, &[old, new]), 0, "Right and Below stay put");
    let read = text(&moved, 0);
    assert!(read.contains("Left") && read.contains("Right") && read.contains("Below"), "{read}");
}

#[test]
fn reordering_puts_an_object_over_or_under_another() {
    let document = open(pdf(SCENE));
    // Where the blue square covers the red one.
    let (x, y) = (120.0, 140.0);
    assert!(is_blue(pixel(&render(&document, 0), x, y)));
    let red = objects(&document, 0)[0].clone();
    let (front, _, index) = edit(&document, 0, &red, ObjectEdit::Arrange(Arrange::Front));
    assert_eq!(index, Some(4));
    assert!(is_red(pixel(&render(&front, 0), x, y)));
    let before = render(&document, 0);
    // Nothing else changed, the overlap aside.
    assert_eq!(changes_outside(&before, &render(&front, 0), &[(100.0, 120.0, 140.0, 160.0)]), 0);

    // Back under: one step.
    let red = objects(&front, 0)[4].clone();
    let (under, _, index) = edit(&front, 0, &red, ObjectEdit::Arrange(Arrange::Backward));
    assert!(is_blue(pixel(&render(&under, 0), x, y)));
    assert_eq!(index, Some(3), "just under the blue square");

    // The blue one to the back, then forward again.
    let blue = objects(&document, 0)[4].clone();
    let (back, _, index) = edit(&document, 0, &blue, ObjectEdit::Arrange(Arrange::Back));
    assert_eq!(index, Some(0));
    assert!(is_red(pixel(&render(&back, 0), x, y)));
    let blue = objects(&back, 0)[0].clone();
    let (forward, _, _) = edit(&back, 0, &blue, ObjectEdit::Arrange(Arrange::Forward));
    assert!(is_blue(pixel(&render(&forward, 0), x, y)));
    qpdf_check(&forward.bytes(), None, "arranged.pdf");
}

#[test]
fn reordering_keeps_the_state_an_object_was_drawn_in() {
    // A dashed, thick, coloured stroke under a scaled CTM, clipped; then a cover.
    let content = "q 2 0 0 2 0 0 cm 10 10 150 150 re W n 0 0 1 RG 6 w [4 2] 0 d 20 20 m 120 20 l S Q\n\
                   q 1 0 0 rg 0 0 400 200 re f Q";
    let document = open(pdf(content));
    let found = objects(&document, 0);
    let stroke = found[0].clone();
    let before = render(&document, 0);
    assert!(!is_blue(pixel(&before, 100.0, 360.0)), "covered");
    let (front, _, _) = edit(&document, 0, &stroke, ObjectEdit::Arrange(Arrange::Front));
    let after = render(&front, 0);
    // The same stroke, now on top: drawn exactly as it was, where the cover is.
    let line = |b: &Bitmap| (40..240).filter(|x| is_blue(pixel(b, *x as f32, 360.0))).count();
    assert!(line(&after) > 100, "{} blue pixels", line(&after));
    let uncovered =
        open(pdf("q 2 0 0 2 0 0 cm 10 10 150 150 re W n 0 0 1 RG 6 w [4 2] 0 d 20 20 m 120 20 l S Q"));
    let reference = render(&uncovered, 0);
    for x in 40..240 {
        let (a, b) = (pixel(&after, x as f32, 360.0), pixel(&reference, x as f32, 360.0));
        assert_eq!(is_blue(a), is_blue(b), "x {x}: {a:?} vs {b:?}");
    }
}

#[test]
fn a_new_image_lands_upright_where_it_was_put_on_turned_pages() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = open(bytes);
    let png = png_of(30, 20, [0, 0, 220]);
    for page in 0..4 {
        let before = render(&document, page);
        let rect = ViewRect { x: 30.0, y: 40.0, width: 90.0, height: 60.0 };
        let (placed, _, index) =
            document.with_new_image(&NewImage { page, rect, data: png.clone() }).expect("places");
        let after = render(&placed, page);
        assert!(is_blue(pixel(&after, 75.0, 70.0)), "page {page}");
        assert_eq!(changes_outside(&before, &after, &[area_of(&rect)]), 0, "page {page}");
        let object = &objects(&placed, page)[index.expect("found")];
        assert_eq!(object.kind, ObjectKind::Image);
        assert!(
            (object.rect.x - 30.0).abs() < 0.05 && (object.rect.width - 90.0).abs() < 0.05,
            "{:?}",
            object.rect
        );
        // Moving it on the turned page moves it as displayed.
        let (moved, _, _) = edit(&placed, page, object, translate(100.0, 0.0));
        let moved_object = &objects(&moved, page)[index.expect("found")];
        assert!((moved_object.rect.x - 130.0).abs() < 0.05, "page {page}: {:?}", moved_object.rect);
        document = moved;
    }
    qpdf_check(&document.bytes(), None, "placed.pdf");
}

#[test]
fn encrypted_files_are_edited_and_stay_encrypted() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let found = objects(&document, 0);
    assert!(!found.is_empty(), "the encrypted page's content is read");
    let png = png_of(10, 10, [0, 180, 0]);
    let rect = ViewRect { x: 300.0, y: 30.0, width: 50.0, height: 50.0 };
    let (placed, _, index) = document.with_new_image(&NewImage { page: 0, rect, data: png }).expect("places");
    assert!(is_green(pixel(&render(&placed, 0), 325.0, 55.0)));
    let image = objects(&placed, 0)[index.expect("found")].clone();
    let (moved, _, _) = edit(&placed, 0, &image, translate(0.0, 100.0));
    assert!(is_green(pixel(&render(&moved, 0), 325.0, 155.0)));
    let text_block = objects(&moved, 0).into_iter().find(|o| o.kind == ObjectKind::Text).expect("text");
    let (moved, _, _) = edit(&moved, 0, &text_block, translate(0.0, 20.0));
    qpdf_check(&moved.bytes(), password.as_deref(), "encrypted.pdf");
    let reopened = engine().open_bytes(moved.bytes().to_vec(), password.as_deref()).expect("reopens");
    assert_eq!(reopened.page_count(), document.page_count());
}

#[test]
fn arabic_text_blocks_move_and_read_back_in_logical_order() {
    for name in ["chrome/ar-plain", "chrome/mixed-inline"] {
        let (bytes, _) = fixture(name);
        let document = open(bytes);
        let before = text(&document, 0);
        let found = objects(&document, 0);
        // The first block (a heading), a little up, clear of the text below it.
        let block = found.iter().find(|o| o.kind == ObjectKind::Text).expect("a text block").clone();
        let (moved, _, _) = edit(&document, 0, &block, translate(0.0, -4.0));
        let after = text(&moved, 0);
        // The same words, in logical order; only the block's place changed.
        let words = |s: &str| {
            let mut w: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
            w.sort();
            w
        };
        assert_eq!(words(&before), words(&after), "{name}");
        let moved_block = &objects(&moved, 0)[block.index];
        // Its bottom edge moved; its top may meet the clip Chrome draws the page in.
        let bottom = |r: &ViewRect| r.y + r.height;
        assert!(
            (bottom(&moved_block.rect) - bottom(&block.rect) + 4.0).abs() < 0.5,
            "{name}: {:?} → {:?}",
            block.rect,
            moved_block.rect
        );
        qpdf_check(&moved.bytes(), None, "arabic.pdf");
    }
}

#[test]
fn undo_returns_the_original_and_a_save_keeps_the_edit() {
    let bytes = pdf(SCENE);
    let document = open(bytes.clone());
    let red = objects(&document, 0)[0].clone();
    let (moved, update, _) = edit(&document, 0, &red, translate(200.0, 0.0));
    let mut history = EditHistory::new(Arc::from(bytes.clone()));
    history.push_content(update);
    assert_eq!(&*history.undo().expect("undoes"), bytes.as_slice());

    let target = scratch("saved.pdf");
    save(&moved, &target).expect("saves");
    let reopened = engine().open_file(&target, None).expect("reopens");
    let red = objects(&reopened, 0)[0].clone();
    assert!((red.rect.x - 240.0).abs() < 0.01, "{:?}", red.rect);
    // Edited again after the save: still one wrapper.
    let (again, _, _) = edit(&reopened, 0, &red, translate(-200.0, 0.0));
    let red = &objects(&again, 0)[0];
    assert!((red.rect.x - 40.0).abs() < 0.01, "{:?}", red.rect);
    assert!(is_red(pixel(&render(&again, 0), 50.0, 70.0)));
}

#[test]
fn a_stale_reference_is_refused() {
    let document = open(pdf(SCENE));
    let red = objects(&document, 0)[0].clone();
    let (moved, _, _) = edit(&document, 0, &red, translate(5.0, 0.0));
    let result =
        moved.with_object_edit(&ObjectChange { page: 0, object: reference(&red), edit: ObjectEdit::Delete });
    assert!(result.is_err());
}
