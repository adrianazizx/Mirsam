//! Fill & Sign (ADR 0045) on real files: text, dates, marks, and drawn,
//! typed, and image signatures placed anywhere, drawn by PDFium where they
//! were placed and nowhere else, moved and resized, read back in logical
//! order when merged into the page, kept out of the comments, and kept by
//! a save — in English and Arabic, on turned pages, and in encrypted files.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::manifest;
use mirsam_core::annotate::{
    AnnotationEdit, AnnotationKind, DrawnSignature, FillText, Mark, MarkKind, Signature, SignaturePoint,
    SignatureStamp, TypedSignature,
};
use mirsam_core::fonts::FontFamily;
use mirsam_core::save::save;
use mirsam_core::view::ViewRect;
use mirsam_core::{Bitmap, Document, Engine, PdfiumSource};

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
    let dir = std::env::temp_dir().join(format!("mirsam-fill-sign-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn flattened(document: &Document, password: Option<&str>, name: &str) -> Document {
    let input = scratch(&format!("{name}-in.pdf"));
    let output = scratch(&format!("{name}-flat.pdf"));
    std::fs::write(&input, document.bytes()).expect("write");
    let password = format!("--password={}", password.unwrap_or(""));
    let (input, output) = (input.to_string_lossy().into_owned(), output.to_string_lossy().into_owned());
    for args in [
        vec![password.as_str(), "--check", &input],
        vec![&password, "--decrypt", "--flatten-annotations=all", &input, &output],
    ] {
        let out = Command::new("qpdf").args(&args).output().expect("qpdf runs (install qpdf)");
        assert!(
            matches!(out.status.code(), Some(0 | 3)),
            "qpdf {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    engine().open_file(std::path::Path::new(&output), None).expect("the flattened file opens")
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

/// Changed pixels inside a displayed rectangle, and outside it.
fn changes(before: &Bitmap, after: &Bitmap, area: ViewRect) -> (usize, usize) {
    let (mut inside, mut outside) = (0, 0);
    for y in 0..after.height() {
        for x in 0..after.width() {
            if before.pixel(x, y) == after.pixel(x, y) {
                continue;
            }
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            let within = vx >= area.x - 1.0
                && vx <= area.x + area.width + 1.0
                && vy >= area.y - 1.0
                && vy <= area.y + area.height + 1.0;
            if within { inside += 1 } else { outside += 1 }
        }
    }
    (inside, outside)
}

fn apply(document: &Document, edit: AnnotationEdit) -> Document {
    document.with_annotation_edit(&edit).expect("the edit applies").0
}

/// The last annotation's displayed box.
fn last_box(document: &Document, page: usize) -> (usize, AnnotationKind, ViewRect) {
    let (annotations, view) = document.page_annotations(page).expect("listed");
    let last = annotations.last().expect("one");
    (last.index, last.kind, view.rect(last.rects[0]))
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

fn png_signature() -> Signature {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 60, 20);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("header");
        // A dark bar across a transparent ground.
        let pixels: Vec<u8> = (0..60 * 20)
            .flat_map(|i| {
                if (i / 60) % 20 > 6 && (i / 60) % 20 < 13 { [20, 30, 120, 255] } else { [255, 255, 255, 0] }
            })
            .collect();
        writer.write_image_data(&pixels).expect("pixels");
    }
    Signature::Image(out)
}

#[test]
fn text_marks_and_signatures_are_drawn_where_they_are_placed() {
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    let view = document.view_transform(0).expect("view");
    let today = "٢٢ نوفمبر ٢٠٢٦";
    let edits = vec![
        AnnotationEdit::AddFillText {
            page: 0,
            text: FillText::at(
                60.0,
                520.0,
                "محمد عبدالله العلي (Mohammed)",
                12.0,
                FillText::DEFAULT_COLOR,
                &view,
            )
            .expect("fits"),
        },
        AnnotationEdit::AddFillText {
            page: 0,
            text: FillText::at(60.0, 560.0, today, 11.0, FillText::DEFAULT_COLOR, &view).expect("fits"),
        },
        AnnotationEdit::AddMark {
            page: 0,
            mark: Mark::at(60.0, 600.0, Mark::DEFAULT_SIZE, MarkKind::Check, Mark::DEFAULT_COLOR, &view),
        },
        AnnotationEdit::AddMark {
            page: 0,
            mark: Mark::at(90.0, 600.0, Mark::DEFAULT_SIZE, MarkKind::Cross, Mark::DEFAULT_COLOR, &view),
        },
        AnnotationEdit::AddSignature {
            page: 0,
            stamp: SignatureStamp::at(60.0, 640.0, 150.0, drawn(), Signature::DEFAULT_COLOR, &view)
                .expect("fits"),
        },
        AnnotationEdit::AddSignature {
            page: 0,
            stamp: SignatureStamp::at(
                260.0,
                640.0,
                150.0,
                Signature::Typed(TypedSignature {
                    text: "سارة الحربي".into(), family: FontFamily::Naskh
                }),
                Signature::DEFAULT_COLOR,
                &view,
            )
            .expect("fits"),
        },
        AnnotationEdit::AddSignature {
            page: 0,
            stamp: SignatureStamp::at(420.0, 640.0, 120.0, png_signature(), [0.0; 3], &view).expect("fits"),
        },
    ];
    let kinds = [
        AnnotationKind::FillText,
        AnnotationKind::FillText,
        AnnotationKind::Mark,
        AnnotationKind::Mark,
        AnnotationKind::Signature,
        AnnotationKind::Signature,
        AnnotationKind::Signature,
    ];
    for (edit, kind) in edits.into_iter().zip(kinds) {
        let before = render(&document, 0);
        document = apply(&document, edit);
        let (_, listed, area) = last_box(&document, 0);
        assert_eq!(listed, kind);
        let (inside, outside) = changes(&before, &render(&document, 0), area);
        assert!(inside > 40, "{kind:?}: {inside} pixels in its box {area:?}");
        assert_eq!(outside, 0, "{kind:?}: drawn outside its box");
    }
    // An image signature keeps its aspect: 120 × 40.
    let (_, _, image) = last_box(&document, 0);
    assert!((image.width - 120.0).abs() < 0.1 && (image.height - 40.0).abs() < 0.1, "{image:?}");
    // None of it is a comment.
    assert!(document.comments().expect("comments").is_empty());

    // Merged into the page, the text and the typed signature read as typed.
    let flat = flattened(&document, None, "all");
    let read = normalize(&flat.page_text(0).expect("text").text());
    // On this English page the extractor reads the mixed line left to right,
    // so its two scripts are checked apart.
    for expected in ["محمد عبدالله العلي", "(Mohammed)", today, "سارة الحربي"] {
        assert!(read.contains(expected), "{expected} missing from:\n{read}");
    }

    // Saved through PDFium and reopened, they are all still there.
    let path = scratch("signed.pdf");
    save(&document, &path).expect("saves");
    let reopened = engine().open_file(&path, None).expect("reopens");
    let (annotations, _) = reopened.page_annotations(0).expect("listed");
    assert_eq!(annotations.iter().map(|a| a.kind).collect::<Vec<_>>(), kinds);
}

#[test]
fn items_move_resize_and_change_their_text() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let view = document.view_transform(0).expect("view");
    let stamp =
        SignatureStamp::at(100.0, 600.0, 120.0, drawn(), Signature::DEFAULT_COLOR, &view).expect("fits");
    let signed = apply(&document, AnnotationEdit::AddSignature { page: 0, stamp });
    let (index, _, area) = last_box(&signed, 0);

    // Twice as large, its top-left corner elsewhere.
    let (x, y) = (300.0, 500.0);
    let corners = [(x, y), (x + area.width * 2.0, y + area.height * 2.0)];
    let page: Vec<(f32, f32)> = corners.iter().map(|(x, y)| view.page_point(*x, *y)).collect();
    let rect = [
        page[0].0.min(page[1].0),
        page[0].1.min(page[1].1),
        page[0].0.max(page[1].0),
        page[0].1.max(page[1].1),
    ];
    let before = render(&document, 0);
    let moved = apply(&signed, AnnotationEdit::SetRect { page: 0, index, rect });
    let (_, _, now) = last_box(&moved, 0);
    assert!((now.x - x).abs() < 0.01 && (now.width - area.width * 2.0).abs() < 0.01, "{now:?}");
    let (inside, outside) = changes(&before, &render(&moved, 0), now);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");

    // Fill text set again keeps its corner and its (resized) size.
    let text = FillText::at(60.0, 100.0, "نعم", 10.0, FillText::DEFAULT_COLOR, &view).expect("fits");
    let filled = apply(&moved, AnnotationEdit::AddFillText { page: 0, text });
    let (index, _, small) = last_box(&filled, 0);
    let page: Vec<(f32, f32)> =
        [(small.x, small.y), (small.x + small.width * 2.0, small.y + small.height * 2.0)]
            .iter()
            .map(|(x, y)| view.page_point(*x, *y))
            .collect();
    let rect = [
        page[0].0.min(page[1].0),
        page[0].1.min(page[1].1),
        page[0].0.max(page[1].0),
        page[0].1.max(page[1].1),
    ];
    let larger = apply(&filled, AnnotationEdit::SetRect { page: 0, index, rect });
    let changed = apply(
        &larger,
        AnnotationEdit::SetContents {
            page: 0, index, contents: "نعم، أوافق على الشروط".into()
        },
    );
    let (annotations, view) = changed.page_annotations(0).expect("listed");
    let shown = view.rect(annotations[index].rects[0]);
    assert_eq!(annotations[index].contents.as_deref(), Some("نعم، أوافق على الشروط"));
    assert!(
        (shown.x - small.x).abs() < 0.01 && (shown.y - small.y).abs() < 0.01,
        "the corner stays: {shown:?}"
    );
    // Twice the size: the padding around the text does not grow with it.
    assert!(shown.height > small.height * 1.8, "the doubled size stays: {shown:?} vs {small:?}");

    // Only Fill & Sign items move this way; removing works as for comments.
    let removed = apply(&changed, AnnotationEdit::Remove { page: 0, index });
    assert_eq!(removed.page_annotations(0).expect("listed").0.len(), 1);
}

#[test]
fn on_turned_pages_items_stand_upright_where_they_were_placed() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    for page in 0..4 {
        let view = document.view_transform(page).expect("view");
        let before = render(&document, page);
        let text = FillText::at(20.0, 20.0, "تم — Done", 12.0, FillText::DEFAULT_COLOR, &view).expect("fits");
        document = apply(&document, AnnotationEdit::AddFillText { page, text });
        let (_, _, area) = last_box(&document, page);
        assert!((area.x - 20.0).abs() < 0.01 && (area.y - 20.0).abs() < 0.01, "page {page}: {area:?}");
        assert!(area.width > area.height, "page {page}: upright, wider than tall: {area:?}");
        let (inside, outside) = changes(&before, &render(&document, page), area);
        assert!(inside > 50 && outside == 0, "page {page}: {inside} inside, {outside} outside");
    }
}

#[test]
fn signatures_in_encrypted_files() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let view = document.view_transform(0).expect("view");
    let typed = Signature::Typed(TypedSignature {
        text: "خالد بن سعد العتيبي Khalid".into(),
        family: FontFamily::Sans,
    });
    let stamp = SignatureStamp::at(60.0, 600.0, 140.0, typed, Signature::DEFAULT_COLOR, &view).expect("fits");
    let signed = apply(&document, AnnotationEdit::AddSignature { page: 0, stamp });
    let image = SignatureStamp::at(260.0, 600.0, 90.0, png_signature(), [0.0; 3], &view).expect("fits");
    let signed = apply(&signed, AnnotationEdit::AddSignature { page: 0, stamp: image });
    let flat = flattened(&signed, password.as_deref(), "encrypted");
    let read = normalize(&flat.page_text(0).expect("text").text());
    assert!(read.contains("خالد بن سعد العتيبي") && read.contains("Khalid"), "{read}");
    assert_eq!(signed.page_annotations(0).expect("listed").0.len(), 2);
}

