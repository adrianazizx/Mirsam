//! A page's text as editable blocks (ADR 0023), on the whole fixture corpus:
//! every block traces to the content-stream operations that drew it, and
//! taking those off the page removes exactly that block (PDFium renders the
//! result: no pixel changes outside it, its ink goes). Styles, alignment and
//! line spacing read back from text this app wrote with known settings.

mod common;

use std::process::Command;

use common::manifest;
use safha_core::fonts::FontFamily;
use safha_core::text::Direction;
use safha_core::text_edit::{BlockAlign, EditBlock, NewText};
use safha_core::typeset::{Align, BaseDirection};
use safha_core::{Bitmap, Document, Engine, PdfiumSource};

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

type Area = (f32, f32, f32, f32);

fn inside((x0, y0, x1, y1): Area, x: f32, y: f32) -> bool {
    x >= x0 && x <= x1 && y >= y0 && y <= y1
}

/// Pixels changed outside `area`; and dark pixels inside `own` (and in none
/// of `others`) before and after.
fn compare(before: &Bitmap, after: &Bitmap, area: Area, own: Area, others: &[Area]) -> (usize, usize, usize) {
    let (mut outside, mut ink_before, mut ink_after) = (0, 0, 0);
    let dark = |p: Option<[u8; 4]>| p.is_some_and(|p| p[0] < 160 && p[1] < 160 && p[2] < 160);
    for y in 0..before.height() {
        for x in 0..before.width() {
            let (fx, fy) = (x as f32, y as f32);
            if !inside(area, fx, fy) {
                outside += usize::from(before.pixel(x, y) != after.pixel(x, y));
            } else if inside(own, fx, fy) && !others.iter().any(|&other| inside(other, fx, fy)) {
                ink_before += usize::from(dark(before.pixel(x, y)));
                ink_after += usize::from(dark(after.pixel(x, y)));
            }
        }
    }
    (outside, ink_before, ink_after)
}

/// Takes block `index` off page 0 and checks that exactly it went: nothing
/// changes outside it (with a margin for marks that reach past its box), and
/// its ink, where no other block overlaps it, is gone.
fn remove_and_check(
    document: &Document,
    before: &Bitmap,
    label: &str,
    index: usize,
    blocks: &[EditBlock],
    ops: &[safha_core::text_edit::SourceOp],
) -> Document {
    let block = &blocks[index];
    let sources: Vec<_> = block.sources.iter().map(|&i| ops[i].clone()).collect();
    let (edited, _) =
        document.without_text(0, &sources).unwrap_or_else(|e| panic!("{label} block {index}: {e}"));
    let view = document.view_transform(0).expect("view");
    let area_of = |b: &EditBlock, margin: f32| {
        let r = view.rect(b.bounds);
        (r.x - margin, r.y - margin, r.x + r.width + margin, r.y + r.height + margin)
    };
    let others: Vec<Area> =
        blocks.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, b)| area_of(b, 2.0)).collect();
    let (outside, ink_before, ink_after) = compare(
        before,
        &edited.render_page(0, 1.0).expect("renders"),
        area_of(block, 3.0),
        area_of(block, 0.0),
        &others,
    );
    assert_eq!(outside, 0, "{label} block {index} ({:?}): pixels changed outside it", block.text);
    assert!(
        ink_after * 10 <= ink_before,
        "{label} block {index} ({:?}): ink {ink_before} -> {ink_after}",
        block.text
    );
    edited
}

