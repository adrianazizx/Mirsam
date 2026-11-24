//! Moving what a user placed on a page (ADR 0045's amendment): signatures,
//! Fill & Sign text and marks, text boxes, notes and drawings, each moved
//! as one `SetRect` and drawn where it went and nowhere else — right after
//! placing it and after the file is saved and reopened, on English and
//! Arabic pages, on turned pages, and in an encrypted file.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use mirsam_core::annotate::{
    AnnotationEdit, AnnotationKind, DrawnSignature, FillText, Ink, InkPoint, Mark, MarkKind, Markup,
    MarkupKind, Note, Quad, Signature, SignaturePoint, SignatureStamp, TextBox,
};
use mirsam_core::edit::EditHistory;
use mirsam_core::save::save;
use mirsam_core::view::ViewRect;
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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mirsam-moving-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(path: &std::path::Path, password: Option<&str>) {
    let password = format!("--password={}", password.unwrap_or(""));
    let out = Command::new("qpdf")
        .args([password.as_str(), "--check", &path.to_string_lossy()])
        .output()
        .expect("qpdf runs (install qpdf)");
    assert!(
        matches!(out.status.code(), Some(0 | 3)),
        "qpdf --check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Saves `document` and opens the saved file again, checked by qpdf.
fn save_and_reopen(document: &Document, password: Option<&str>, name: &str) -> Document {
    let path = scratch(name);
    save(document, &path).expect("saves");
    qpdf_check(&path, password);
    engine().open_file(&path, password).expect("reopens")
}

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

fn within(area: ViewRect, (x, y): (f32, f32)) -> bool {
    x >= area.x - 1.5
        && x <= area.x + area.width + 1.5
        && y >= area.y - 1.5
        && y <= area.y + area.height + 1.5
}

/// Changed pixels inside either rectangle, and outside both.
fn changes(before: &Bitmap, after: &Bitmap, areas: [ViewRect; 2]) -> (usize, usize) {
    let (mut inside, mut outside) = (0, 0);
    for y in 0..after.height() {
        for x in 0..after.width() {
            if before.pixel(x, y) == after.pixel(x, y) {
                continue;
            }
            let point = (x as f32 / SCALE, y as f32 / SCALE);
            if areas.iter().any(|area| within(*area, point)) { inside += 1 } else { outside += 1 }
        }
    }
    (inside, outside)
}

/// The displayed box of the annotation at `index`, with its kind.
fn shown(document: &Document, page: usize, index: usize) -> (AnnotationKind, ViewRect) {
    let (annotations, view) = document.page_annotations(page).expect("listed");
    let found = annotations.iter().find(|a| a.index == index).expect("still there");
    (found.kind, view.rect(found.rects[0]))
}

/// `SetRect` giving the annotation at `index` the displayed box `to`.
fn set_rect(document: &Document, page: usize, index: usize, to: ViewRect) -> AnnotationEdit {
    let view = document.view_transform(page).expect("view");
    let a = view.page_point(to.x, to.y);
    let b = view.page_point(to.x + to.width, to.y + to.height);
    AnnotationEdit::SetRect { page, index, rect: [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)] }
}

/// Moves the annotation at `index` by (`dx`, `dy`) displayed points: one
/// update, drawn where it went and nowhere else, its box moved exactly.
fn moved_by(document: &Document, page: usize, index: usize, (dx, dy): (f32, f32)) -> (Document, Vec<u8>) {
    let (kind, from) = shown(document, page, index);
    let to = ViewRect { x: from.x + dx, y: from.y + dy, ..from };
    let before = render(document, page);
    let (after, update) =
        document.with_annotation_edit(&set_rect(document, page, index, to)).expect("the move applies");
    let (kind_after, now) = shown(&after, page, index);
    assert_eq!(kind_after, kind, "the same annotation");
    assert!(
        (now.x - to.x).abs() < 0.05
            && (now.y - to.y).abs() < 0.05
            && (now.width - from.width).abs() < 0.05
            && (now.height - from.height).abs() < 0.05,
        "{kind:?} moved to {now:?}, not {to:?}"
    );
    let (inside, outside) = changes(&before, &render(&after, page), [from, to]);
    assert!(
        inside > 20 && outside == 0,
        "{kind:?}: {inside} changed where it was and went, {outside} elsewhere"
    );
    (after, update)
}

fn drawn() -> Signature {
    let point = |x: f32, y: f32, pressure: f32| SignaturePoint { x, y, pressure };
    Signature::Drawn(DrawnSignature {
        strokes: vec![
            (0..40).map(|i| point(i as f32 * 5.0, 40.0 + (i as f32 * 0.6).sin() * 25.0, 0.5)).collect(),
            vec![point(30.0, 80.0, 0.8), point(170.0, 75.0, 0.3)],
        ],
        width: 4.0,
    })
}

/// One of each kind a user places, down the inline-start margin of `page`
/// (in the bottom half, below the fixtures' text): (kind, its index).
fn place_everything(
    document: &Document,
    page: usize,
    arabic: bool,
) -> (Document, Vec<(AnnotationKind, usize)>) {
    let view = document.view_transform(page).expect("view");
    let (said, note_text, box_text) = if arabic {
        ("نعم، أوافق", "راجع هذه الفقرة", "تمت المراجعة — Checked")
    } else {
        ("Yes, I agree", "Check this paragraph", "Checked — تمت المراجعة")
    };
    let top = 420.0;
    let edits = vec![
        AnnotationEdit::AddSignature {
            page,
            stamp: SignatureStamp::at(40.0, top, 90.0, drawn(), Signature::DEFAULT_COLOR, &view)
                .expect("fits"),
        },
        AnnotationEdit::AddSignature {
            page,
            stamp: SignatureStamp::at(
                40.0,
                top + 60.0,
                60.0,
                Signature::Typed(mirsam_core::annotate::TypedSignature {
                    text: if arabic { "م. ع".into() } else { "A. A.".into() },
                    family: mirsam_core::fonts::FontFamily::Naskh,
                }),
                Signature::DEFAULT_COLOR,
                &view,
            )
            .expect("fits"),
        },
        AnnotationEdit::AddFillText {
            page,
            text: FillText::at(40.0, top + 110.0, said, 11.0, FillText::DEFAULT_COLOR, &view).expect("fits"),
        },
        AnnotationEdit::AddMark {
            page,
            mark: Mark::at(40.0, top + 140.0, 14.0, MarkKind::Check, Mark::DEFAULT_COLOR, &view),
        },
        AnnotationEdit::AddTextBox {
            page,
            text_box: {
                let height = TextBox::height_for(box_text, 160.0, TextBox::DEFAULT_SIZE).expect("fits");
                TextBox {
                    rect: TextBox::rect_at(40.0, top + 170.0, 160.0, height, &view),
                    contents: box_text.into(),
                    size: TextBox::DEFAULT_SIZE,
                    color: TextBox::DEFAULT_COLOR,
                }
            },
        },
        AnnotationEdit::AddNote {
            page,
            note: Note {
                rect: Note::rect_at(40.0, top + 220.0, &view),
                contents: note_text.into(),
                color: Note::DEFAULT_COLOR,
            },
        },
        AnnotationEdit::AddInk {
            page,
            ink: Ink {
                strokes: vec![
                    (0..30)
                        .map(|i| {
                            let x = 45.0 + i as f32 * 3.0;
                            InkPoint::from_view(x, top + 265.0 + (i as f32 * 0.5).sin() * 8.0, 0.5, &view)
                        })
                        .collect(),
                ],
                color: Ink::DEFAULT_COLOR,
                width: Ink::DEFAULT_WIDTH,
            },
        },
    ];
    let mut placed = Vec::new();
    let mut current: Option<Document> = None;
    for edit in edits {
        let next = current.as_ref().unwrap_or(document).with_annotation_edit(&edit).expect("placed").0;
        let document = current.insert(next);
        let (annotations, _) = document.page_annotations(page).expect("listed");
        let last = annotations.last().expect("one");
        placed.push((last.kind, last.index));
    }
    let kinds: Vec<AnnotationKind> = placed.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds,
        [
            AnnotationKind::Signature,
            AnnotationKind::Signature,
            AnnotationKind::FillText,
            AnnotationKind::Mark,
            AnnotationKind::TextBox,
            AnnotationKind::Note,
            AnnotationKind::Ink,
        ]
    );
    (current.expect("placed"), placed)
}