/// Pixels inside a displayed rectangle that are clearly red (dark ink of
/// another colour, or paper, is not).
fn red_inside(bitmap: &Bitmap, area: ViewRect) -> usize {
    let mut red = 0;
    for y in (area.y * SCALE) as u32..((area.y + area.height) * SCALE) as u32 {
        for x in (area.x * SCALE) as u32..((area.x + area.width) * SCALE) as u32 {
            if let Some([r, g, b, _]) = bitmap.pixel(x, y)
                && r > 140
                && g < 100
                && b < 100
            {
                red += 1;
            }
        }
    }
    red
}

#[test]
fn text_and_marks_change_colour_in_place_and_keep_it() {
    const RED: [f32; 3] = [0.72, 0.11, 0.11];
    for (fixture_name, text) in [("chrome/ar-plain", "موافق عليه"), ("qpdf/encrypted", "Approved — موافق")]
    {
        let (bytes, password) = fixture(fixture_name);
        let mut document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
        let view = document.view_transform(0).expect("view");
        let fill = FillText::at(60.0, 600.0, text, 24.0, FillText::DEFAULT_COLOR, &view).expect("fits");
        document = apply(&document, AnnotationEdit::AddFillText { page: 0, text: fill });
        let mark = Mark::at(60.0, 660.0, 30.0, MarkKind::Cross, Mark::DEFAULT_COLOR, &view);
        document = apply(&document, AnnotationEdit::AddMark { page: 0, mark });
        let (annotations, _) = document.page_annotations(0).expect("listed");
        let (text_index, mark_index) = (annotations[0].index, annotations[1].index);
        let boxes: Vec<ViewRect> = annotations.iter().map(|a| view.rect(a.rects[0])).collect();
        let black = render(&document, 0);
        assert!(boxes.iter().all(|area| red_inside(&black, *area) == 0), "{fixture_name}: black at first");

        for index in [text_index, mark_index] {
            document = apply(&document, AnnotationEdit::SetColor { page: 0, index, color: RED });
        }
        let red = render(&document, 0);
        for (area, what) in boxes.iter().zip(["text", "mark"]) {
            assert!(red_inside(&red, *area) > 40, "{fixture_name}: the {what} is red now");
        }
        // The same items, in the same boxes; the text reads as before.
        let (after, _) = document.page_annotations(0).expect("listed");
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].contents.as_deref(), Some(text));
        assert_eq!(view.rect(after[0].rects[0]), boxes[0]);
        // Changed later, the text stays red.
        document = apply(
            &document,
            AnnotationEdit::SetContents { page: 0, index: text_index, contents: format!("{text} ✓") },
        );
        let (changed, _) = document.page_annotations(0).expect("listed");
        assert!(red_inside(&render(&document, 0), view.rect(changed[0].rects[0])) > 40, "{fixture_name}");
        let flat =
            flattened(&document, password.as_deref(), &format!("colour-{}", fixture_name.replace('/', "-")));
        assert!(normalize(&flat.page_text(0).expect("text").text()).contains(text), "{fixture_name}");
    }

    // Signatures keep their own colours.
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let document = engine().open_bytes(bytes, None).expect("opens");
    let view = document.view_transform(0).expect("view");
    let stamp =
        SignatureStamp::at(60.0, 600.0, 120.0, drawn(), Signature::DEFAULT_COLOR, &view).expect("fits");
    let signed = apply(&document, AnnotationEdit::AddSignature { page: 0, stamp });
    let index = signed.page_annotations(0).expect("listed").0[0].index;
    let refused = signed.with_annotation_edit(&AnnotationEdit::SetColor { page: 0, index, color: RED });
    assert!(matches!(refused, Err(mirsam_core::Error::InvalidEdit(_))));
}

