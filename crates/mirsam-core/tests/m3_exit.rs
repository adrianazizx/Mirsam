//! Milestone 3's exit test: spec §7's four acceptance tests, each clause
//! checked as far as this machine allows (docs/demos/m3.md).
//!
//! "Another viewer" is checked with what is installed:
//!
//! - PDFium's own page text and rendering: Chrome's engine, everywhere;
//! - Poppler (`pdftoppm`, `pdftotext`): the engine of Linux's viewers, when
//!   installed (it comes with qpdf from Homebrew);
//! - PDFKit (`sips`, and `tests/support/pdfkit_text.swift`): Preview's
//!   engine, on macOS.
//!
//! Every check that cannot run here prints why ("not checked here"); none is
//! skipped silently. Acrobat Reader cannot be driven here: the demo script
//! lists it as a check by hand.
//!
//! Copy-paste of Arabic is required to be exact in this app (our extractor)
//! and in PDFKit. PDFium's and Poppler's text is printed, not required: both
//! reverse the letters of Arabic ligatures and the order of Latin runs inside
//! Arabic lines, in Chrome's own files as much as in ours, and no single
//! `/ToUnicode` convention satisfies them and PDFKit together (PROGRESS.md,
//! owner item 10; measured in docs/demos/m3.md).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::manifest;
use mirsam_core::block_edit::{BlockEdit, EditedBlock, FontSource, TextBlock};
use mirsam_core::fonts::FontFamily;
use mirsam_core::save::save;
use mirsam_core::search::{Query, SearchOptions};
use mirsam_core::text_edit::NewText;
use mirsam_core::typeset::{Align, BaseDirection};
use mirsam_core::{Bitmap, Document, Engine, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mirsam-m3-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Saves `document` (PDFium's full rewrite, as Save does), reopens it from disk.
fn saved(document: &Document, name: &str) -> (Document, PathBuf) {
    let path = scratch(name);
    save(document, &path).expect("saves");
    (engine().open_file(&path, None).expect("reopens"), path)
}

fn written(bytes: &[u8], name: &str) -> PathBuf {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("writes");
    path
}

fn qpdf_check(path: &Path) {
    let output = Command::new("qpdf").arg("--check").arg(path).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stdout)
    );
}

fn block_with(document: &Document, needle: &str) -> TextBlock {
    let blocks = document.text_blocks(0).expect("blocks");
    blocks
        .iter()
        .find(|b| normalize(&b.text).contains(needle))
        .unwrap_or_else(|| {
            panic!("no block holds {needle:?}: {:#?}", blocks.iter().map(|b| &b.text).collect::<Vec<_>>())
        })
        .clone()
}

fn edit_to(document: &Document, block: &TextBlock, text: &str) -> (Document, EditedBlock) {
    let edit = BlockEdit {
        page: 0,
        index: block.index,
        fingerprint: block.fingerprint,
        text: text.into(),
        family: None,
        marks: None,
    };
    let (edited, _, outcome) = document.with_block_edit(&edit).unwrap_or_else(|e| panic!("{e}"));
    (edited, outcome)
}

fn add_text(document: &Document, text: &str, family: FontFamily, width: f32, y: f32) -> Document {
    let new = NewText {
        page: 0,
        x: 72.0,
        y,
        width,
        text: text.into(),
        family,
        size: 14.0,
        color: [0.0; 3],
        align: Align::Start,
        direction: BaseDirection::Auto,
    };
    document.with_new_text(&new).expect("adds").0
}

/// Font programs in a document after PDFium's full rewrite (dead objects gone).
fn embedded_fonts(document: &Document) -> usize {
    String::from_utf8_lossy(&document.to_pdf_bytes().expect("rewrites")).matches("/FontFile").count()
}

// ------------------------------------------------------------- the viewers

/// A grey image: one byte per pixel.
struct Grey {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Grey {
    fn from_bitmap(bitmap: &Bitmap) -> Grey {
        let (width, height) = (bitmap.width() as usize, bitmap.height() as usize);
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..bitmap.height() {
            for x in 0..bitmap.width() {
                let [r, g, b, _] = bitmap.pixel(x, y).unwrap_or([255; 4]);
                pixels.push(((u32::from(r) + u32::from(g) + u32::from(b)) / 3) as u8);
            }
        }
        Grey { width, height, pixels }
    }

