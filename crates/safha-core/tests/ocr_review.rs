//! M6's OCR work on real files (ADR 0037–0039), in Arabic, English and mixed:
//!
//! - a review correction rewrites the page's layer as an incremental update,
//!   reusing the layer's font, and the document says which layer it has;
//! - another tool's poor text layer over a scan is found and removed by byte
//!   range when our layer replaces it (the whole page, or one area);
//! - a recognised page becomes paragraphs of page text that the M3 editor
//!   opens, read back in logical order, with the scan kept behind or removed.
//!
//! Every write passes `qpdf --check`.

use std::path::PathBuf;
use std::process::Command;

use safha_core::ocr::convert::{Colors, PageConversion, paragraphs};
use safha_core::ocr::{Clear, LayerKind, LayerLine, LayerQuality, LayerWord, PageLayer, Problem, Verdict, check_page};
use safha_core::{Document, Engine, PdfiumSource};
use unicode_bidi::{BidiInfo, Level};
use unicode_normalization::UnicodeNormalization as _;

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("safha-ocr-review-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(bytes: &[u8], name: &str) {
    let path = scratch(name);
    std::fs::write(&path, bytes).expect("write");
    let output = Command::new("qpdf").arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check {name} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Text with whitespace collapsed, NFC, bidi formatting characters dropped.
fn clean(text: &str) -> String {
    let text: String = text
        .chars()
        .filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ").nfc().collect()
}

/// A one-page PDF (595 × 842) from object bodies 4…; objects 1–3 are the
/// catalog, the page tree and the page, whose `/Resources` and `/Contents`
/// are given.
fn pdf(resources: &str, contents: &str, more: &[Vec<u8>]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources {resources} /Contents {contents} >>")
            .into_bytes(),
    ];
    objects.extend(more.iter().cloned());
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    out
}

