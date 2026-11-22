//! True redaction and sanitizing (ADR 0042), on hand-built PDFs (English) and
//! on the generated corpus (Arabic and English, two producers). Every test
//! reads the result back the ways a leak would show: the text PDFium
//! extracts, the decompressed bytes of the saved file (UTF-8 and UTF-16),
//! and a render.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use common::manifest;
use safha_core::edit::EditHistory;
use safha_core::redact::{Mark, Pattern, Preset, SanitizeOptions, find_matches};
use safha_core::save::save;
use safha_core::search::{DocumentIndex, SearchOptions};
use safha_core::text::Rect;
use safha_core::{Bitmap, Document, Engine, Error, PdfiumSource, RevisionStep};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-redact-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

/// A classic-xref PDF from the bodies of objects 1, 2, 3, …; object 1 is the
/// catalog.
fn pdf(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

/// A page of English text, an image, a grey rectangle, and two annotations,
/// with personal data in its metadata.
fn sample() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 20 150 Td (Name: Jane Roe) Tj ET\n\
BT /F1 12 Tf 20 130 Td (Account 4111111111111111 ok) Tj ET\n\
BT /F1 12 Tf 20 110 Td [(Keep) -500 (this)] TJ ET\n\
q 0.8 g 20 20 100 40 re f Q\n\
q 100 0 0 50 150 20 cm /Im1 Do Q\n";
    // 8 x 8 RGB, all red.
    let pixels: Vec<u8> = (0..64).flat_map(|_| [255u8, 0, 0]).collect();
    pdf(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R \
/Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R >> >> /Annots [7 0 R 8 0 R] >>"
                .to_vec(),
            stream("", content),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            stream(
                "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8",
                &pixels,
            ),
            b"<< /Type /Annot /Subtype /Text /Rect [60 150 80 170] /Contents (Jane Roe called back) >>"
                .to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [200 150 250 170] /Border [0 0 0] >>".to_vec(),
            b"<< /Author (Jane Roe) /Subject (Account 4111111111111111) /Title (Report) >>".to_vec(),
        ],
        "/Info 9 0 R",
    )
}

fn open(bytes: Vec<u8>) -> Document {
    engine().open_bytes(bytes, None).expect("opens")
}

fn text_of(document: &Document, page: usize) -> String {
    document.page_text(page).expect("text").text()
}

fn marks_for(document: &Document, pattern: &Pattern) -> Vec<Mark> {
    let index = DocumentIndex::build(document, || false).expect("builds").expect("not cancelled");
    let (hits, _) = find_matches(&index, pattern, 1000).expect("compiles");
    hits.into_iter()
        .flat_map(|hit| {
            let text = hit.text.clone();
            hit.rects.into_iter().map(move |rect| Mark { page: hit.page, rect, text: Some(text.clone()) })
        })
        .collect()
}

fn literal(text: &str) -> Pattern {
    Pattern::Text { text: text.to_string(), options: SearchOptions::default() }
}

