//! M6 (ADR 0050) on real files: headers, footers, and page numbers in English
//! and Arabic (Western and Arabic-Indic digits, right-to-left placement),
//! watermarks (text and image, opacity, rotation, behind the page), page
//! export as PNG and JPEG, text and Markdown export in logical order, and
//! compression (unused objects, object streams, image downsampling). Every
//! written file passes `qpdf --check` and is read back by PDFium.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use safha_core::compress::CompressOptions;
use safha_core::edit::{EditHistory, PageEdit};
use safha_core::export::{ImageFormat, TextFormat};
use safha_core::fonts::FontFamily;
use safha_core::stamp::{Band, Digits, Edge, Mark, Stamp, StampKind, Watermark};
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
    let bytes = std::fs::read(fixture.file()).expect("fixture readable");
    engine().open_bytes(bytes, fixture.password.as_deref()).expect("opens")
}

fn qpdf_check(bytes: &[u8], name: &str) {
    qpdf_check_with(bytes, name, None);
}

fn qpdf_check_with(bytes: &[u8], name: &str, password: Option<&str>) {
    let dir = std::env::temp_dir().join(format!("safha-m6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path: PathBuf = dir.join(name);
    std::fs::write(&path, bytes).expect("write");
    let output = Command::new("qpdf")
        .arg(format!("--password={}", password.unwrap_or("")))
        .arg("--check")
        .arg(&path)
        .output()
        .expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check {name} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn footer(center: &str) -> Band {
    Band {
        edge: Edge::Bottom,
        start: String::new(),
        center: center.into(),
        end: String::new(),
        rtl: false,
        family: FontFamily::Sans,
        size: 10.0,
        color: [0.0, 0.0, 0.0],
        margin: 30.0,
        digits: Digits::Western,
        first_number: 1,
    }
}

fn stamp(document: &Document, pages: Vec<usize>, kind: StampKind) -> Document {
    document.with_stamp(&Stamp { pages, kind }).unwrap_or_else(|e| panic!("{e}")).0
}

/// Dark pixels per displayed region (x0, y0, x1, y1), at 1 px per point.
fn dark_in(bitmap: &Bitmap, (x0, y0, x1, y1): (f32, f32, f32, f32)) -> usize {
    let mut count = 0;
    for y in (y0.max(0.0) as u32)..(y1 as u32).min(bitmap.height()) {
        for x in (x0.max(0.0) as u32)..(x1 as u32).min(bitmap.width()) {
            let pixel = bitmap.pixel(x, y).expect("pixel");
            if pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128 {
                count += 1;
            }
        }
    }
    count
}

/// The displayed boxes of a page's lines that contain `needle`.
fn line_boxes(document: &Document, page: usize, needle: &str) -> Vec<safha_core::view::ViewRect> {
    let (text, view) = document.page_text_view(page).expect("text");
    text.blocks
        .iter()
        .flat_map(|block| &block.lines)
        .filter(|line| normalize(&line.text).contains(needle))
        .map(|line| view.rect(line.bounds))
        .collect()
}

#[test]
fn page_numbers_in_english_land_in_the_footer_and_read_back() {
    let document = open("qpdf/rotated");
    let count = document.page_count();
    let pages: Vec<usize> = (0..count).collect();
    let stamped = stamp(&document, pages, StampKind::Band(footer("Page {page} of {pages}")));
    // The unturned page reads its number back, centred at the bottom.
    let expected = format!("Page 1 of {count}");
    let text = normalize(&stamped.page_text(0).expect("text").text());
    assert!(text.contains(&expected), "{text}");
    let size = stamped.page_size(0).expect("size");
    let boxes = line_boxes(&stamped, 0, &expected);
    let rect = boxes.first().expect("a footer line");
    assert!(rect.y > size.height - 60.0 && rect.width > rect.height, "{rect:?}");
    assert!((rect.x + rect.width / 2.0 - size.width / 2.0).abs() < 3.0, "centred: {rect:?}");
    // Turned pages: the number stands upright at the bottom as displayed
    // (the text extractor reads lines in page space, so it is checked by ink).
    for page in 0..count {
        let size = stamped.page_size(page).expect("size");
        let before = document.render_page(page, 1.0).expect("renders");
        let after = stamped.render_page(page, 1.0).expect("renders");
        let band = (size.width / 2.0 - 45.0, size.height - 45.0, size.width / 2.0 + 45.0, size.height - 25.0);
        let whole = (0.0, 0.0, size.width, size.height);
        let added = dark_in(&after, band) - dark_in(&before, band);
        assert!(added > 40, "page {page}: {added} dark pixels in the footer");
        assert_eq!(dark_in(&after, whole) - dark_in(&before, whole), added, "page {page}: ink elsewhere");
        // Wider than tall: upright, not turned with the page.
        let (mut left, mut right, mut top, mut bottom) = (u32::MAX, 0, u32::MAX, 0);
        for y in (size.height - 45.0) as u32..(size.height - 25.0) as u32 {
            for x in 0..after.width() {
                if after.pixel(x, y) != before.pixel(x, y) {
                    (left, right, top, bottom) = (left.min(x), right.max(x), top.min(y), bottom.max(y));
                }
            }
        }
        assert!(right - left > 3 * (bottom - top), "page {page}: {left}..{right} x {top}..{bottom}");
    }
    qpdf_check(&stamped.bytes(), "numbers.pdf");
}

#[test]
fn arabic_page_numbers_take_arabic_indic_digits_and_the_start_is_on_the_right() {
    let document = open("chrome/ar-plain");
    let band = Band {
        edge: Edge::Top,
        start: "تقرير المشروع".into(),
        center: String::new(),
        end: "صفحة {page} من {pages}".into(),
        rtl: true,
        digits: Digits::ArabicIndic,
        first_number: 5,
        family: FontFamily::Naskh,
        ..footer("")
    };
    let stamped = stamp(&document, vec![0], StampKind::Band(band));
    let text = normalize(&stamped.page_text(0).expect("text").text());
    assert!(text.contains("صفحة ٥ من ٥"), "logical order, Arabic-Indic digits: {text}");
    assert!(text.contains("تقرير المشروع"), "{text}");
    let width = stamped.page_size(0).expect("size").width;
    let start = line_boxes(&stamped, 0, "تقرير المشروع");
    let end = line_boxes(&stamped, 0, "صفحة ٥ من ٥");
    let (start, end) = (start.first().expect("start slot"), end.first().expect("end slot"));
    assert!(start.x > width / 2.0, "the start slot is on the right: {start:?}");
    assert!(end.x + end.width < width / 2.0, "the end slot is on the left: {end:?}");
    assert!(start.y < 60.0 && end.y < 60.0, "in the header: {start:?} {end:?}");
    qpdf_check(&stamped.bytes(), "arabic-header.pdf");
}

#[test]
fn a_band_is_one_undo_step_and_can_be_stamped_twice() {
    let document = open("qpdf/rtl-book-objstm");
    let mut history = EditHistory::new(document.bytes());
    let (once, update) = document
        .with_stamp(&Stamp { pages: vec![0, 2], kind: StampKind::Band(footer("{page}")) })
        .expect("ok");
    history.push_content(update);
    let (twice, update) = once
        .with_stamp(&Stamp {
            pages: vec![1],
            kind: StampKind::Band(Band { edge: Edge::Top, ..footer("Draft") }),
        })
        .expect("ok");
    history.push_content(update);
    assert!(normalize(&twice.page_text(2).expect("text").text()).ends_with('2'), "second stamped page is 2");
    assert!(normalize(&twice.page_text(1).expect("text").text()).contains("Draft"));
    qpdf_check(&twice.bytes(), "twice.pdf");
    history.undo();
    assert_eq!(history.undo().as_deref(), Some(&*document.bytes()), "two undos restore the file");
}

#[test]
fn text_watermarks_are_translucent_rotated_and_can_go_behind() {
    let document = open("chrome/en-paragraphs");
    let size = document.page_size(0).expect("size");
    let watermark = |behind| Watermark {
        mark: Mark::Text {
            text: "سري CONFIDENTIAL".into(),
            family: FontFamily::Sans,
            size: 60.0,
            color: [1.0, 0.0, 0.0],
        },
        opacity: 0.4,
        rotation: 45.0,
        behind,
    };
    let over = stamp(&document, vec![0], StampKind::Watermark(watermark(false)));
    let bitmap = over.render_page(0, 1.0).expect("renders");
    // Light red: red full, green and blue raised by the 60 % of white showing through.
    let reddish = |p: [u8; 4]| p[0] > 230 && p[1] > 120 && p[1] < 190 && p[2] > 120 && p[2] < 190;
    let mut found = 0;
    let (mut top, mut bottom, mut left, mut right) = (u32::MAX, 0, u32::MAX, 0);
    for y in 0..bitmap.height() {
        for x in 0..bitmap.width() {
            if reddish(bitmap.pixel(x, y).expect("pixel")) {
                found += 1;
                (top, bottom, left, right) = (top.min(y), bottom.max(y), left.min(x), right.max(x));
            }
        }
    }
    assert!(found > 2_000, "translucent red ink: {found}");
    // Rotated 45°: as tall as it is wide, around the centre.
    let (w, h) = ((right - left) as f32, (bottom - top) as f32);
    assert!((w - h).abs() < w * 0.25, "a diagonal: {w} x {h}");
    let centre = ((left + right) as f32 / 2.0, (top + bottom) as f32 / 2.0);
    assert!((centre.0 - size.width / 2.0).abs() < 10.0 && (centre.1 - size.height / 2.0).abs() < 10.0);
    // Drawn as shapes: the page's text is what it was.
    assert_eq!(over.page_text(0).expect("text").text(), document.page_text(0).expect("text").text());

    // Behind: the page's own black text stays black where they cross.
    let behind = stamp(&document, vec![0], StampKind::Watermark(watermark(true)));
    let plain = document.render_page(0, 1.0).expect("renders");
    let under = behind.render_page(0, 1.0).expect("renders");
    let all = (0.0, 0.0, size.width, size.height);
    assert_eq!(dark_in(&plain, all), dark_in(&under, all), "nothing dark is covered");
    qpdf_check(&behind.bytes(), "watermark-behind.pdf");
}

fn png_bytes(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&pixel(x, y));
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");
    writer.write_image_data(&data).expect("data");
    writer.finish().expect("finish");
    out
}

#[test]
fn image_watermarks_take_png_with_alpha_and_our_own_jpeg() {
    let document = open("chrome/ar-sans");
    let size = document.page_size(0).expect("size");
    // A blue square with a transparent left half.
    let png = png_bytes(40, 40, |x, _| if x < 20 { [0, 0, 0, 0] } else { [0, 0, 255, 255] });
    let image = |data: Vec<u8>| Watermark {
        mark: Mark::Image { data, width: 0.5 },
        opacity: 1.0,
        rotation: 0.0,
        behind: false,
    };
    let stamped = stamp(&document, vec![0], StampKind::Watermark(image(png)));
    let bitmap = stamped.render_page(0, 1.0).expect("renders");
    let (cx, cy) = (size.width / 2.0, size.height / 2.0);
    let at = |x: f32, y: f32| bitmap.pixel(x as u32, y as u32).expect("pixel");
    assert_eq!(&at(cx + size.width * 0.15, cy)[..3], &[0, 0, 255], "opaque half is blue");
    assert_ne!(&at(cx - size.width * 0.15, cy)[..3], &[0, 0, 255], "transparent half shows the page");
    qpdf_check(&stamped.bytes(), "png-watermark.pdf");

    // JPEG from our own encoder, decoded by PDFium's: the colours survive.
    let rgb: Vec<u8> =
        (0..64 * 64).flat_map(|i| if i % 64 < 32 { [200, 30, 30] } else { [30, 160, 40] }).collect();
    let jpeg = safha_core::jpeg::encode_rgb(&rgb, 64, 64, 90).expect("jpeg");
    let stamped = stamp(&document, vec![0], StampKind::Watermark(image(jpeg)));
    let bitmap = stamped.render_page(0, 1.0).expect("renders");
    let left = bitmap.pixel((cx - size.width * 0.12) as u32, cy as u32).expect("pixel");
    let right = bitmap.pixel((cx + size.width * 0.12) as u32, cy as u32).expect("pixel");
    let close = |p: [u8; 4], q: [u8; 3]| (0..3).all(|i| (i32::from(p[i]) - i32::from(q[i])).abs() < 12);
    assert!(close(left, [200, 30, 30]), "{left:?}");
    assert!(close(right, [30, 160, 40]), "{right:?}");
}

#[test]
fn image_watermarks_in_encrypted_files_are_encrypted_with_them() {
    let document = open("qpdf/encrypted");
    let size = document.page_size(0).expect("size");
    let png = png_bytes(40, 40, |x, _| if x < 20 { [0, 0, 0, 0] } else { [0, 0, 255, 255] });
    let watermark =
        Watermark { mark: Mark::Image { data: png, width: 0.5 }, opacity: 1.0, rotation: 0.0, behind: false };
    let stamped = stamp(&document, vec![0], StampKind::Watermark(watermark));
    // PDFium decrypts the image and its soft mask: written in the clear, they would not decode.
    let bitmap = stamped.render_page(0, 1.0).expect("renders");
    let (cx, cy) = (size.width / 2.0, size.height / 2.0);
    let at = |x: f32, y: f32| bitmap.pixel(x as u32, y as u32).expect("pixel");
    assert_eq!(&at(cx + size.width * 0.15, cy)[..3], &[0, 0, 255], "opaque half is blue");
    assert_ne!(&at(cx - size.width * 0.15, cy)[..3], &[0, 0, 255], "transparent half shows the page");
    qpdf_check_with(&stamped.bytes(), "encrypted-png-watermark.pdf", Some("safha-test"));
}

#[test]
fn pages_export_as_png_and_jpeg_at_the_chosen_resolution() {
    let document = open("chrome/mixed-inline");
    let size = document.page_size(0).expect("size");
    let png = document.export_page_image(0, 144, ImageFormat::Png).expect("png");
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let reader = decoder.read_info().expect("a PNG");
    let info = reader.info();
    assert_eq!(
        (info.width, info.height),
        ((size.width * 2.0).ceil() as u32, (size.height * 2.0).ceil() as u32)
    );
    let jpeg = document.export_page_image(0, 72, ImageFormat::Jpeg { quality: 85 }).expect("jpeg");
    assert_eq!(
        safha_core::jpeg::frame(&jpeg),
        Some((size.width.ceil() as u32, size.height.ceil() as u32, 3))
    );
    assert!(document.export_page_image(0, 2_000, ImageFormat::Png).is_err());
}

#[test]
fn text_export_is_in_logical_order_in_english_and_arabic() {
    for name in ["chrome/ar-plain", "chrome/ar-vocalized", "chrome/ar-sans", "chrome/en-paragraphs"] {
        let manifest = manifest();
        let fixture = manifest.fixtures.iter().find(|f| f.name() == name).expect("fixture");
        let document = open(name);
        let plain = normalize(&document.export_text(&[0], TextFormat::Plain).expect("text"));
        for block in &fixture.expected {
            assert!(plain.contains(&normalize(&block.text)), "{name}: {} missing from\n{plain}", block.text);
        }
        let markdown = document.export_text(&[0], TextFormat::Markdown).expect("markdown");
        assert!(markdown.starts_with("<!-- page 1 -->"), "{name}");
        assert!(!markdown.contains("<div dir="), "{name}: one direction, no wrappers:\n{markdown}");
    }
    // Mixed-direction lines come out as the page's text reads (the extractor's own tests cover them).
    let mixed = open("chrome/mixed-inline");
    let plain = mixed.export_text(&[0], TextFormat::Plain).expect("text");
    assert_eq!(plain.trim_end(), mixed.page_text(0).expect("text").text().trim_end());

    // Two pages of different direction: the second page's blocks are marked.
    let english = open("chrome/en-paragraphs");
    let arabic = open("chrome/ar-plain");
    let (both, _) = english.with_pages_from(&[&arabic], 1).expect("combined");
    let markdown = both.export_text(&[0, 1], TextFormat::Markdown).expect("markdown");
    assert!(markdown.contains("<div dir=\"rtl\">\n\nتقرير عن حالة المشروع\n\n</div>"), "{markdown}");
    assert!(markdown.contains("\n---\n\n<!-- page 2 -->"));
    let plain = both.export_text(&[0, 1], TextFormat::Plain).expect("text");
    assert!(plain.contains("\n\u{000C}\n"), "a page break between pages");
}

#[test]
fn compression_keeps_every_page_and_its_text() {
    let manifest = manifest();
    for fixture in &manifest.fixtures {
        if fixture.damaged || fixture.password.is_some() || fixture.pages.is_some_and(|p| p > 10) {
            continue;
        }
        let name = fixture.name();
        let document = open(&name);
        let (bytes, report) =
            document.compressed(CompressOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(report.after, bytes.len() as u64);
        let compressed = engine().open_bytes(bytes.clone(), None).expect("opens");
        assert_eq!(compressed.page_count(), document.page_count(), "{name}");
        for page in 0..document.page_count() {
            assert_eq!(
                normalize(&compressed.page_text(page).expect("text").text()),
                normalize(&document.page_text(page).expect("text").text()),
                "{name} page {page}"
            );
        }
        qpdf_check(&bytes, &format!("compressed-{}.pdf", name.replace('/', "-")));
    }
}

#[test]
fn compression_drops_what_edits_left_behind_and_reduces_large_images() {
    let document = open("chrome/en-paragraphs");
    // A 1,600 px photo-like image drawn 4 inches wide: 400 ppi.
    let png = png_bytes(1600, 1600, |x, y| [(x / 7) as u8, (y / 7) as u8, ((x + y) / 13) as u8, 255]);
    let stamped = stamp(
        &document,
        vec![0],
        StampKind::Watermark(Watermark {
            mark: Mark::Image { data: png, width: 288.0 / document.page_size(0).expect("size").width },
            opacity: 1.0,
            rotation: 0.0,
            behind: true,
        }),
    );
    let (edited, _) = stamped
        .with_page_edit(&PageEdit::InsertBlank { at: 1, width: 300.0, height: 300.0 })
        .expect("insert");
    let (edited, _) = edited.with_page_edit(&PageEdit::Delete { pages: vec![1] }).expect("delete");
    let before = edited.render_page(0, 1.0).expect("renders");

    let lossless = CompressOptions { image_dpi: Some(100), jpeg_quality: None, object_streams: true };
    let (bytes, report) = edited.compressed(lossless).expect("compressed");
    assert_eq!(report.images_reduced, 1, "{report:?}");
    assert!(report.objects_removed > 0, "{report:?}");
    assert!(report.after * 2 < report.before, "{report:?}");
    let compressed = engine().open_bytes(bytes.clone(), None).expect("opens");
    let after = compressed.render_page(0, 1.0).expect("renders");
    // At one pixel per point the page looks the same (within averaging).
    let (mut differ, mut total) = (0, 0);
    for y in (0..after.height()).step_by(4) {
        for x in (0..after.width()).step_by(4) {
            total += 1;
            let (p, q) = (before.pixel(x, y).expect("pixel"), after.pixel(x, y).expect("pixel"));
            if (0..3).any(|i| (i32::from(p[i]) - i32::from(q[i])).abs() > 24) {
                differ += 1;
            }
        }
    }
    assert!(differ * 50 < total, "{differ} of {total} sampled pixels changed");
    qpdf_check(&bytes, "reduced.pdf");

    let jpeg = CompressOptions { jpeg_quality: Some(70), ..lossless };
    let (bytes, report) = edited.compressed(jpeg).expect("compressed");
    assert_eq!(report.images_reduced, 1);
    let compressed = engine().open_bytes(bytes.clone(), None).expect("opens");
    assert_eq!(compressed.page_count(), 1);
    qpdf_check(&bytes, "reduced-jpeg.pdf");

    // Without object streams: a classic table.
    let classic = CompressOptions { image_dpi: None, jpeg_quality: None, object_streams: false };
    let (bytes, report) = edited.compressed(classic).expect("compressed");
    assert_eq!(report.images_reduced, 0);
    assert!(bytes.windows(5).any(|w| w == b"\nxref"));
    qpdf_check(&bytes, "classic.pdf");
    let _ = Arc::clone(&edited.bytes());
}

#[test]
fn encrypted_files_are_not_compressed() {
    let document = open("qpdf/encrypted");
    assert!(document.compressed(CompressOptions::default()).is_err());
}
