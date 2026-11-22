//! Editing text in place (ADR 0024), against spec §7's acceptance tests:
//! fixing a typo in English keeps every other line where it was; a word
//! inserted into Arabic joins, grows its line or re-wraps right to left, and
//! reads back in logical order; a mixed sentence keeps its order; and open → edit → save →
//! reopen → edit again drifts nowhere and leaves one set of fonts. PDFium
//! renders every result, our extractor reads it back, `qpdf --check` passes.

mod common;

use std::process::Command;

use common::manifest;
use safha_core::save::save;
use safha_core::text_edit::{BlockAlign, BlockEditor};
use safha_core::typeset::MatchKind;
use safha_core::view::ViewRect;
use safha_core::{Bitmap, Document, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn open(name: &str) -> Document {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|f| f.name() == name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    engine().open_file(&fixture.file(), fixture.password.as_deref()).expect("opens")
}

fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn qpdf_check(document: &Document, name: &str) {
    let path = std::env::temp_dir().join(format!("safha-in-place-{}-{name}.pdf", std::process::id()));
    std::fs::write(&path, document.bytes().as_ref()).expect("write");
    let output = Command::new("qpdf").arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(output.status.code(), Some(0 | 3)), "{}", String::from_utf8_lossy(&output.stdout));
}

const SCALE: f32 = 2.0;

/// Pixels changed outside `area` (view points, with a margin).
fn changed_outside(before: &Bitmap, after: &Bitmap, area: ViewRect) -> usize {
    let (x0, y0) = ((area.x - 2.0) * SCALE, (area.y - 2.0) * SCALE);
    let (x1, y1) = ((area.x + area.width + 2.0) * SCALE, (area.y + area.height + 2.0) * SCALE);
    let mut count = 0;
    for y in 0..before.height() {
        for x in 0..before.width() {
            let (fx, fy) = (x as f32, y as f32);
            if fx >= x0 && fx <= x1 && fy >= y0 && fy <= y1 {
                continue;
            }
            count += usize::from(before.pixel(x, y) != after.pixel(x, y));
        }
    }
    count
}

/// Pixels that differ inside `area`, and dark pixels there before.
fn differ_inside(before: &Bitmap, after: &Bitmap, area: ViewRect) -> (usize, usize) {
    let (mut differ, mut ink) = (0, 0);
    let x0 = (area.x * SCALE).max(0.0) as u32;
    let y0 = (area.y * SCALE).max(0.0) as u32;
    let x1 = (((area.x + area.width) * SCALE) as u32).min(before.width());
    let y1 = (((area.y + area.height) * SCALE) as u32).min(before.height());
    for y in y0..y1 {
        for x in x0..x1 {
            let (a, b) = (before.pixel(x, y), after.pixel(x, y));
            let dark = |p: Option<[u8; 4]>| p.is_some_and(|p| p[0] < 128 && p[1] < 128 && p[2] < 128);
            ink += usize::from(dark(a));
            differ += usize::from(dark(a) != dark(b));
        }
    }
    (differ, ink)
}

/// The block's box as displayed, from a fresh analysis.
fn shown(document: &Document, block: usize) -> ViewRect {
    let analysis = document.edit_blocks(0).expect("analyses");
    document.view_transform(0).expect("view").rect(analysis.blocks[block].bounds)
}

fn editor(document: &Document, block: usize) -> BlockEditor {
    document.block_editor(0, block, false).unwrap_or_else(|e| panic!("block {block}: {e}"))
}

#[test]
fn fixing_a_typo_in_english_keeps_every_other_line_where_it_was() {
    let document = open("coretext/en-paragraphs");
    let edit = editor(&document, 1);
    assert_eq!(edit.font.used, "IBMPlexSans");
    assert_eq!(edit.font.kind, MatchKind::Exact, "the paragraph's own font, bundled");
    assert!(!edit.mixed, "one size and colour throughout");
    assert_eq!(
        words(&edit.text),
        "The office staff finished the final draft efficiently; the affluent client found it difficult to fault."
    );
    let area = shown(&document, 1);
    let before = document.render_page(0, SCALE).expect("renders");

    // The same text again, in the same font at the same places: what you
    // see after editing is what was there.
    let (same, _) = document.with_replaced_block(&edit, &edit.text).expect("replaces");
    let after = same.render_page(0, SCALE).expect("renders");
    assert_eq!(changed_outside(&before, &after, area), 0);
    let (differ, ink) = differ_inside(&before, &after, area);
    assert!(differ * 20 < ink, "re-set unchanged: {differ} of {ink} ink pixels differ");

    // The typo fixed: "efficiently" → "efficaciously" (longer: it re-wraps inside the box).
    let fixed = edit.text.replace("efficiently", "efficaciously");
    let (edited, _) = document.with_replaced_block(&edit, &fixed).expect("replaces");
    let after = edited.render_page(0, SCALE).expect("renders");
    assert_eq!(changed_outside(&before, &after, area), 0, "every other line where it was");
    let analysis = edited.edit_blocks(0).expect("analyses");
    let block = analysis.blocks.iter().find(|b| words(&b.text) == words(&fixed)).expect("reads back exactly");
    let original = document.edit_blocks(0).expect("analyses");
    let baseline = |b: &safha_core::text_edit::EditBlock| b.lines[0].baseline;
    assert!((baseline(block) - baseline(&original.blocks[1])).abs() < 0.05, "the first baseline stays");
    assert!(block.written_here && block.align == BlockAlign::Left);
    qpdf_check(&edited, "typo");
}