/// The document's decompressed objects, by qpdf, for scanning the bytes a
/// saved file holds.
fn decompressed(bytes: &[u8]) -> Vec<u8> {
    let input = scratch(&format!("in-{}.pdf", bytes.len()));
    let output = scratch(&format!("out-{}.pdf", bytes.len()));
    std::fs::write(&input, bytes).expect("write");
    let status = Command::new("qpdf")
        .args(["--qdf", "--object-streams=disable", "--stream-data=uncompress"])
        .arg(&input)
        .arg(&output)
        .status()
        .expect("qpdf runs (install qpdf)");
    assert!(matches!(status.code(), Some(0 | 3)), "qpdf failed");
    std::fs::read(&output).expect("read")
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

fn holds(bytes: &[u8], text: &str) -> bool {
    let needle = text.as_bytes();
    let wide = utf16(text);
    bytes.windows(needle.len()).any(|w| w == needle)
        || bytes.windows(wide.len()).any(|w| w == wide.as_slice())
}

/// The saved file (rewritten in full, as a save does), decompressed.
fn saved_and_decompressed(document: &Document, name: &str) -> Vec<u8> {
    let target = scratch(name);
    save(document, &target).expect("saves");
    decompressed(&std::fs::read(&target).expect("read"))
}

const SCALE: f32 = 3.0;

fn render(document: &Document, page: usize) -> Bitmap {
    document.render_page(page, SCALE).expect("renders")
}

fn dark([r, g, b, _]: [u8; 4]) -> bool {
    r < 40 && g < 40 && b < 40
}

/// Whether every pixel inside `rect` (page space) of `bitmap` is dark.
fn all_dark(document: &Document, page: usize, bitmap: &Bitmap, rect: Rect) -> bool {
    let view = document.view_transform(page).expect("view").rect(rect);
    let (x0, y0) = ((view.x * SCALE).ceil() as u32 + 1, (view.y * SCALE).ceil() as u32 + 1);
    let (x1, y1) = (((view.x + view.width) * SCALE) as u32 - 1, ((view.y + view.height) * SCALE) as u32 - 1);
    (y0..y1).all(|y| (x0..x1).all(|x| bitmap.pixel(x, y).is_some_and(dark)))
}

fn changed_pixels_outside(
    document: &Document,
    page: usize,
    before: &Bitmap,
    after: &Bitmap,
    rects: &[Rect],
) -> usize {
    let view = document.view_transform(page).expect("view");
    let views: Vec<_> = rects.iter().map(|r| view.rect(*r)).collect();
    let mut changed = 0;
    for y in 0..after.height() {
        for x in 0..after.width() {
            let (vx, vy) = (x as f32 / SCALE, y as f32 / SCALE);
            if views.iter().any(|v| {
                vx >= v.x - 2.0 && vx <= v.x + v.width + 2.0 && vy >= v.y - 2.0 && vy <= v.y + v.height + 2.0
            }) {
                continue;
            }
            if before.pixel(x, y) != after.pixel(x, y) {
                changed += 1;
            }
        }
    }
    changed
}

// ---------------------------------------------------------------- English

#[test]
fn text_under_a_mark_is_removed_and_its_neighbours_stay_put() {
    let document = open(sample());
    assert!(text_of(&document, 0).contains("Jane Roe"));
    let before = render(&document, 0);
    let marks = marks_for(&document, &literal("Jane Roe"));
    assert_eq!(marks.len(), 1, "one run of glyphs: {marks:?}");

    let done = document.with_redactions(&marks).expect("redacts");
    let redacted = done.document;
    let text = text_of(&redacted, 0);
    assert!(!text.contains("Jane") && !text.contains("Roe"), "{text}");
    assert!(text.contains("Name:"), "text before the mark stays: {text}");
    assert!(text.contains("Account 4111111111111111 ok"), "other lines stay: {text}");
    assert!(text.contains("Keep") && text.contains("this"), "{text}");
    assert_eq!(done.report.counts.glyphs, 8, "eight glyphs cut: {:?}", done.report);

    // Black where the text was; nothing else on the page moved.
    let after = render(&redacted, 0);
    assert!(all_dark(&redacted, 0, &after, marks[0].rect), "the area is black");
    // (The note's icon goes with the note, which lay under the mark.)
    let note = Rect { left: 60.0, bottom: 150.0, right: 80.0, top: 170.0 };
    let outside = changed_pixels_outside(&redacted, 0, &before, &after, &[marks[0].rect, note]);
    assert_eq!(outside, 0, "{outside} pixels changed outside the mark");

    // The saved file holds none of it: not the text, not the metadata, not the note.
    let saved = saved_and_decompressed(&redacted, "text.pdf");
    for gone in ["Jane Roe", "Jane", "called back"] {
        assert!(!holds(&saved, gone), "{gone} survived in the saved file");
    }
    assert!(holds(&saved, "4111111111111111"), "unmarked text is still there");
    assert!(holds(&saved, "[redacted]"), "metadata was scrubbed, not dropped");
    assert!(holds(&saved, "Report"), "unrelated metadata stays");
    assert_eq!(done.report.annotations, 1, "the note under the mark");
    assert!(done.report.strings >= 1);
}

#[test]
fn a_redaction_is_one_update_and_undo_brings_everything_back() {
    let original = sample();
    let document = open(original.clone());
    let marks = marks_for(&document, &literal("Jane Roe"));
    let done = document.with_redactions(&marks).expect("redacts");
    let RevisionStep::Update(update) = &done.step else { panic!("an unsigned file gets an update") };

    let mut history = EditHistory::new(Arc::from(original.as_slice()));
    history.push(update.clone());
    assert_eq!(&*history.current(), &*done.document.bytes());
    let undone = history.undo().expect("undoes");
    assert_eq!(&*undone, original.as_slice());
    let back = document.revision(undone).expect("reopens");
    assert!(text_of(&back, 0).contains("Jane Roe"), "undo brings the text back");
    let redone = history.redo().expect("redoes");
    assert!(!text_of(&document.revision(redone).expect("reopens"), 0).contains("Jane"));
}

#[test]
fn a_card_number_preset_finds_and_removes_only_valid_numbers() {
    let document = open(sample());
    let marks = marks_for(&document, &Pattern::Preset(Preset::CardNumber));
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].text.as_deref(), Some("4111111111111111"));
    let redacted = document.with_redactions(&marks).expect("redacts").document;
    let text = text_of(&redacted, 0);
    assert!(!text.contains("4111"), "{text}");
    assert!(text.contains("Account") && text.contains("ok"), "{text}");
}

