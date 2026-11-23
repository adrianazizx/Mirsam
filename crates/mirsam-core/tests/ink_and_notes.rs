//! Ink and sticky notes as incremental updates (ADR 0018), on real files:
//! PDFium reopens and renders each result, and reads back what was written
//! (an Arabic note's text, ink strokes); `qpdf --check` passes on each.

mod common;

use std::process::Command;
use std::sync::Arc;

use common::manifest;
use mirsam_core::annotate::{AnnotationEdit, AnnotationKind, Ink, InkPoint, NOTE_SIZE, Note};
use mirsam_core::edit::EditHistory;
use mirsam_core::{Bitmap, Document, Engine, Error, PdfiumSource};

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

fn qpdf_check(bytes: &[u8], password: Option<&str>) {
    let dir = std::env::temp_dir().join(format!("mirsam-ink-notes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{}.pdf", bytes.len()));
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

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

/// The pixel displayed at page view point (`x`, `y`).
fn at(bitmap: &Bitmap, x: f32, y: f32) -> [u8; 4] {
    bitmap.pixel((x * SCALE) as u32, (y * SCALE) as u32).expect("on the page")
}

fn blank([r, g, b, _]: [u8; 4]) -> bool {
    r > 245 && g > 245 && b > 245
}

/// Pixels that changed outside the view rectangle (x0, y0, x1, y1), with a
/// pixel's margin.
fn changed_outside(before: &Bitmap, after: &Bitmap, (x0, y0, x1, y1): (f32, f32, f32, f32)) -> usize {
    let mut count = 0;
    for y in 0..before.height() {
        for x in 0..before.width() {
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            if vx >= x0 - 1.0 && vx <= x1 + 1.0 && vy >= y0 - 1.0 && vy <= y1 + 1.0 {
                continue;
            }
            if before.pixel(x, y) != after.pixel(x, y) {
                count += 1;
            }
        }
    }
    count
}

/// Rows painted in the column at view x, between view rows `y0` and `y1`.
fn thickness(bitmap: &Bitmap, x: f32, y0: f32, y1: f32) -> usize {
    let column = (x * SCALE) as u32;
    ((y0 * SCALE) as u32..(y1 * SCALE) as u32)
        .filter(|&row| bitmap.pixel(column, row).is_some_and(|pixel| !blank(pixel)))
        .count()
}

/// One stroke through view points, all at `pressure`.
fn ink(document: &Document, page: usize, points: &[(f32, f32, f32)]) -> AnnotationEdit {
    let view = document.view_transform(page).expect("view");
    AnnotationEdit::AddInk {
        page,
        ink: Ink {
            strokes: vec![points.iter().map(|&(x, y, p)| InkPoint::from_view(x, y, p, &view)).collect()],
            color: Ink::DEFAULT_COLOR,
            width: Ink::DEFAULT_WIDTH,
        },
    }
}

fn note(document: &Document, page: usize, x: f32, y: f32, contents: &str) -> AnnotationEdit {
    let view = document.view_transform(page).expect("view");
    AnnotationEdit::AddNote {
        page,
        note: Note {
            rect: Note::rect_at(x, y, &view),
            contents: contents.into(),
            color: Note::DEFAULT_COLOR,
        },
    }
}

fn apply(document: &Document, edit: &AnnotationEdit) -> (Document, Vec<u8>) {
    document.with_annotation_edit(edit).expect("the edit applies")
}

#[test]
fn ink_is_drawn_where_the_pen_went_and_wider_where_pressed() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    // Below the text: a stroke at mouse pressure, then pressed hard.
    let stroke = [(100.0, 600.0, 0.5), (200.0, 600.0, 0.5), (210.0, 600.0, 1.0), (300.0, 600.0, 1.0)];
    let before = render(&document, 0);
    let (edited, _) = apply(&document, &ink(&document, 0, &stroke));
    let after = render(&edited, 0);
    assert_eq!(
        changed_outside(&before, &after, (95.0, 595.0, 305.0, 605.0)),
        0,
        "drawn only along the stroke"
    );
    let light = thickness(&after, 150.0, 590.0, 610.0);
    let heavy = thickness(&after, 260.0, 590.0, 610.0);
    // 2 pt at mouse pressure is 4 px at this scale; 3 pt pressed hard is 6.
    assert!((3..=6).contains(&light), "{light} px at mouse pressure");
    assert!(heavy > light, "{heavy} px pressed vs {light} px");

    // Listed back with its stroke in page space.
    let (annotations, view) = edited.page_annotations(0).expect("annotations");
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].kind, AnnotationKind::Ink);
    let shown: Vec<(f32, f32)> = annotations[0].strokes[0].iter().map(|&(x, y)| view.point(x, y)).collect();
    assert_eq!(shown.len(), 4);
    for ((x, y), (ex, ey, _)) in shown.iter().zip(stroke) {
        assert!((x - ex).abs() < 0.01 && (y - ey).abs() < 0.01, "({x}, {y}) vs ({ex}, {ey})");
    }
    qpdf_check(&edited.bytes(), None);
}

#[test]
fn ink_on_a_turned_page_lands_where_it_was_drawn() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    for page in 0..4 {
        let size = document.page_size(page).expect("size");
        let (x, y) = (size.width / 2.0, size.height - 40.0);
        let before = render(&document, page);
        document = apply(&document, &ink(&document, page, &[(x - 50.0, y, 0.5), (x + 50.0, y, 0.5)])).0;
        let after = render(&document, page);
        assert!(!blank(at(&after, x, y)), "page {page}: the stroke is under the pen");
        assert_eq!(
            changed_outside(&before, &after, (x - 55.0, y - 5.0, x + 55.0, y + 5.0)),
            0,
            "page {page}"
        );
    }
}