#[test]
fn every_fixture_block_traces_to_its_operations_and_removing_them_removes_it() {
    let (mut blocks, mut untraced, mut removed, mut letters) = (0, 0, 0, 0);
    for fixture in &manifest().fixtures {
        let Ok(document) = engine().open_file(&fixture.file(), fixture.password.as_deref()) else { continue };
        let analysis = document.edit_blocks(0).expect("analyses");
        assert!(analysis.complete, "{}: its content reads", fixture.name());
        let before = document.render_page(0, 1.0).expect("renders");
        // The blocks are the extractor's, in reading order.
        let page_text = document.page_text(0).expect("text");
        assert_eq!(
            analysis.blocks.iter().map(|b| words(&b.text)).collect::<Vec<_>>(),
            page_text
                .blocks
                .iter()
                .map(|b| words(&b.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ")))
                .collect::<Vec<_>>(),
            "{}",
            fixture.name()
        );
        for (index, block) in analysis.blocks.iter().enumerate() {
            blocks += 1;
            untraced += block.untraced;
            letters += block.text.chars().filter(|c| !c.is_whitespace()).count();
            // Runs cover the block's text, in order, each with a size.
            assert_eq!(
                block.runs.first().map(|r| r.range.start),
                Some(0),
                "{} block {index}",
                fixture.name()
            );
            assert_eq!(
                block.runs.last().map(|r| r.range.end),
                Some(block.text.len()),
                "{} block {index}",
                fixture.name()
            );
            assert!(block.runs.windows(2).all(|w| w[0].range.end == w[1].range.start));
            assert!(block.runs.iter().all(|r| r.style.size > 4.0));
            if block.untraced == 0 && !block.shared {
                remove_and_check(&document, &before, &fixture.name(), index, &analysis.blocks, &analysis.ops);
                removed += 1;
            }
        }
    }
    // Measured on macOS (ADR 0023): 6 of 8,450 characters (marks on two
    // CoreText pages) trace to no operation, which leaves 2 of 146 blocks not
    // removed. Other platforms make their own fixtures: shares, not counts.
    assert!(untraced * 200 <= letters, "{untraced} of {letters} characters untraced");
    assert!(removed * 50 >= blocks * 49, "{removed} of {blocks} blocks removed cleanly");
}

#[test]
fn styles_alignment_and_spacing_read_back_from_text_this_app_wrote() {
    let document = open("chrome/en-paragraphs");
    let arabic = "تعد هذه الفقرة مثالا على نص عربي مضبوط الأطراف يلتف داخل عرضه المحدد على عدة أسطر، ويقرأ من جديد بخطه وحجمه ولونه كما كتب تماما دون تغيير.";
    let english =
        "This centred note is set in blue at twelve points and wraps over a few lines to show its alignment.";
    let text = |text: &str, y: f32, family, size, color, align| NewText {
        page: 0,
        x: 80.0,
        y,
        width: 220.0,
        text: text.into(),
        family,
        size,
        color,
        align,
        direction: BaseDirection::Auto,
    };
    let red = [0.72, 0.11, 0.11];
    let blue = [0.05, 0.28, 0.63];
    let (with_arabic, _, _) = document
        .with_new_text(&text(arabic, 420.0, FontFamily::Naskh, 14.0, red, Align::Justify))
        .expect("adds");
    let (both, _, _) = with_arabic
        .with_new_text(&text(english, 600.0, FontFamily::Sans, 12.0, blue, Align::Center))
        .expect("adds");
    let analysis = both.edit_blocks(0).expect("analyses");
    let find = |needle: &str| {
        analysis
            .blocks
            .iter()
            .find(|b| words(&b.text).contains(needle))
            .unwrap_or_else(|| panic!("no block with {needle}"))
    };

    let block = find("مضبوط الأطراف");
    assert_eq!(words(&block.text), words(arabic));
    assert_eq!(block.direction, Direction::Rtl);
    assert_eq!((block.align, block.align_seen), (BlockAlign::Justified, true));
    assert!(block.lines.len() >= 3);
    assert!(block.written_here && block.untraced == 0 && !block.shared);
    let style = &block.runs[0].style;
    assert_eq!(style.font, "Amiri-Regular");
    assert!((style.size - 14.0).abs() < 0.05, "{}", style.size);
    assert!(style.color.iter().zip(red).all(|(a, b)| (a - b).abs() < 0.01), "{:?}", style.color);
    let pitch = block.line_pitch.expect("several lines");
    assert!(pitch > 14.0 && pitch < 32.0, "{pitch}");

    let block = find("centred note");
    assert_eq!((block.align, block.direction), (BlockAlign::Center, Direction::Ltr));
    assert_eq!(block.runs[0].style.font, "IBMPlexSans");
    assert!(block.runs[0].style.color.iter().zip(blue).all(|(a, b)| (a - b).abs() < 0.01));
    assert!(block.written_here);

    // The page's own text is no text of ours.
    assert!(!find("Quarterly Report").written_here);
}

#[test]
fn fixture_fonts_sizes_and_directions_read_back() {
    let english = open("coretext/en-paragraphs").edit_blocks(0).expect("analyses");
    let heading = &english.blocks[0];
    assert_eq!(words(&heading.text), "Quarterly Report");
    assert_eq!(heading.runs[0].style.font, "IBMPlexSans-Regular");
    assert!((heading.runs[0].style.size - 20.0).abs() < 0.5);
    assert!(english.blocks[1..].iter().all(|b| (b.runs[0].style.size - 12.0).abs() < 0.5));
    assert!(english.blocks.iter().all(|b| b.align == BlockAlign::Left && b.direction == Direction::Ltr));

    let arabic = open("coretext/ar-plain").edit_blocks(0).expect("analyses");
    assert!(arabic.blocks.iter().all(|b| b.direction == Direction::Rtl && b.align == BlockAlign::Right));
    assert!(arabic.blocks.iter().any(|b| b.align_seen), "a multi-line paragraph shows its alignment");
    assert!(arabic.blocks.iter().all(|b| b.runs.iter().all(|r| r.style.font == "NotoNaskhArabic-Regular")));
}

#[test]
fn an_encrypted_files_text_is_analysed_and_removed() {
    let manifest = manifest();
    let fixture = manifest.fixtures.iter().find(|f| f.name() == "qpdf/encrypted").expect("fixture");
    let document = open("qpdf/encrypted");
    let analysis = document.edit_blocks(0).expect("analyses");
    assert!(analysis.blocks.iter().all(|b| b.untraced == 0));
    let before = document.render_page(0, 1.0).expect("renders");
    let edited = remove_and_check(&document, &before, "encrypted", 1, &analysis.blocks, &analysis.ops);
    let path = std::env::temp_dir().join(format!("safha-analysis-{}.pdf", std::process::id()));
    std::fs::write(&path, edited.bytes().as_ref()).expect("write");
    let output = Command::new("qpdf")
        .arg(format!("--password={}", fixture.password.as_deref().unwrap_or("")))
        .arg("--check")
        .arg(&path)
        .output()
        .expect("qpdf runs (install qpdf)");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(output.status.code(), Some(0 | 3)), "{}", String::from_utf8_lossy(&output.stdout));
    let read = words(&edited.page_text(0).expect("text").text());
    assert!(!read.contains(&words(&analysis.blocks[1].text)));
    assert!(read.contains(&words(&analysis.blocks[2].text)));
}