#[test]
fn image_pixels_under_a_mark_are_overwritten_in_a_copy_and_the_rest_stays() {
    let document = open(sample());
    // The image covers x 150..250, y 20..70; mark its left third.
    let mark =
        Mark { page: 0, rect: Rect { left: 150.0, bottom: 20.0, right: 183.0, top: 70.0 }, text: None };
    let done = document.with_redactions(&[mark]).expect("redacts");
    assert_eq!(done.report.counts.images_blanked, 1, "{:?}", done.report);
    let RevisionStep::Update(update) = &done.step else { panic!("update") };

    // The new image object: Flate, 8 × 8 × 3, with the left pixels now black.
    let at = update.windows(8).position(|w| w == b"/Width 8").expect("a new 8-wide image in the update");
    let from = update[at..].windows(7).position(|w| w == b"stream\n").expect("its data") + at + 7;
    let to = update[from..].windows(10).position(|w| w == b"\nendstream").expect("its end") + from;
    let pixels = miniz_oxide::inflate::decompress_to_vec_zlib(&update[from..to]).expect("inflates");
    assert_eq!(pixels.len(), 192);
    let red = pixels.chunks(3).filter(|p| *p == [255, 0, 0]).count();
    let black = pixels.chunks(3).filter(|p| *p == [0, 0, 0]).count();
    assert_eq!(red + black, 64);
    // 33 of 100 points of width: pixel centres at 6.25, 18.75, 31.25 → 3 columns of 8 rows.
    assert_eq!(black, 24, "three columns of eight pixels");

    // The original image object is no longer used by this page.
    let saved = saved_and_decompressed(&done.document, "image.pdf");
    let red_run: Vec<u8> = [255u8, 0, 0].repeat(64);
    assert!(!saved.windows(red_run.len()).any(|w| w == red_run.as_slice()), "the whole red image survived");
}

#[test]
fn a_filled_rectangle_is_cut_around_the_mark() {
    let document = open(sample());
    let before = render(&document, 0);
    let mark = Mark { page: 0, rect: Rect { left: 50.0, bottom: 30.0, right: 70.0, top: 50.0 }, text: None };
    let done = document.with_redactions(std::slice::from_ref(&mark)).expect("redacts");
    assert_eq!(done.report.counts.paths, 1);
    let after = render(&done.document, 0);
    assert!(all_dark(&done.document, 0, &after, mark.rect));
    // The grey rectangle is intact on both sides of the hole.
    let outside = changed_pixels_outside(&done.document, 0, &before, &after, &[mark.rect]);
    assert_eq!(outside, 0, "{outside} pixels changed outside");
}