/// Every kind is moved right after placing it, then again after a save and
/// a reopen, and the second save keeps it where it went.
fn every_kind_moves(fixture_name: &str, arabic: bool) {
    let (bytes, _) = fixture(fixture_name);
    let document = engine().open_bytes(bytes, None).expect("opens");
    let (mut document, placed) = place_everything(&document, 0, arabic);
    let first: Vec<ViewRect> = placed.iter().map(|&(_, index)| shown(&document, 0, index).1).collect();
    // Towards the inline end, as an Arabic user would drag across the page.
    for &(_, index) in &placed {
        document = moved_by(&document, 0, index, (200.0, 3.0)).0;
    }
    let mut reopened =
        save_and_reopen(&document, None, &format!("{}-once.pdf", fixture_name.replace('/', "-")));
    for &(kind, index) in &placed {
        let (again, now) = shown(&reopened, 0, index);
        assert_eq!(again, kind, "kinds read back after a reopen");
        assert!(now.x > 230.0, "{kind:?} stayed where it went: {now:?}");
        // Back towards the inline start, and down.
        reopened = moved_by(&reopened, 0, index, (-150.0, 12.0)).0;
    }
    let twice = save_and_reopen(&reopened, None, &format!("{}-twice.pdf", fixture_name.replace('/', "-")));
    for (&(kind, index), first) in placed.iter().zip(&first) {
        let (_, now) = shown(&twice, 0, index);
        let (x, y) = (first.x + 50.0, first.y + 15.0);
        assert!(
            (now.x - x).abs() < 0.1 && (now.y - y).abs() < 0.1,
            "{kind:?} where it was moved last: {now:?}"
        );
    }
    // A note and a text box keep their text; ink keeps its strokes, moved along.
    let (annotations, view) = twice.page_annotations(0).expect("listed");
    let text_box = annotations.iter().find(|a| a.kind == AnnotationKind::TextBox).expect("text box");
    assert!(text_box.contents.as_deref().is_some_and(|text| text.contains("تمت المراجعة")));
    let ink = annotations.iter().find(|a| a.kind == AnnotationKind::Ink).expect("ink");
    let (first_x, _) = view.point(ink.strokes[0][0].0, ink.strokes[0][0].1);
    assert!((first_x - 95.0).abs() < 0.1, "the strokes moved with the drawing: {first_x}");
}