#[test]
fn a_word_inserted_into_arabic_joins_grows_the_line_leftwards_then_rewraps_and_reads_back() {
    let document = open("coretext/ar-sans");
    let analysis = document.edit_blocks(0).expect("analyses");
    // The longest paragraph, the last on the page: one line, room below it to grow.
    let index = analysis.blocks.len() - 1;
    assert_eq!(analysis.blocks[index].lines.len(), 1);
    let edit = editor(&document, index);
    assert_eq!((edit.font.used.as_str(), edit.font.kind), ("IBMPlexSansArabic", MatchKind::Exact));
    let area = shown(&document, index);
    let before = document.render_page(0, SCALE).expect("renders");
    let right = |b: &safha_core::text_edit::EditBlock| b.lines[0].right;
    let insert = |added: &str| {
        let words_in: Vec<&str> = edit.text.split(' ').collect();
        let middle = words_in.len() / 2;
        [&words_in[..middle], &[added], &words_in[middle..]].concat().join(" ")
    };

    // A word in the middle: one line still, longer to the left, its right edge kept.
    let one_word = insert("الجديدة");
    let (edited, _) = document.with_replaced_block(&edit, &one_word).expect("replaces");
    let after = edited.render_page(0, SCALE).expect("renders");
    // Only the line's own band changes, from the page's left edge to the block's right.
    let band = ViewRect { x: 0.0, width: area.x + area.width, ..area };
    assert_eq!(changed_outside(&before, &after, band), 0);
    let read = edited.edit_blocks(0).expect("analyses");
    let block =
        read.blocks.iter().find(|b| words(&b.text) == words(&one_word)).expect("reads back in logical order");
    assert!(block.written_here);
    assert_eq!(block.lines.len(), 1, "a single line grows along itself");
    assert!(block.lines[0].left < analysis.blocks[index].lines[0].left - 10.0, "it grew leftwards");
    assert!((right(block) - right(&analysis.blocks[index])).abs() < 1.0);

    // More than the page has room for on one line: it re-wraps, right to left.
    let many = insert(&["وهذه كلمات أخرى تضاف حتى يلتف السطر إلى سطر جديد"; 4].join(" "));
    let (edited, _) = document.with_replaced_block(&edit, &many).expect("replaces");
    let read = edited.edit_blocks(0).expect("analyses");
    let block =
        read.blocks.iter().find(|b| words(&b.text) == words(&many)).expect("reads back in logical order");
    assert!(block.lines.len() > 1, "it re-wrapped");
    assert_eq!(block.align, BlockAlign::Right, "lines start at the right");
    // Right to left: every line starts at the original's right edge (the
    // first exactly; the others by their first letter's side bearing).
    for (n, line) in block.lines.iter().enumerate() {
        let slack = if n == 0 { 1.0 } else { 0.25 * edit.size };
        let want = right(&analysis.blocks[index]);
        assert!((line.right - want).abs() < slack, "line {n}: {} vs {want}", line.right);
    }
    qpdf_check(&edited, "arabic");
}

#[test]
fn a_heading_grows_along_its_line_instead_of_onto_the_next_paragraph() {
    // Chrome's mixed page: its heading is one line right above a paragraph.
    let document = open("chrome/mixed-inline");
    let edit = editor(&document, 0);
    let changed = format!("{} تعديل", edit.text);
    let (laid, _) = edit.layout(&changed).expect("lays out");
    assert_eq!(laid.lines.len(), 1, "a longer heading stays one line");
    let (edited, _) = document.with_replaced_block(&edit, &changed).expect("replaces");
    let read = edited.edit_blocks(0).expect("analyses");
    assert!(read.blocks.iter().any(|b| b.written_here && words(&b.text) == words(&changed)));
    // The paragraph below reads as it did.
    let original = document.edit_blocks(0).expect("analyses");
    assert!(read.blocks.iter().any(|b| b.text == original.blocks[1].text));
}