#[test]
fn marks_that_cover_nothing_still_draw_their_box_and_change_nothing_else() {
    let document = open(sample());
    let mark =
        Mark { page: 0, rect: Rect { left: 200.0, bottom: 100.0, right: 280.0, top: 120.0 }, text: None };
    let before = render(&document, 0);
    let done = document.with_redactions(std::slice::from_ref(&mark)).expect("redacts");
    assert_eq!(done.report.counts.glyphs, 0);
    let after = render(&done.document, 0);
    assert!(all_dark(&done.document, 0, &after, mark.rect));
    assert_eq!(changed_pixels_outside(&done.document, 0, &before, &after, &[mark.rect]), 0);
}

#[test]
fn bad_marks_are_refused() {
    let document = open(sample());
    let rect = Rect { left: 0.0, bottom: 0.0, right: 10.0, top: 10.0 };
    assert!(matches!(document.with_redactions(&[]), Err(Error::InvalidEdit(_))));
    assert!(matches!(
        document.with_redactions(&[Mark { page: 3, rect, text: None }]),
        Err(Error::PageOutOfRange { .. })
    ));
    let empty = Rect { left: 5.0, bottom: 5.0, right: 5.0, top: 9.0 };
    assert!(matches!(
        document.with_redactions(&[Mark { page: 0, rect: empty, text: None }]),
        Err(Error::InvalidEdit(_))
    ));
    let nan = Rect { left: f32::NAN, bottom: 0.0, right: 1.0, top: 1.0 };
    assert!(matches!(
        document.with_redactions(&[Mark { page: 0, rect: nan, text: None }]),
        Err(Error::InvalidEdit(_))
    ));
}

#[test]
fn encrypted_files_are_refused_rather_than_half_redacted() {
    let manifest = manifest();
    let fixture = manifest.fixtures.iter().find(|f| f.name() == "qpdf/encrypted").expect("fixture");
    let document = engine().open_file(&fixture.file(), fixture.password.as_deref()).expect("opens");
    let mark = Mark { page: 0, rect: Rect { left: 0.0, bottom: 0.0, right: 100.0, top: 100.0 }, text: None };
    assert!(matches!(document.with_redactions(&[mark]), Err(Error::EditUnsupported(_))));
}

// ---------------------------------------------------------------- Arabic and the corpus

fn corpus(name: &str) -> Option<Document> {
    let manifest = manifest();
    let fixture = manifest.fixture_or_unavailable(name)?;
    Some(engine().open_file(&fixture.file(), fixture.password.as_deref()).expect("opens"))
}

/// Redacts every hit of `pattern`, then checks the page text lost them and
/// kept `survivors`, and that the saved file holds no trace of them.
fn redact_and_check(name: &str, pattern: &Pattern, gone: &[&str], survivors: &[&str]) {
    let Some(document) = corpus(name) else { return };
    // Lines that hold nothing to remove must extract the same afterwards,
    // whatever the producer's text quality is.
    let untouched: Vec<String> = text_of(&document, 0)
        .lines()
        .filter(|line| !line.trim().is_empty() && !gone.iter().any(|g| line.contains(g)))
        .map(str::to_string)
        .collect();
    let marks = marks_for(&document, pattern);
    assert!(!marks.is_empty(), "{name}: nothing found for {pattern:?}");
    let before = render(&document, 0);
    let done = document.with_redactions(&marks).expect("redacts");
    let text = text_of(&done.document, 0);
    for text_gone in gone {
        assert!(!text.contains(text_gone), "{name}: {text_gone} still extracts:\n{text}");
    }
    // CoreText's Arabic does not extract to the right text even before
    // redaction, so only Chrome's output is held to the literal strings.
    let literal: &[&str] = if name.starts_with("chrome/") { survivors } else { &[] };
    for kept in literal.iter().map(|s| s.to_string()).chain(untouched) {
        assert!(text.contains(&kept), "{name}: {kept} was lost:\n{text}");
    }
    let after = render(&done.document, 0);
    for mark in &marks {
        assert!(all_dark(&done.document, 0, &after, mark.rect), "{name}: a marked area is not black");
    }
    let rects: Vec<Rect> = marks.iter().map(|m| m.rect).collect();
    let outside = changed_pixels_outside(&done.document, 0, &before, &after, &rects);
    assert!(outside < 40, "{name}: {outside} pixels changed outside the marks");

    let saved = saved_and_decompressed(&done.document, &format!("{name}.pdf").replace('/', "-"));
    for text_gone in gone {
        assert!(!holds(&saved, text_gone), "{name}: {text_gone} is still in the saved file");
    }
}