    fn from_png(path: &Path) -> Option<Grey> {
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buffer).ok()?;
        let channels = info.color_type.samples();
        let bytes = &buffer[..info.buffer_size()];
        let pixels = bytes
            .chunks(channels * (info.bit_depth as usize / 8).max(1))
            .map(|p| match channels {
                1 | 2 => p[0],
                _ => ((u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3) as u8,
            })
            .collect();
        Some(Grey { width: info.width as usize, height: info.height as usize, pixels })
    }

    /// Pixels that differ visibly from `other` outside the band of page
    /// points `top..bottom` (all of the page's width), at `per_point`
    /// pixels per point.
    fn changes_outside(&self, other: &Grey, (top, bottom): (f32, f32), per_point: f32) -> usize {
        assert_eq!((self.width, self.height), (other.width, other.height), "renders differ in size");
        let (y0, y1) = ((top - 3.0) * per_point, (bottom + 3.0) * per_point);
        (0..self.height)
            .filter(|&y| (y as f32) < y0 || (y as f32) > y1)
            .map(|y| {
                (0..self.width)
                    .filter(|&x| {
                        self.pixels[y * self.width + x].abs_diff(other.pixels[y * self.width + x]) > 48
                    })
                    .count()
            })
            .sum()
    }
}

fn tool(name: &str) -> bool {
    Command::new(name).arg("-v").output().is_ok()
}

/// Page 1 rendered by Poppler at 72 dpi (a pixel a point).
fn poppler_render(path: &Path) -> Option<Grey> {
    if !tool("pdftoppm") {
        println!("not checked here: Poppler's rendering (pdftoppm is not installed)");
        return None;
    }
    let prefix = path.with_extension("poppler");
    let status = Command::new("pdftoppm")
        .args(["-f", "1", "-l", "1", "-r", "72", "-gray", "-png", "-singlefile"])
        .arg(path)
        .arg(&prefix)
        .status()
        .ok()?;
    assert!(status.success(), "pdftoppm failed on {}", path.display());
    Grey::from_png(&prefix.with_extension("png"))
}

/// Page 1 rendered by PDFKit (`sips`, macOS) at a pixel a point.
fn pdfkit_render(path: &Path) -> Option<Grey> {
    if !cfg!(target_os = "macos") {
        println!("not checked here: PDFKit's rendering (macOS only)");
        return None;
    }
    let out = path.with_extension("pdfkit.png");
    let status =
        Command::new("sips").args(["-s", "format", "png"]).arg(path).arg("--out").arg(&out).output().ok()?;
    assert!(status.status.success(), "sips failed on {}", path.display());
    Grey::from_png(&out)
}

fn poppler_text(path: &Path) -> Option<String> {
    if !tool("pdftotext") {
        println!("not checked here: Poppler's text (pdftotext is not installed)");
        return None;
    }
    let output = Command::new("pdftotext")
        .args(["-f", "1", "-l", "1", "-enc", "UTF-8"])
        .arg(path)
        .arg("-")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).replace(['\u{202A}', '\u{202B}', '\u{202C}'], ""))
}

fn pdfkit_text(path: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        println!("not checked here: PDFKit's text (macOS only)");
        return None;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/pdfkit_text.swift");
    let output = Command::new("xcrun").arg("swift").arg(script).arg(path).output().ok()?;
    assert!(
        output.status.success(),
        "PDFKit could not read {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The text each viewer reads from `path`, as (viewer, text), for viewers present.
fn texts(document: &Document, path: &Path) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("this app", normalize(&document.page_text(0).expect("text").text())),
        ("PDFium (Chrome)", normalize(&document.page_text_raw(0).expect("raw text"))),
    ];
    if let Some(text) = poppler_text(path) {
        out.push(("Poppler", normalize(&text)));
    }
    if let Some(text) = pdfkit_text(path) {
        out.push(("PDFKit (Preview)", normalize(&text)));
    }
    out
}

// ------------------------------------------------------- 1. an English typo

