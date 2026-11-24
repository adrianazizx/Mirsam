//! Moving page text with the Edit tool (ADR 0051's amendment): text added
//! with the Edit tool (ADR 0021), and a paragraph of the page's own text,
//! moved by the box the Text mode shows for it — drawn where it went and
//! nowhere else, read back there, before and after a save and a reopen, on
//! English and Arabic pages, on turned pages, and in an encrypted file.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use mirsam_core::block_edit::{BlockEdit, TextBlock};
use mirsam_core::edit::EditHistory;
use mirsam_core::fonts::FontFamily;
use mirsam_core::save::save;
use mirsam_core::text_edit::NewText;
use mirsam_core::typeset::{Align, BaseDirection};
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
    let dir = std::env::temp_dir().join(format!("mirsam-moving-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

/// Saves `document`, checks it with qpdf, and opens the saved file.
fn save_and_reopen(document: &Document, password: Option<&str>, name: &str) -> Document {
    let path = scratch(name);
    save(document, &path).expect("saves");
    let out = Command::new("qpdf")
        .args([
            format!("--password={}", password.unwrap_or("")),
            "--check".into(),
            path.to_string_lossy().into(),
        ])
        .output()
        .expect("qpdf runs (install qpdf)");
    assert!(
        matches!(out.status.code(), Some(0 | 3)),
        "qpdf --check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    engine().open_file(&path, password).expect("reopens")
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const SCALE: f32 = 2.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

/// Changed pixels inside either box (grown by 4 pt: a Naskh mark reaches
/// past the extracted box), and outside both.
fn changes(before: &Bitmap, after: &Bitmap, areas: [ViewRect; 2]) -> (usize, usize) {
    let (mut inside, mut outside) = (0, 0);
    for y in 0..after.height() {
        for x in 0..after.width() {
            if before.pixel(x, y) == after.pixel(x, y) {
                continue;
            }
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            let within = areas.iter().any(|a| {
                vx >= a.x - 4.0 && vx <= a.x + a.width + 4.0 && vy >= a.y - 4.0 && vy <= a.y + a.height + 4.0
            });
            if within { inside += 1 } else { outside += 1 }
        }
    }
    (inside, outside)
}

fn new_text(page: usize, (x, y, width): (f32, f32, f32), text: &str, family: FontFamily) -> NewText {
    NewText {
        page,
        x,
        y,
        width,
        text: text.into(),
        family,
        size: NewText::DEFAULT_SIZE,
        color: [0.0, 0.0, 0.0],
        align: Align::Start,
        direction: BaseDirection::Auto,
    }
}

fn rect_of(block: &TextBlock) -> ViewRect {
    ViewRect { x: block.x, y: block.y, width: block.width, height: block.height }
}

/// The paragraph whose text is `text`.
fn block(document: &Document, page: usize, text: &str) -> TextBlock {
    let blocks = document.text_blocks(page).expect("blocks");
    blocks
        .iter()
        .find(|b| !text.trim().is_empty() && normalize(&b.text) == normalize(text))
        .cloned()
        .unwrap_or_else(|| {
            panic!("no paragraph {text:?} in {:?}", blocks.iter().map(|b| &b.text).collect::<Vec<_>>())
        })
}

/// Moves the paragraph `text` by (`dx`, `dy`): drawn where it was and where
/// it went only, found there afterwards with the same text.
fn moved(document: &Document, page: usize, text: &str, d: (f32, f32)) -> (Document, Vec<u8>) {
    moved_block(document, page, &block(document, page, text), d)
}

/// Moves paragraph `from` by (`dx`, `dy`), as the Text mode does: by its
/// box. It is found afterwards where it went, with the same text.
fn moved_block(
    document: &Document,
    page: usize,
    from: &TextBlock,
    (dx, dy): (f32, f32),
) -> (Document, Vec<u8>) {
    let before = render(document, page);
    let (after, update) = document
        .with_text_moved(page, rect_of(from), dx, dy)
        .unwrap_or_else(|e| panic!("{:?}: {e}", from.text));
    let goal = ViewRect { x: from.x + dx, y: from.y + dy, ..rect_of(from) };
    let blocks = after.text_blocks(page).expect("blocks");
    let to = blocks
        .iter()
        .find(|b| {
            (b.x - goal.x).abs() < 1.5
                && (b.y - goal.y).abs() < 1.5
                && normalize(&b.text) == normalize(&from.text)
        })
        .unwrap_or_else(|| {
            panic!(
                "{:?} not found at ({}, {}): {:?}",
                from.text,
                goal.x,
                goal.y,
                blocks.iter().map(|b| (b.x, b.y, b.text.clone())).collect::<Vec<_>>()
            )
        });
    assert!((to.width - from.width).abs() < 1.5, "{:?}: the same width", from.text);
    let (inside, outside) = changes(&before, &render(&after, page), [rect_of(from), goal]);
    assert!(
        inside > 100 && outside == 0,
        "{:?}: {inside} changed where it was and went, {outside} elsewhere",
        from.text
    );
    (after, update)
}

fn added_text_moves(fixture_name: &str, text: &str, family: FontFamily) {
    let (bytes, _) = fixture(fixture_name);
    let original = engine().open_bytes(bytes, None).expect("opens");
    let page_text = normalize(&original.page_text(0).expect("text").text());
    let (added, _, _) =
        original.with_new_text(&new_text(0, (60.0, 600.0, 300.0), text, family)).expect("added");
    let first = block(&added, 0, text);
    // Right after adding it: up and towards the inline end.
    let (document, _) = moved(&added, 0, text, (150.0, -40.0));
    // Saved, reopened, moved again, saved again.
    let reopened = save_and_reopen(&document, None, &format!("{}-once.pdf", fixture_name.replace('/', "-")));
    let (again, _) = moved(&reopened, 0, text, (-100.0, 80.0));
    let twice = save_and_reopen(&again, None, &format!("{}-twice.pdf", fixture_name.replace('/', "-")));
    let last = block(&twice, 0, text);
    assert!(
        (last.x - (first.x + 50.0)).abs() < 1.5 && (last.y - (first.y + 40.0)).abs() < 1.5,
        "({}, {}) after starting at ({}, {})",
        last.x,
        last.y,
        first.x,
        first.y
    );
    // The page's own text is untouched, and the moved text still reads as typed.
    let read = normalize(&twice.page_text(0).expect("text").text());
    assert!(read.contains(&page_text) || page_text.split(' ').all(|w| read.contains(w)), "{read}");
    assert!(read.contains(&normalize(text)), "{read}");
    // And it can still be edited in place where it went (ADR 0023).
    let edited = format!("{text} ✓");
    let target = block(&twice, 0, text);
    assert!(target.editable, "text this app wrote stays editable where it went: {:?}", target.reason);
    {
        let (changed, _, _) = twice
            .with_block_edit(&BlockEdit {
                page: 0,
                index: target.index,
                fingerprint: target.fingerprint,
                text: edited.clone(),
                family: None,
                marks: None,
            })
            .expect("edits where it went");
        let now = block(&changed, 0, &edited);
        assert!((now.y - last.y).abs() < 2.0, "edited in its new place: {} vs {}", now.y, last.y);
    }
}

#[test]
fn added_english_text_moves_before_and_after_a_reopen() {
    added_text_moves("chrome/en-paragraphs", "Approved by the board on 12 March 2026.", FontFamily::Sans);
}

#[test]
fn added_arabic_text_moves_before_and_after_a_reopen() {
    added_text_moves("chrome/ar-plain", "تمت الموافقة على العقد بتاريخ ١٢ مارس", FontFamily::Naskh);
}

#[test]
fn a_paragraph_of_the_pages_own_text_moves_and_the_rest_stays() {
    // CoreText's paragraphs can be edited; Chrome's cannot (their fonts are
    // not reused), and move all the same: by their boxes, their bytes kept.
    for name in ["coretext/en-paragraphs", "coretext/ar-plain", "chrome/en-paragraphs", "chrome/ar-plain"] {
        // CoreText's pages are made on macOS only; elsewhere the corpus says so and they are not
        // checked there (Chrome's are made everywhere and always are).
        let manifest = manifest();
        let Some(fixture) = manifest.fixture_or_unavailable(name) else { continue };
        let bytes = std::fs::read(fixture.file()).expect("fixture readable");
        let document = engine().open_bytes(bytes, None).expect("opens");
        let page_text = normalize(&document.page_text(0).expect("text").text());
        let blocks = document.text_blocks(0).expect("blocks");
        // CoreText's Arabic lists each letter's dots (glyphs with no text) as
        // paragraphs of no width: they move with the paragraph around them.
        let last =
            blocks.iter().rev().find(|b| b.width > 60.0 && b.height > 1.0).expect("paragraphs").clone();
        let within = |b: &TextBlock| {
            let (cx, cy) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
            cx >= last.x && cx <= last.x + last.width && cy >= last.y && cy <= last.y + last.height
        };
        // The last paragraph, down the page by 200 pt, where the page is empty.
        let (after, _) = moved_block(&document, 0, &last, (0.0, 200.0));
        let now = after.text_blocks(0).expect("blocks");
        // CoreText's Arabic draws dots as glyphs with no text, which the
        // extractor regroups with whatever line is near (ADR 0051): its
        // paragraphs are checked by their pixels (above), not by their boxes.
        let reads_well = name != "coretext/ar-plain";
        for other in blocks.iter().filter(|b| reads_well && b.index != last.index && !within(b)) {
            assert!(
                now.iter().any(|b| (b.x - other.x).abs() < 0.5
                    && (b.y - other.y).abs() < 0.5
                    && b.text == other.text),
                "{name}: paragraph {:?} stayed where it was",
                other.text
            );
        }
        // Every word is still there, read in order within each paragraph.
        let read = normalize(&after.page_text(0).expect("text").text());
        for word in page_text.split(' ').filter(|_| reads_well) {
            assert!(read.contains(word), "{name}: {word:?} lost: {read}");
        }
        if last.editable {
            assert!(read.ends_with(&normalize(&last.text)), "{name}: the moved paragraph reads last: {read}");
        }
    }
}

#[test]
fn on_turned_pages_moved_text_keeps_its_look() {
    let (bytes, _) = fixture("qpdf/rotated");
    let mut document = engine().open_bytes(bytes, None).expect("opens");
    for page in 0..4 {
        let text = "Moved — نقل";
        let base = render(&document, page);
        document = document
            .with_new_text(&new_text(page, (10.0, 10.0, 200.0), text, FontFamily::Sans))
            .expect("added")
            .0;
        let from = block(&document, page, text);
        let placed = render(&document, page);
        let (dx, dy) = (0.0, 480.0);
        document = moved(&document, page, text, (dx, dy)).0;
        let after = render(&document, page);
        let (mut drawn, mut differ) = (0, 0);
        let (x0, y0) = ((from.x * SCALE) as u32, (from.y * SCALE) as u32);
        for y in y0..y0 + (from.height * SCALE) as u32 {
            for x in x0..x0 + (from.width * SCALE) as u32 {
                let old = placed.pixel(x, y).expect("on the page");
                if base.pixel(x, y) == Some(old) {
                    continue;
                }
                drawn += 1;
                let new = after.pixel(x + (dx * SCALE) as u32, y + (dy * SCALE) as u32).expect("on the page");
                if old.iter().zip(new.iter()).any(|(a, b)| a.abs_diff(*b) > 24) {
                    differ += 1;
                }
            }
        }
        assert!(drawn > 50, "page {page}: drawn");
        assert!(
            differ * 20 < drawn,
            "page {page}: {differ} of {drawn} pixels differ: not upright where it went"
        );
    }
}

#[test]
fn a_move_is_one_undo_step_and_nothing_there_is_refused() {
    let (bytes, _) = fixture("chrome/ar-plain");
    let original = engine().open_bytes(bytes.clone(), None).expect("opens");
    let mut history = EditHistory::new(Arc::from(bytes));
    let text = "نص مضاف";
    let (added, update, _) =
        original.with_new_text(&new_text(0, (60.0, 600.0, 200.0), text, FontFamily::Naskh)).expect("added");
    history.push_content(update);
    let (moved_once, update) = moved(&added, 0, text, (40.0, 40.0));
    history.push_content(update);
    let undone = moved_once.revision(history.undo().expect("undo")).expect("opens");
    assert_eq!(undone.bytes(), added.bytes(), "undo returns the bytes before the move");
    let empty = ViewRect { x: 400.0, y: 780.0, width: 20.0, height: 20.0 };
    assert!(matches!(added.with_text_moved(0, empty, 10.0, 0.0), Err(Error::InvalidEdit(_))));
}

#[test]
fn moving_text_in_an_encrypted_file() {
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = engine().open_bytes(bytes, password.as_deref()).expect("opens");
    let text = "سري — Confidential";
    let view_text = new_text(0, (60.0, 600.0, 240.0), text, FontFamily::Sans);
    let (added, _, height) = document.with_new_text(&view_text).expect("added");
    // Encrypted pages list no paragraphs to edit: the Text mode moves by the box it drew.
    let region = ViewRect { x: 60.0, y: 600.0, width: 240.0, height };
    let before = render(&added, 0);
    let (after, _) = added.with_text_moved(0, region, 100.0, -60.0).expect("moves");
    let goal = ViewRect { x: 160.0, y: 540.0, ..region };
    let (inside, outside) = changes(&before, &render(&after, 0), [region, goal]);
    assert!(inside > 100 && outside == 0, "{inside} inside, {outside} outside");
    let reopened = save_and_reopen(&after, password.as_deref(), "encrypted.pdf");
    let read = normalize(&reopened.page_text(0).expect("text").text());
    assert!(read.contains("Confidential"), "{read}");
}