const PRODUCERS: [&str; 2] = ["chrome", "coretext"];

#[test]
fn arabic_indic_phone_numbers_are_found_and_removed() {
    for producer in PRODUCERS {
        redact_and_check(
            &format!("{producer}/personal-data"),
            &Pattern::Preset(Preset::Phone),
            &["٠٥٥ ١٢٣ ٤٥٦٧", "123 4567"],
            &[
                "هذا نص عادي لا يجب حذفه من الصفحة.",
                "This ordinary sentence must survive the redaction untouched.",
            ],
        );
    }
}

#[test]
fn western_digits_find_an_arabic_indic_id_and_remove_it() {
    for producer in PRODUCERS {
        redact_and_check(
            &format!("{producer}/personal-data"),
            &literal("1023456789"),
            &["١٠٢٣٤٥٦٧٨٩"],
            &["هذا نص عادي لا يجب حذفه من الصفحة.", "٠٥٥ ١٢٣ ٤٥٦٧"],
        );
    }
}

#[test]
fn an_arabic_word_is_redacted_and_the_rest_of_the_line_stays() {
    // Chrome only: CoreText's Arabic does not extract to searchable text.
    redact_and_check(
        "chrome/personal-data",
        &literal("الهوية"),
        &["الهوية"],
        &["رقم", "هذا نص عادي لا يجب حذفه من الصفحة."],
    );
}

#[test]
fn a_marked_area_removes_a_whole_arabic_line_in_both_producers() {
    for producer in PRODUCERS {
        let name = format!("{producer}/personal-data");
        let Some(document) = corpus(&name) else { continue };
        let before_text = text_of(&document, 0);
        let page = document.page_text(0).expect("text");
        // The block of Arabic body text (the third Arabic block), by position.
        let arabic: Vec<_> =
            page.blocks.iter().filter(|b| b.direction == safha_core::text::Direction::Rtl).collect();
        let target = arabic.last().expect("an Arabic block");
        let bounds = target.bounds;
        let target_text = target.lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n");
        let mark = Mark { page: 0, rect: bounds, text: None };
        let before = render(&document, 0);
        let done = document.with_redactions(&[mark]).expect("redacts");
        let after_text = text_of(&done.document, 0);
        assert!(!after_text.contains(&target_text), "{name}: the line survived:\n{after_text}");
        assert!(done.report.counts.glyphs > 10, "{name}: {:?}", done.report);
        // The other lines extract as before.
        for line in before_text.lines().filter(|l| !l.trim().is_empty() && !target_text.contains(l)) {
            assert!(after_text.contains(line), "{name}: {line} was lost");
        }
        let after = render(&done.document, 0);
        assert!(all_dark(&done.document, 0, &after, bounds));
        let outside = changed_pixels_outside(&done.document, 0, &before, &after, &[bounds]);
        assert!(outside < 40, "{name}: {outside} pixels changed outside the line");
    }
}

#[test]
fn e_mail_iban_and_id_presets_remove_their_matches_in_english_text() {
    for producer in PRODUCERS {
        let name = format!("{producer}/personal-data");
        redact_and_check(
            &name,
            &Pattern::Preset(Preset::Email),
            &["jane.roe@example.com"],
            &["Contact Jane Roe on", "This ordinary sentence must survive the redaction untouched."],
        );
        redact_and_check(
            &name,
            &Pattern::Preset(Preset::Iban),
            &["SA03 8000 0000 6080 1016 7519"],
            &["for the transfer."],
        );
        redact_and_check(&name, &Pattern::Preset(Preset::IdNumber), &["١٠٢٣٤٥٦٧٨٩"], &["الجوال"]);
    }
}