#[test]
fn an_arabic_note_is_kept_and_its_icon_stands_upright_on_turned_pages() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    let text = "راجع البند ٤ قبل التوقيع — check clause 4";
    for page in 0..4 {
        let before = render(&document, page);
        document = apply(&document, &note(&document, page, 50.0, 50.0, text)).0;
        let after = render(&document, page);
        assert_eq!(changed_outside(&before, &after, (50.0, 50.0, 50.0 + NOTE_SIZE, 50.0 + NOTE_SIZE)), 0);
        // Upright as displayed: the bubble's tail hangs at the bottom left,
        // and the bottom right below the bubble is empty.
        let (x, y) = (50.0, 50.0);
        assert!(!blank(at(&after, x + 5.0, y + 16.5)), "page {page}: the tail is at the bottom left");
        assert!(blank(at(&after, x + 16.0, y + 18.5)), "page {page}: nothing at the bottom right");
        assert!(!blank(at(&after, x + 16.0, y + 4.0)), "page {page}: the bubble is at the top");

        let (annotations, view) = document.page_annotations(page).expect("annotations");
        let listed = &annotations[0];
        assert_eq!(listed.kind, AnnotationKind::Note);
        assert_eq!(listed.contents.as_deref(), Some(text));
        let shown = view.rect(listed.rects[0]);
        assert!((shown.x - 50.0).abs() < 0.01 && (shown.y - 50.0).abs() < 0.01, "{shown:?}");
    }
    qpdf_check(&document.bytes(), None);
}

#[test]
fn a_notes_text_changes_in_place_and_undo_brings_the_old_text_back() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let document = engine().open_bytes(bytes.clone(), None).expect("opens");
    let mut history = EditHistory::new(Arc::from(bytes));
    let (added, update) = apply(&document, &note(&document, 0, 30.0, 30.0, "أول"));
    history.push_annotations(update);
    let (changed, update) =
        apply(&added, &AnnotationEdit::SetContents { page: 0, index: 0, contents: "ثانٍ — second".into() });
    history.push_annotations(update);
    let contents = |document: &Document| document.page_annotations(0).expect("listed").0[0].contents.clone();
    assert_eq!(contents(&changed).as_deref(), Some("ثانٍ — second"));
    // Still one annotation: the same object, rewritten.
    assert_eq!(changed.page_annotations(0).expect("listed").0.len(), 1);
    let undone = document.revision(history.undo().expect("undo")).expect("opens");
    assert_eq!(contents(&undone).as_deref(), Some("أول"));
    qpdf_check(&changed.bytes(), None);

    // Only a note's text can be set.
    let (inked, _) = apply(&added, &ink(&added, 0, &[(100.0, 100.0, 0.5), (120.0, 120.0, 0.5)]));
    assert!(matches!(
        inked.with_annotation_edit(&AnnotationEdit::SetContents { page: 0, index: 1, contents: "x".into() }),
        Err(Error::InvalidEdit(_))
    ));
}

#[test]
fn notes_and_ink_in_an_encrypted_file() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let text = "سري — confidential";
    let (noted, _) = apply(&document, &note(&document, 0, 40.0, 40.0, text));
    let (changed, _) =
        apply(&noted, &AnnotationEdit::SetContents { page: 0, index: 0, contents: format!("{text}!") });
    let (inked, _) = apply(&changed, &ink(&changed, 0, &[(100.0, 700.0, 0.5), (200.0, 700.0, 0.5)]));
    let (annotations, _) = inked.page_annotations(0).expect("annotations");
    assert_eq!(annotations[0].contents.as_deref(), Some(format!("{text}!").as_str()));
    // The ink's appearance decrypts and draws; its strokes are read back.
    assert!(!blank(at(&render(&inked, 0), 150.0, 700.0)));
    assert_eq!(annotations[1].kind, AnnotationKind::Ink);
    assert_eq!(annotations[1].strokes.first().map(Vec::len), Some(2));
    qpdf_check(&inked.bytes(), password.as_deref());
}

#[test]
fn the_eraser_removes_whole_strokes_in_one_step() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let edits: Vec<AnnotationEdit> = (0..3)
        .map(|i| {
            let y = 620.0 + 20.0 * i as f32;
            ink(&document, 0, &[(100.0, y, 0.5), (300.0, y, 0.5)])
        })
        .collect();
    let (inked, _) = document.with_annotation_edits(&edits).expect("three strokes");
    assert_eq!(inked.page_annotations(0).expect("listed").0.len(), 3);
    // Erasing the first and the last: highest index first, so the other stays put.
    let (erased, _) = inked
        .with_annotation_edits(&[
            AnnotationEdit::Remove { page: 0, index: 2 },
            AnnotationEdit::Remove { page: 0, index: 0 },
        ])
        .expect("erases");
    let (left, view) = erased.page_annotations(0).expect("listed");
    assert_eq!(left.len(), 1);
    let (_, y) = view.point(left[0].strokes[0][0].0, left[0].strokes[0][0].1);
    assert!((y - 640.0).abs() < 0.01, "the middle stroke is left: {y}");
    qpdf_check(&erased.bytes(), None);
}