/// A one-page letter in Helvetica and Helvetica-Bold (standard fonts, with
/// metrics), three paragraphs: a page every viewer draws with its own fonts.
fn helvetica_letter() -> Vec<u8> {
    let content = "BT /F1 11 Tf 14 TL 72 700 Td (Dear tenant,) Tj T* T* (Please note that the rent for the month of March is due on the first day of the) Tj T* (month. We did not recieve last month's payment until the tenth, and ask that) Tj T* (you arrange a standing order with your bank.) Tj T* T* (Yours sincerely,) Tj T* /F2 11 Tf (The Landlord) Tj ET\n";
    let widths = |w: &str| vec![w; 95].join(" ");
    let font = |name: &str, w: &str| {
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths [{}] >>",
            widths(w)
        )
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        font("Helvetica", "500"),
        font("Helvetica-Bold", "556"),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    pdf
}

/// Fixes `typo` → `fixed` in the paragraph holding `needle`, saves, and checks
/// every clause of the first acceptance test on what comes back.
fn check_typo(name: &str, original: &Document, original_path: &Path, needle: &str, typo: &str, fixed: &str) {
    let block = block_with(original, needle);
    assert!(block.editable, "{name}: {block:?}");
    let (edited, outcome) = edit_to(original, &block, &block.text.replace(typo, fixed));
    assert_eq!(outcome.source, FontSource::Page, "{name}: the paragraph's own font, nothing embedded");
    let (reopened, path) = saved(&edited, &format!("{name}-fixed.pdf"));
    qpdf_check(&path);

    // Surrounding lines keep their positions (and text).
    let before = original.text_blocks(0).expect("blocks");
    let after = reopened.text_blocks(0).expect("blocks");
    assert_eq!(before.len(), after.len(), "{name}");
    for (a, b) in before.iter().zip(&after) {
        if a.index != block.index {
            assert!(
                (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && a.text == b.text,
                "{name}: another paragraph moved: {a:?} -> {b:?}"
            );
        }
    }
    let band = (block.y, block.y + block.height);
    // Drawn the same, outside the paragraph, by each engine there is.
    let scale = 2.0;
    let pdfium = |document: &Document| Grey::from_bitmap(&document.render_page(0, scale).expect("renders"));
    assert_eq!(
        pdfium(original).changes_outside(&pdfium(&reopened), band, scale),
        0,
        "{name}: PDFium (Chrome)"
    );
    for (viewer, render) in
        [("Poppler", poppler_render as fn(&Path) -> Option<Grey>), ("PDFKit", pdfkit_render)]
    {
        if let (Some(a), Some(b)) = (render(original_path), render(&path)) {
            assert_eq!(
                a.changes_outside(&b, band, 1.0),
                0,
                "{name}: {viewer} draws something else outside the paragraph"
            );
        }
    }
    // Read the same, by each viewer there is.
    let sentence = block.text.replace(typo, fixed);
    let words: Vec<&str> = sentence.split_whitespace().collect();
    let fixed_phrase = words
        .windows(3)
        .find(|w| w.iter().any(|word| word.contains(fixed)))
        .map(|w| w.join(" "))
        .unwrap_or_default();
    for (viewer, text) in texts(&reopened, &path) {
        assert!(text.contains(&fixed_phrase) && !text.contains(typo), "{name}: {viewer} reads {text:?}");
    }
}

#[test]
fn acceptance_1_a_typo_fixed_in_english_moves_no_other_line_and_reads_the_same_in_every_viewer() {
    let letter = helvetica_letter();
    let path = written(&letter, "letter.pdf");
    let document = engine().open_bytes(letter, None).expect("opens");
    check_typo("letter", &document, &path, "We did not recieve", "recieve", "receive");

    // A producer's own embedded font: CoreText's (macOS fixtures).
    let manifest = manifest();
    if let Some(fixture) = manifest.fixture_or_unavailable("coretext/en-paragraphs") {
        let document = engine().open_file(&fixture.file(), None).expect("opens");
        check_typo("coretext", &document, &fixture.file(), "Questions? Email", "call", "phone");
    }
    println!("checked by hand (docs/demos/m3.md): Acrobat Reader");
}

// ---------------------------------------------- 2. a word inserted in Arabic

/// The union of the boxes of `word` (whole word) on page 0, as displayed, and
/// the dark pixels in it, from a render at `scale`.
fn word_ink(document: &Document, word: &str, scale: f32) -> (f32, usize) {
    let options = SearchOptions { whole_word: true, arabic_normalize: false, ..SearchOptions::default() };
    let query = Query::new(word, options).expect("query");
    let page = document.page_text(0).expect("text");
    let hits = query.find_on_page(&page);
    assert_eq!(hits.len(), 1, "{word:?} once on the page");
    let view = document.view_transform(0).expect("view");
    let rects: Vec<_> = hits[0].rects.iter().map(|r| view.rect(*r)).collect();
    let left = rects.iter().map(|r| r.x).fold(f32::MAX, f32::min);
    let right = rects.iter().map(|r| r.x + r.width).fold(f32::MIN, f32::max);
    let top = rects.iter().map(|r| r.y).fold(f32::MAX, f32::min);
    let bottom = rects.iter().map(|r| r.y + r.height).fold(f32::MIN, f32::max);
    let image = document.render_page(0, scale).expect("renders");
    let mut ink = 0;
    for y in (top * scale) as u32..(bottom * scale) as u32 {
        for x in (left * scale) as u32..(right * scale) as u32 {
            ink += usize::from(image.pixel(x, y).is_some_and(|p| p[0] < 128 && p[1] < 128 && p[2] < 128));
        }
    }
    (right - left, ink)
}

#[test]
fn acceptance_2_a_word_inserted_into_arabic_joins_rewraps_right_to_left_and_copies_in_order() {
    // Chrome's Arabic in IBM Plex Sans Arabic: the paragraph, with its damma,
    // is set again in the same face, bundled (its subset cannot place marks).
    let manifest = manifest();
    let fixture = manifest.fixture_or_unavailable("chrome/ar-sans").expect("a Chrome fixture");
    let document = engine().open_file(&fixture.file(), None).expect("opens");
    let block = block_with(&document, "من جميع الموظفين");
    let inserted = block.text.replace("الموظفين", "الموظفين والمتعاقدين");
    let (edited, outcome) = edit_to(&document, &block, &inserted);
    assert_eq!(outcome.source, FontSource::Bundled);
    let (reopened, path) = saved(&edited, "arabic-inserted.pdf");
    qpdf_check(&path);

    // Letters join: a word of the paragraph, set again by this app, is drawn
    // as Chrome drew it (same width and ink, within a few percent); unjoined
    // letters are wider and lighter.
    let (width_before, ink_before) = word_ink(&document, "الموظفين", 4.0);
    let (width_after, ink_after) = word_ink(&reopened, "الموظفين", 4.0);
    assert!((width_after - width_before).abs() < 0.05 * width_before, "{width_before} vs {width_after} pt");
    assert!(ink_after.abs_diff(ink_before) * 10 < ink_before, "{ink_before} vs {ink_after} dark pixels");

    // Re-wrapped right to left: its first line and right edge stay where they were.
    let after = block_with(&reopened, "والمتعاقدين");
    assert!(after.rtl && after.lines >= block.lines, "{after:?}");
    assert!((after.y - block.y).abs() < 1.5, "{} vs {}", after.y, block.y);
    assert!(((after.x + after.width) - (block.x + block.width)).abs() < 2.5, "{after:?} vs {block:?}");

    // Copied from the saved file: exact here and in PDFKit; printed for PDFium and Poppler.
    for (viewer, text) in texts(&reopened, &path) {
        println!("{viewer}: {text}");
        match viewer {
            "this app" | "PDFKit (Preview)" => {
                assert!(text.contains(&normalize(&inserted)), "{viewer} reads {text:?}");
            }
            _ => assert!(text.contains("والمتعاقدين"), "{viewer} reads {text:?}"),
        }
    }
}

// ------------------------------------------------------- 3. a mixed sentence

#[test]
fn acceptance_3_a_mixed_sentence_keeps_its_order() {
    let manifest = manifest();
    let fixture = manifest.fixture_or_unavailable("chrome/en-paragraphs").expect("a Chrome fixture");
    let page = engine().open_file(&fixture.file(), None).expect("opens");
    let sentence = "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال للجهاز.";
    let document = add_text(&page, sentence, FontFamily::Sans, 400.0, 560.0);
    let block = block_with(&document, "iPhone");
    assert_eq!(normalize(&block.text), sentence);
    let changed =
        sentence.replace("3 أجهزة", "5 أجهزة").replace("15 Pro", "16 Pro").replace("4,500", "4,250");
    let (edited, _) = edit_to(&document, &block, &changed);
    let (reopened, path) = saved(&edited, "mixed.pdf");
    qpdf_check(&path);
    assert_eq!(normalize(&block_with(&reopened, "iPhone").text), changed);
    for (viewer, text) in texts(&reopened, &path) {
        println!("{viewer}: {text}");
        if matches!(viewer, "this app" | "PDFKit (Preview)") {
            assert!(text.contains(&changed), "{viewer} reads {text:?}");
        }
    }
}

// ---------------------------------------------------------- 4. a round trip

/// Edit → save → reopen → edit again → save → reopen: the paragraph stays
/// where it is, reads as edited, and no font is embedded twice.
fn round_trip(name: &str, document: &Document, needle: &str, first: (&str, &str), second: (&str, &str)) {
    let block = block_with(document, needle);
    let (edited, _) = edit_to(document, &block, &block.text.replace(first.0, first.1));
    let (once, path) = saved(&edited, &format!("{name}-once.pdf"));
    qpdf_check(&path);
    let one = block_with(&once, needle);
    let fonts_once = embedded_fonts(&once);

    let (edited, _) = edit_to(&once, &one, &one.text.replace(second.0, second.1));
    let (twice, path) = saved(&edited, &format!("{name}-twice.pdf"));
    qpdf_check(&path);
    let two = block_with(&twice, needle);
    let expected = block.text.replace(first.0, first.1).replace(second.0, second.1);
    assert_eq!(normalize(&two.text), normalize(&expected), "{name}");
    assert!(
        (two.x - one.x).abs() < 0.6 && (two.y - one.y).abs() < 0.6 && (two.width - one.width).abs() < 0.6,
        "{name}: drift: {one:?} -> {two:?}"
    );
    assert!(
        embedded_fonts(&twice) <= fonts_once.max(embedded_fonts(document)),
        "{name}: a font embedded twice"
    );
}

#[test]
fn acceptance_4_open_edit_save_reopen_edit_again_without_drift_or_duplicate_fonts() {
    // English in the page's own fonts.
    let letter = engine().open_bytes(helvetica_letter(), None).expect("opens");
    round_trip("english", &letter, "We did not", ("recieve", "receive"), ("tenth", "twelfth"));

    // Arabic written by this app, set again in bundled faces.
    let manifest = manifest();
    let fixture = manifest.fixture_or_unavailable("chrome/en-paragraphs").expect("a Chrome fixture");
    let page = engine().open_file(&fixture.file(), None).expect("opens");
    let arabic =
        "يستحق السداد خلال ثلاثين يوما من تاريخ الفاتورة، وتترتب على المتأخرات فائدة بالنسبة المحددة في العقد.";
    let document = add_text(&page, arabic, FontFamily::Naskh, 260.0, 560.0);
    round_trip("arabic", &document, "يستحق السداد", ("ثلاثين", "ستين"), ("ستين", "تسعين"));

    // Chrome's own Arabic font, embedded again from the PDF (ADR 0026).
    let fixture = manifest.fixture_or_unavailable("chrome/ar-sans").expect("a Chrome fixture");
    let document = engine().open_file(&fixture.file(), None).expect("opens");
    let heading = block_with(&document, "إشعار للموظفين");
    let (_, outcome) = edit_to(&document, &heading, "إشعار");
    assert_eq!(outcome.source, FontSource::Embedded);
    // A word taken away, then the rest set again: its own glyphs both times.
    round_trip("embedded", &document, "إشعار", (" للموظفين", ""), ("إشعار", "إشعار"));
}