#[test]
fn the_preset_patterns_find_arabic_digit_numbers_in_arabic_lines() {
    let document = corpus("chrome/personal-data").expect("chrome fixtures exist on every machine");
    let index = DocumentIndex::build(&document, || false).expect("builds").expect("not cancelled");
    let (hits, truncated) = find_matches(&index, &Pattern::Preset(Preset::Phone), 100).expect("compiles");
    assert!(!truncated);
    let texts: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("٠٥٥ ١٢٣ ٤٥٦٧")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("123 4567")), "{texts:?}");
    let (ids, _) = find_matches(&index, &Pattern::Preset(Preset::IdNumber), 100).expect("compiles");
    assert!(ids.iter().any(|h| h.text == "١٠٢٣٤٥٦٧٨٩"), "{ids:?}");
    assert!(hits.iter().all(|h| h.page == 0 && !h.rects.is_empty()));
}

// ---------------------------------------------------------------- sanitize

fn dirty() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 20 150 Td (Visible text) Tj ET\n\
BT /F1 12 Tf 3 Tr 20 130 Td (Invisible OCR layer) Tj ET\n\
BT /F1 12 Tf 0 Tr 500 100 Td (Off the page text) Tj ET\n\
/OC /L1 BDC BT /F1 12 Tf 20 110 Td (Hidden layer text) Tj ET EMC\n\
BT /F1 12 Tf 20 90 Td (Also visible) Tj ET\n";
    pdf(
        &[
            b"<< /Type /Catalog /Pages 2 0 R /OpenAction 8 0 R /Metadata 9 0 R \
/Names << /EmbeddedFiles << /Names [(note.txt) 11 0 R] >> /JavaScript << /Names [(init) 8 0 R] >> >> \
/OCProperties << /OCGs [7 0 R] /D << /OFF [7 0 R] >> >> >>"
                .to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R \
/Resources << /Font << /F1 5 0 R >> /Properties << /L1 7 0 R >> >> /Annots [6 0 R 12 0 R] >>"
                .to_vec(),
            stream("", content),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            b"<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (A private comment) >>".to_vec(),
            b"<< /Type /OCG /Name (Secret layer) >>".to_vec(),
            b"<< /S /JavaScript /JS (app.alert\\(\"hello\"\\)) >>".to_vec(),
            stream(
                "/Type /Metadata /Subtype /XML",
                b"<x:xmpmeta><dc:creator>Jane Roe</dc:creator></x:xmpmeta>",
            ),
            b"<< /Author (Jane Roe) /Producer (Some Tool) >>".to_vec(),
            stream("/Type /EmbeddedFile", b"attached secret contents"),
            b"<< /Type /Annot /Subtype /Link /Rect [100 10 150 30] /A 8 0 R >>".to_vec(),
        ],
        "/Info 10 0 R",
    )
}

#[test]
fn sanitizing_removes_metadata_scripts_attachments_comments_and_hidden_text() {
    let document = open(dirty());
    let text = text_of(&document, 0);
    assert!(text.contains("Invisible OCR layer"), "the file hides text PDFium reads: {text}");

    let (sanitized, snapshot, report) =
        document.with_sanitized(SanitizeOptions::default()).expect("sanitizes");
    assert!(report.metadata >= 2 && report.scripts >= 3 && report.attachments >= 1, "{report:?}");
    assert!(report.comments >= 1 && report.hidden >= 3, "{report:?}");
    let text = text_of(&sanitized, 0);
    assert!(text.contains("Visible text") && text.contains("Also visible"), "{text}");
    for hidden in ["Invisible OCR layer", "Off the page text", "Hidden layer text"] {
        assert!(!text.contains(hidden), "{hidden} survived: {text}");
    }
    let bytes = decompressed(&snapshot);
    for leaked in [
        "Jane Roe",
        "Some Tool",
        "app.alert",
        "attached secret contents",
        "A private comment",
        "Invisible OCR layer",
        "Hidden layer text",
        "Off the page text",
        "xmpmeta",
        "EmbeddedFile",
    ] {
        assert!(!holds(&bytes, leaked), "{leaked} survived sanitizing");
    }
    assert!(holds(&bytes, "Visible text") && holds(&bytes, "Also visible"));
    assert!(sanitized.page_annotations(0).expect("annotations").0.is_empty());
}