#[test]
fn every_kind_moves_on_an_english_page_before_and_after_a_reopen() {
    every_kind_moves("chrome/en-paragraphs", false);
}

#[test]
fn every_kind_moves_on_an_arabic_page_before_and_after_a_reopen() {
    every_kind_moves("chrome/ar-plain", true);
}

#[test]
fn on_turned_pages_a_moved_item_keeps_its_look() {
    // Pages turned 0, 90, 180 and 270 degrees: what was drawn in the old box
    // is drawn, pixel for pixel, in the new one.
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    for page in 0..4 {
        let view = document.view_transform(page).expect("view");
        let text = "Received — استلم";
        let height = TextBox::height_for(text, 150.0, TextBox::DEFAULT_SIZE).expect("fits");
        let edits = [
            AnnotationEdit::AddTextBox {
                page,
                text_box: TextBox {
                    rect: TextBox::rect_at(10.0, 10.0, 150.0, height, &view),
                    contents: text.into(),
                    size: TextBox::DEFAULT_SIZE,
                    color: TextBox::DEFAULT_COLOR,
                },
            },
            AnnotationEdit::AddSignature {
                page,
                stamp: SignatureStamp::at(10.0, 60.0, 80.0, drawn(), Signature::DEFAULT_COLOR, &view)
                    .expect("fits"),
            },
        ];
        for edit in edits {
            let base = render(&document, page);
            document = document.with_annotation_edit(&edit).expect("placed").0;
            let index = document.page_annotations(page).expect("listed").0.last().expect("one").index;
            let (_, from) = shown(&document, page, index);
            let before = render(&document, page);
            // Whole pixels at this scale, so the drawing can be compared exactly.
            let (dx, dy) = (0.0, 480.0);
            document = moved_by(&document, page, index, (dx, dy)).0;
            let after = render(&document, page);
            let (mut differ, mut drawn_pixels) = (0, 0);
            let (x0, y0) = ((from.x * SCALE) as u32, (from.y * SCALE) as u32);
            for y in y0..y0 + (from.height * SCALE) as u32 {
                for x in x0..x0 + (from.width * SCALE) as u32 {
                    // The item's own pixels: those its placing changed.
                    let old = before.pixel(x, y).expect("on the page");
                    if base.pixel(x, y) == Some(old) {
                        continue;
                    }
                    drawn_pixels += 1;
                    let new =
                        after.pixel(x + (dx * SCALE) as u32, y + (dy * SCALE) as u32).expect("on the page");
                    if old.iter().zip(new.iter()).any(|(a, b)| a.abs_diff(*b) > 24) {
                        differ += 1;
                    }
                }
            }
            assert!(drawn_pixels > 50, "page {page}: something was drawn");
            assert!(
                differ * 20 < drawn_pixels,
                "page {page}: {differ} of {drawn_pixels} pixels differ after the move ({from:?})"
            );
        }
    }
}