fn stream(data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

/// A light grey image over the whole page, and `extra` drawn after it.
fn scan(extra: &[u8]) -> Vec<u8> {
    let mut content = b"q 595 0 0 842 0 0 cm /Im0 Do Q\n".to_vec();
    content.extend_from_slice(extra);
    let image = {
        let mut body =
            b"<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >>\nstream\n"
                .to_vec();
        body.extend_from_slice(&[0xF0; 4]);
        body.extend_from_slice(b"\nendstream");
        body
    };
    pdf(
        "<< /XObject << /Im0 5 0 R >> /Font << /F1 6 0 R >> >>",
        "4 0 R",
        &[
            stream(&content),
            image,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        ],
    )
}

/// Another tool's invisible layer: Arabic stored as Windows-1256 bytes and
/// shown in a Latin font, so it reads as "ÇáÚÞÏ" (mojibake), two lines.
fn foreign_layer() -> Vec<u8> {
    let mut out = b"BT 3 Tr /F1 14 Tf 1 0 0 1 300 760 Tm (".to_vec();
    out.extend_from_slice(&[0xC7, 0xE1, 0xDA, 0xDE, 0xCF, 0x20, 0xC7, 0xE1, 0xC5, 0xED, 0xCC, 0xC7, 0xD1]);
    out.extend_from_slice(b") Tj 1 0 0 1 300 700 Tm (");
    out.extend_from_slice(&[0xC8, 0xED, 0xE4, 0x20, 0xC7, 0xE1, 0xD8, 0xD1, 0xDD, 0xED, 0xE4]);
    out.extend_from_slice(b") Tj ET\n");
    out
}

/// Lines as a scan shows them, at `tops` (displayed points), 7 pt a character,
/// in visual order, 14 pt tall.
fn layer(document: &Document, texts: &[(&str, bool, f32)]) -> PageLayer {
    let lines = texts
        .iter()
        .map(|&(text, rtl, top)| {
            let words: Vec<&str> = text.split(' ').collect();
            let bidi = BidiInfo::new(text, Some(if rtl { Level::rtl() } else { Level::ltr() }));
            let paragraph = &bidi.paragraphs[0];
            let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
            let mut visual = Vec::new();
            for run in runs {
                let mut offsets: Vec<usize> = run.clone().filter(|at| text.is_char_boundary(*at)).collect();
                if levels[run.start].is_rtl() {
                    offsets.reverse();
                }
                visual.extend(offsets);
            }
            let mut ranges = Vec::new();
            let mut at = 0;
            for word in &words {
                ranges.push(at..at + word.len());
                at += word.len() + 1;
            }
            let strong = |at: usize| text[at..].chars().next().is_some_and(char::is_alphanumeric);
            let mut order: Vec<usize> = (0..words.len()).collect();
            order.sort_by_key(|&i| visual.iter().position(|&v| ranges[i].contains(&v) && strong(v)));
            let width = |w: &str| w.chars().count() as f32 * 7.0;
            let total: f32 = words.iter().map(|w| width(w)).sum::<f32>() + 5.0 * (words.len() - 1) as f32;
            let mut x = if rtl { 545.0 - total } else { 50.0 };
            let mut boxes = vec![(0.0, 0.0); words.len()];
            for i in order {
                boxes[i] = (x, x + width(words[i]));
                x += width(words[i]) + 5.0;
            }
            LayerLine {
                rtl,
                words: words
                    .iter()
                    .zip(boxes)
                    .map(|(w, (left, right))| LayerWord { text: (*w).into(), left, top, right, bottom: top + 14.0 })
                    .collect(),
            }
        })
        .collect();
    PageLayer {
        page: 0,
        quality: LayerQuality::Final,
        lines,
        placement: document.ocr_placement(0).expect("placement"),
        clear: Clear::None,
    }
}

const PAGE: &[(&str, bool, f32)] = &[
    // An Arabic paragraph of three lines, then an English one of two, then a mixed line apart.
    ("نص العقد بين الطرفين في مدينة الرياض", true, 80.0),
    ("على أن يلتزم المستأجر بدفع الأجرة", true, 100.0),
    ("الشهرية في موعدها", true, 120.0),
    ("This lease agreement is made between", false, 200.0),
    ("the parties on 1 March 2025.", false, 220.0),
    ("اشتريت جهاز iPhone 15 Pro بسعر ٤٥٠٠ ريال", true, 320.0),
];

fn text_of(document: &Document) -> String {
    clean(&document.page_text(0).expect("text").text())
}

#[test]
fn a_correction_rewrites_the_layer_and_reuses_its_font() {
    let document = engine().open_bytes(scan(b""), None).expect("opens");
    let recognised = layer(&document, PAGE);
    let (with_layer, _) = document.with_text_layers(std::slice::from_ref(&recognised)).expect("writes");
    let states = with_layer.text_layer_states().expect("states");
    assert_eq!(states[0].quality, Some(LayerQuality::Final));
    assert_eq!(states[0].digest, Some(recognised.digest()));

    // The review corrects the price (Arabic-Indic digits) and an English word.
    let mut corrected = recognised.clone();
    corrected.lines[5].words[6].text = "٤٨٠٠".into();
    corrected.lines[3].words[1].text = "Lease".into();
    let (fixed, update) = with_layer.with_text_layers(std::slice::from_ref(&corrected)).expect("corrects");
    qpdf_check(&fixed.bytes(), "corrected.pdf");
    assert!(!update.windows(10).any(|w| w == b"/FontFile2"), "the layer's font is reused, not written again");
    assert!(update.len() < 4_000, "a correction is a small update: {} bytes", update.len());
    let text = text_of(&fixed);
    assert!(text.contains("بسعر ٤٨٠٠ ريال") && !text.contains("٤٥٠٠"), "{text}");
    assert!(text.contains("This Lease agreement"), "{text}");
    assert_eq!(fixed.text_layer_states().expect("states")[0].digest, Some(corrected.digest()));
    // One layer stream on the page, as before.
    let layers = fixed.bytes().windows(10).filter(|w| *w == b"/MirsamOCR").count();
    assert_eq!(layers, 2, "the old stream stays in the file's history, the page uses the new one");
}

#[test]
fn another_tools_poor_layer_is_found_and_replaced() {
    let document = engine().open_bytes(scan(&foreign_layer()), None).expect("opens");
    let info = document.text_info(0).expect("info");
    assert_eq!((info.kind(), info.ours), (LayerKind::Invisible, false));
    assert!(info.text.contains("ÇáÚÞÏ"), "{}", info.text);
    let check = check_page(0, &info, None);
    assert_eq!(check.verdict, Verdict::Poor);
    assert!(check.problems.contains(&Problem::Mojibake), "{:?}", check.problems);

    // Our layer takes its place: the old text is gone by byte range.
    let mut ours = layer(&document, &PAGE[..3]);
    ours.clear = Clear::Page;
    let (replaced, _) = document.with_text_layers(&[ours]).expect("replaces");
    qpdf_check(&replaced.bytes(), "replaced.pdf");
    let text = text_of(&replaced);
    assert!(!text.contains('Ç') && text.contains("نص العقد بين الطرفين"), "{text}");
    let info = replaced.text_info(0).expect("info");
    assert_eq!((info.invisible, info.visible, info.ours), (0, 0, true));
    // Nothing visible changed: both layers are invisible, the scan stays.
    let (before, after) = (document.render_page(0, 0.5).expect("renders"), replaced.render_page(0, 0.5).expect("renders"));
    assert!(before.as_bgra() == after.as_bgra());

    // An area read again clears only the other tool's text inside it.
    let mut area = layer(&document, &[("بين الطرفين", true, 140.0)]);
    let rect = [280.0, 72.0, 595.0, 92.0]; // the first foreign line (y 760 → 82 displayed)
    area.clear = Clear::Region { rect };
    let (partly, _) = document.with_text_layers(&[area]).expect("area");
    qpdf_check(&partly.bytes(), "area.pdf");
    let text = text_of(&partly);
    assert!(!text.contains("ÇáÚÞÏ") && text.contains("Èíä ÇáØÑÝíä"), "{text}");
}

#[test]
fn a_recognised_page_becomes_paragraphs_the_editor_opens() {
    for keep_scan in [true, false] {
        let document = engine().open_bytes(scan(&foreign_layer()), None).expect("opens");
        let recognised = layer(&document, PAGE);
        let mut written = recognised.clone();
        written.clear = Clear::Page;
        let (with_layer, _) = document.with_text_layers(&[written]).expect("layer");
        let found = paragraphs(&recognised);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert_eq!(found[0].lines.len(), 3);
        assert!(found[0].rtl && !found[1].rtl && found[2].rtl);

        let conversion = PageConversion {
            page: 0,
            layer: recognised.clone(),
            from: recognised.digest(),
            keep_scan,
            family: None,
            colors: Colors::default(),
        };
        let (converted, _) = with_layer.with_conversion(&[conversion]).expect("converts");
        qpdf_check(&converted.bytes(), &format!("converted-{keep_scan}.pdf"));

        // The page's text is page content now, in logical order; the OCR layer is gone.
        let states = converted.text_layer_states().expect("states");
        assert_eq!((states[0].quality, states[0].converted), (None, Some(recognised.digest())));
        let text = text_of(&converted);
        for expected in [
            "نص العقد بين الطرفين في مدينة الرياض على أن يلتزم المستأجر بدفع الأجرة الشهرية في موعدها",
            "This lease agreement is made between the parties on 1 March 2025.",
            "اشتريت جهاز iPhone 15 Pro بسعر ٤٥٠٠ ريال",
        ] {
            assert!(clean(&text.replace('\n', " ")).contains(expected), "{expected} not in {text}");
        }
        assert!(!text.contains('Ç'), "the other tool's layer went too");

        // Paragraphs the M3 editor opens and edits.
        let blocks = converted.text_blocks(0).expect("blocks");
        let editable: Vec<_> = blocks.iter().filter(|b| b.editable).collect();
        assert!(editable.len() >= 3, "{blocks:#?}");
        let arabic = editable.iter().find(|b| b.text.contains("نص العقد")).expect("the Arabic paragraph");
        assert!(arabic.rtl);
        let edit = safha_core::block_edit::BlockEdit {
            page: 0,
            index: arabic.index,
            fingerprint: arabic.fingerprint,
            text: arabic.text.replace("الرياض", "جدة"),
            family: None,
            marks: None,
        };
        let (edited, _, _) = converted.with_block_edit(&edit).expect("the editor edits it");
        assert!(text_of(&edited).contains("مدينة جدة"));

        // The scan: kept behind (patched where the text is), or removed.
        let images = converted
            .page_objects(0)
            .expect("objects")
            .iter()
            .filter(|o| o.kind == safha_core::objects::ObjectKind::Image)
            .count();
        assert_eq!(images, usize::from(keep_scan));
    }
}