#[test]
fn sanitizing_only_what_was_asked_leaves_the_rest() {
    let document = open(dirty());
    let options = SanitizeOptions {
        metadata: true,
        scripts: false,
        attachments: false,
        comments: false,
        hidden_content: false,
    };
    let (sanitized, snapshot, _) = document.with_sanitized(options).expect("sanitizes");
    let bytes = decompressed(&snapshot);
    assert!(!holds(&bytes, "Jane Roe") && !holds(&bytes, "Some Tool"));
    assert!(holds(&bytes, "app.alert") && holds(&bytes, "A private comment"));
    assert!(text_of(&sanitized, 0).contains("Invisible OCR layer"));
}

#[test]
fn a_sanitized_file_passes_qpdf_and_reopens() {
    let document = open(dirty());
    let (_, snapshot, _) = document.with_sanitized(SanitizeOptions::default()).expect("sanitizes");
    let path = scratch("sanitized.pdf");
    std::fs::write(&path, &*snapshot).expect("write");
    let output = Command::new("qpdf").arg("--check").arg(&path).output().expect("qpdf runs");
    assert!(matches!(output.status.code(), Some(0 | 3)), "{}", String::from_utf8_lossy(&output.stdout));
    assert_eq!(engine().open_file(&path, None).expect("reopens").page_count(), 1);
}

// ---------------------------------------------------------------- the whole corpus

#[test]
fn every_text_block_of_the_corpus_can_be_redacted_and_leaves_nothing_readable() {
    let manifest = manifest();
    let mut redacted = 0;
    for fixture in &manifest.fixtures {
        if fixture.damaged || fixture.password.is_some() || fixture.pages.is_some_and(|p| p > 50) {
            continue;
        }
        let document = engine().open_file(&fixture.file(), None).expect("opens");
        for page in 0..document.page_count().min(3) {
            let text = document.page_text(page).expect("text");
            // One block at a time, so every block's own fonts are exercised.
            for block in text.blocks.iter().take(4) {
                let mark = Mark { page, rect: block.bounds, text: None };
                let done = document
                    .with_redactions(&[mark])
                    .unwrap_or_else(|err| panic!("{} page {page}: {err}", fixture.name()));
                assert!(done.report.counts.glyphs > 0, "{} page {page}: {:?}", fixture.name(), done.report);
                redacted += 1;
            }
        }
    }
    assert!(redacted > 60, "only {redacted} blocks were exercised");
}

#[test]
fn sanitizing_the_whole_corpus_changes_no_visible_text_and_keeps_every_file_sound() {
    let manifest = manifest();
    let mut sanitized = 0;
    for fixture in &manifest.fixtures {
        if fixture.damaged || fixture.password.is_some() || fixture.pages.is_some_and(|p| p > 50) {
            continue;
        }
        let document = engine().open_file(&fixture.file(), None).expect("opens");
        let (after, bytes, _) = document
            .with_sanitized(SanitizeOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", fixture.name()));
        assert_eq!(after.page_count(), document.page_count(), "{}", fixture.name());
        for page in 0..document.page_count().min(3) {
            assert_eq!(text_of(&after, page), text_of(&document, page), "{} page {page}", fixture.name());
        }
        let path = scratch("corpus-sanitized.pdf");
        std::fs::write(&path, &*bytes).expect("write");
        let output = Command::new("qpdf").arg("--check").arg(&path).output().expect("qpdf runs");
        assert!(
            matches!(output.status.code(), Some(0 | 3)),
            "{}: {}",
            fixture.name(),
            String::from_utf8_lossy(&output.stdout)
        );
        sanitized += 1;
    }
    assert!(sanitized > 15, "only {sanitized} files were exercised");
}