#[test]
fn a_mixed_sentence_keeps_its_order() {
    // Chrome's page: CoreText's draws this Arabic with a broken character
    // mapping, which is refused (below).
    let document = open("chrome/mixed-inline");
    let analysis = document.edit_blocks(0).expect("analyses");
    let index = analysis.blocks.iter().position(|b| b.text.contains("iPhone")).expect("the mixed sentence");
    let edit = editor(&document, index);
    // Chrome embeds the font unnamed (Type 3). Its Latin letters and digits
    // are Noto Naskh Arabic's widths exactly, so that is the substitute, and
    // the sentence still fits on its one line.
    assert_eq!(edit.font.used, "NotoNaskhArabic-Regular", "{:?}", edit.font);
    assert_eq!(edit.font.kind, MatchKind::Substitute);
    assert_eq!(edit.layout(&edit.text).expect("lays out").0.lines.len(), 1);
    // The number and the product name change; the sentence keeps its length.
    let changed = edit.text.replace("iPhone 15 Pro", "iPhone 16 Pro").replace("اشتريت 3", "اشتريت 4");
    assert_ne!(changed, edit.text);
    let (edited, _) = document.with_replaced_block(&edit, &changed).expect("replaces");
    let read = words(&edited.page_text(0).expect("text").text());
    assert!(read.contains(&words(&changed)), "order kept: {read}");
    qpdf_check(&edited, "mixed");
}

#[test]
fn edit_save_reopen_edit_again_drifts_nowhere_and_keeps_one_set_of_fonts() {
    let document = open("coretext/en-paragraphs");
    let first = editor(&document, 2);
    let once = first.text.replace("12.5%", "13.0%");
    let (edited, _) = document.with_replaced_block(&first, &once).expect("replaces");
    let path = std::env::temp_dir().join(format!("safha-in-place-roundtrip-{}.pdf", std::process::id()));
    save(&edited, &path).expect("saves");
    let reopened = engine().open_file(&path, None).expect("reopens");
    let _ = std::fs::remove_file(&path);

    let analysis = reopened.edit_blocks(0).expect("analyses");
    let index =
        analysis.blocks.iter().position(|b| words(&b.text) == words(&once)).expect("found after reopening");
    let block = &analysis.blocks[index];
    assert!(block.written_here, "still known as this app's text after a full save");
    let again = editor(&reopened, index);
    assert_eq!(again.font.used, "IBMPlexSans");
    let twice = once.replace("13.0%", "14.5%");
    let (edited_again, _) = reopened.with_replaced_block(&again, &twice).expect("replaces again");
    let after = edited_again.edit_blocks(0).expect("analyses");
    let block_again = after.blocks.iter().find(|b| words(&b.text) == words(&twice)).expect("reads back");
    let original = document.edit_blocks(0).expect("analyses");
    for (a, b) in block_again.lines.iter().zip(&original.blocks[2].lines) {
        assert!((a.baseline - b.baseline).abs() < 0.05, "no drift: {} vs {}", a.baseline, b.baseline);
        assert!((a.left - b.left).abs() < 0.5, "no drift: {} vs {}", a.left, b.left);
    }
    // One set of the app's fonts on the page: the second edit's.
    let ours: Vec<&str> = after.ops.iter().filter(|op| op.mirsam).map(|op| op.font.as_str()).collect();
    let page = String::from_utf8_lossy(edited_again.bytes().as_ref()).into_owned();
    let last_resources =
        page.rfind("/Resources").map(|at| &page[at..(at + 400).min(page.len())]).unwrap_or("");
    let fonts_listed = last_resources.matches("/MT").count();
    assert!(!ours.is_empty());
    assert!(fonts_listed <= 2, "one set of fonts, not two: {last_resources}");
    qpdf_check(&edited_again, "roundtrip");
}

#[test]
fn blocks_that_cannot_be_traced_are_refused() {
    // Text drawn together with another block's, or partly untraceable, is
    // not edited in place: its other glyphs would stay, or another's go.
    let document = open("coretext/ar-lam-alef");
    let analysis = document.edit_blocks(0).expect("analyses");
    let index = analysis.blocks.iter().position(|b| b.untraced > 0).expect("a block with untraced marks");
    assert!(matches!(document.block_editor(0, index, false), Err(Error::EditUnsupported(_))));
    assert!(matches!(document.block_editor(0, 99, false), Err(Error::InvalidEdit(_))));
    // A page whose text is not what it draws (CoreText's Noto Naskh: dotless
    // letter bodies mapped to `#` and `%`) would be written back as that.
    let suspect = open("coretext/mixed-inline");
    assert!(matches!(suspect.block_editor(0, 1, false), Err(Error::EditUnsupported(_))));
}

#[test]
fn blanking_a_block_removes_it() {
    let document = open("coretext/en-paragraphs");
    let edit = editor(&document, 3);
    let (edited, _) = document.with_replaced_block(&edit, "  ").expect("removes");
    let read = words(&edited.page_text(0).expect("text").text());
    assert!(!read.contains(&words(&edit.text)));
    assert!(read.contains("Quarterly Report"));
}