/// The page-space box displayed at `area` on a page.
fn page_rect(view: &mirsam_core::view::ViewTransform, area: ViewRect) -> [f32; 4] {
    let [(x0, y0), (x1, y1)] =
        [(area.x, area.y), (area.x + area.width, area.y + area.height)].map(|(x, y)| view.page_point(x, y));
    [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
}

#[test]
fn items_move_to_another_page_and_stand_upright_there() {
    // Pages turned 0, 90, 180 and 270 degrees.
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    let view = document.view_transform(0).expect("view");
    let text = FillText::at(30.0, 40.0, "منقول — Moved", 14.0, FillText::DEFAULT_COLOR, &view).expect("fits");
    let before = [render(&document, 0), render(&document, 1), render(&document, 2)];
    document = apply(&document, AnnotationEdit::AddFillText { page: 0, text });
    let (index, _, placed) = last_box(&document, 0);

    // To the page turned a quarter, at the same place as displayed.
    let to = document.view_transform(1).expect("view");
    let moved = apply(
        &document,
        AnnotationEdit::MoveToPage { page: 0, index, to_page: 1, rect: page_rect(&to, placed) },
    );
    assert!(moved.page_annotations(0).expect("listed").0.is_empty(), "it left page 1");
    let (_, kind, shown) = last_box(&moved, 1);
    assert_eq!(kind, AnnotationKind::FillText);
    assert!((shown.x - placed.x).abs() < 0.01 && (shown.width - placed.width).abs() < 0.01, "{shown:?}");
    assert!(shown.width > shown.height, "upright, wider than tall: {shown:?}");
    // Drawn in its new box only; its old page is as it was.
    let (inside, outside) = changes(&before[1], &render(&moved, 1), shown);
    assert!(inside > 50 && outside == 0, "{inside} inside, {outside} outside");
    let (_, left_behind) = changes(&before[0], &render(&moved, 0), shown);
    assert_eq!(left_behind, 0, "nothing stays on the first page");

    // Once more, to the upside-down page: still upright, and still readable.
    let (index, _, _) = last_box(&moved, 1);
    let to = moved.view_transform(2).expect("view");
    let again = apply(
        &moved,
        AnnotationEdit::MoveToPage { page: 1, index, to_page: 2, rect: page_rect(&to, placed) },
    );
    let (_, _, shown) = last_box(&again, 2);
    let (inside, outside) = changes(&before[2], &render(&again, 2), shown);
    assert!(inside > 50 && outside == 0, "{inside} inside, {outside} outside");
    let path = scratch("moved.pdf");
    save(&again, &path).expect("saves");
    let reopened = engine().open_file(&path, None).expect("reopens");
    assert_eq!(reopened.page_annotations(2).expect("listed").0.len(), 1);
    assert!(reopened.page_annotations(1).expect("listed").0.is_empty());

    // Only Fill & Sign items move between pages.
    let (bytes, _) = fixture("chrome/en-paragraphs");
    let plain = engine().open_bytes(bytes, None).expect("opens");
    let view = plain.view_transform(0).expect("view");
    let note = mirsam_core::annotate::Note {
        rect: mirsam_core::annotate::Note::rect_at(10.0, 10.0, &view),
        contents: "x".into(),
        color: mirsam_core::annotate::Note::DEFAULT_COLOR,
    };
    let noted = apply(&plain, AnnotationEdit::AddNote { page: 0, note });
    let refused = noted.with_annotation_edit(&AnnotationEdit::MoveToPage {
        page: 0,
        index: 0,
        to_page: 0,
        rect: [10.0, 10.0, 30.0, 30.0],
    });
    assert!(matches!(refused, Err(mirsam_core::Error::InvalidEdit(_))));
}