#[test]
fn text_boxes_reflow_drawings_scale_notes_only_move_and_markup_stays_on_its_text() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let (document, placed) = place_everything(&document, 0, false);
    let index_of = |kind: AnnotationKind| placed.iter().find(|(k, _)| *k == kind).expect("placed").1;

    // A note's icon keeps its size.
    let note = index_of(AnnotationKind::Note);
    let (_, from) = shown(&document, 0, note);
    let larger = ViewRect { width: from.width * 1.5, height: from.height * 1.5, ..from };
    assert!(matches!(
        document.with_annotation_edit(&set_rect(&document, 0, note, larger)),
        Err(Error::InvalidEdit(_))
    ));

    // A text box half as wide: its text wraps, its top-left corner stays, and it grows down.
    let text_box = index_of(AnnotationKind::TextBox);
    let (_, from) = shown(&document, 0, text_box);
    let narrow = ViewRect { width: from.width / 2.0, ..from };
    let (reflowed, _) =
        document.with_annotation_edit(&set_rect(&document, 0, text_box, narrow)).expect("reflows");
    let (_, now) = shown(&reflowed, 0, text_box);
    assert!((now.x - from.x).abs() < 0.05 && (now.y - from.y).abs() < 0.05, "{now:?}");
    assert!((now.width - narrow.width).abs() < 0.05, "{now:?}");
    assert!(now.height > from.height * 1.5, "two lines or more: {now:?} vs {from:?}");
    let (annotations, _) = reflowed.page_annotations(0).expect("listed");
    let contents = annotations.iter().find(|a| a.index == text_box).and_then(|a| a.contents.clone());
    assert_eq!(contents.as_deref(), Some("Checked — تمت المراجعة"));

    // A drawing twice as large: its strokes scale with it, from its top-left corner.
    let ink = index_of(AnnotationKind::Ink);
    let (_, from) = shown(&document, 0, ink);
    let twice = ViewRect { width: from.width * 2.0, height: from.height * 2.0, ..from };
    let before = render(&document, 0);
    let (scaled, _) = document.with_annotation_edit(&set_rect(&document, 0, ink, twice)).expect("scales");
    let (_, now) = shown(&scaled, 0, ink);
    assert!((now.width - twice.width).abs() < 0.05 && (now.height - twice.height).abs() < 0.05, "{now:?}");
    let (inside, outside) = changes(&before, &render(&scaled, 0), [from, twice]);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");
    let (annotations, view) = scaled.page_annotations(0).expect("listed");
    let strokes = &annotations.iter().find(|a| a.index == ink).expect("ink").strokes;
    let last = strokes[0].last().expect("points");
    let (x, _) = view.point(last.0, last.1);
    // The last point was 87 pt into a 91 pt box (2 pt from its end): now 174 pt into a 182 pt one.
    assert!((x - (from.x + 2.0 * (45.0 + 29.0 * 3.0 - from.x))).abs() < 0.1, "{x} in {now:?}");

    // None of them goes to another page this way.
    for &(kind, index) in placed.iter().filter(|(kind, _)| kind.is_comment_movable()) {
        let to_page = AnnotationEdit::MoveToPage { page: 0, index, to_page: 0, rect: [0.0, 0.0, 10.0, 10.0] };
        assert!(matches!(document.with_annotation_edit(&to_page), Err(Error::InvalidEdit(_))), "{kind:?}");
    }
    // A highlight belongs to the words it marks.
    let view = document.view_transform(0).expect("view");
    let highlight = AnnotationEdit::AddMarkup {
        page: 0,
        markup: Markup {
            kind: MarkupKind::Highlight,
            quads: vec![Quad::from_view(ViewRect { x: 72.0, y: 72.0, width: 100.0, height: 14.0 }, &view)],
            color: MarkupKind::Highlight.default_color(),
        },
    };
    let (marked, _) = document.with_annotation_edit(&highlight).expect("marked");
    let index = marked.page_annotations(0).expect("listed").0.last().expect("one").index;
    let to = ViewRect { x: 200.0, y: 300.0, width: 100.0, height: 14.0 };
    assert!(matches!(
        marked.with_annotation_edit(&set_rect(&marked, 0, index, to)),
        Err(Error::InvalidEdit(_))
    ));
}

#[test]
fn a_move_is_one_undo_step() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let document = engine().open_bytes(bytes.clone(), None).expect("opens");
    let mut history = EditHistory::new(Arc::from(bytes));
    let (placed, update) = document
        .with_annotation_edit(&AnnotationEdit::AddInk {
            page: 0,
            ink: Ink {
                strokes: vec![
                    (0..20)
                        .map(|i| {
                            InkPoint::from_view(
                                100.0 + i as f32 * 4.0,
                                500.0,
                                0.5,
                                &document.view_transform(0).expect("view"),
                            )
                        })
                        .collect(),
                ],
                color: Ink::DEFAULT_COLOR,
                width: Ink::DEFAULT_WIDTH,
            },
        })
        .expect("drawn");
    history.push_annotations(update);
    let (_, from) = shown(&placed, 0, 0);
    let (moved, update) = moved_by(&placed, 0, 0, (-60.0, 30.0));
    history.push_annotations(update);
    let undone = moved.revision(history.undo().expect("undo")).expect("opens");
    assert_eq!(undone.bytes(), placed.bytes(), "undo returns the bytes before the move");
    let (_, back) = shown(&undone, 0, 0);
    assert!((back.x - from.x).abs() < 0.01 && (back.y - from.y).abs() < 0.01);
}

#[test]
fn moving_in_an_encrypted_file() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let (document, placed) = place_everything(&document, 0, true);
    let mut document = document;
    for &(_, index) in &placed {
        document = moved_by(&document, 0, index, (180.0, -20.0)).0;
    }
    let mut reopened = save_and_reopen(&document, password.as_deref(), "encrypted.pdf");
    for &(_, index) in &placed {
        reopened = moved_by(&reopened, 0, index, (-100.0, 0.0)).0;
    }
    let again = save_and_reopen(&reopened, password.as_deref(), "encrypted-twice.pdf");
    let (annotations, _) = again.page_annotations(0).expect("listed");
    let note = annotations.iter().find(|a| a.kind == AnnotationKind::Note).expect("note");
    assert_eq!(note.contents.as_deref(), Some("راجع هذه الفقرة"), "strings stay readable");
}
