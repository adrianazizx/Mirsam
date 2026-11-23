//! The OCR text layer on real files (ADR 0035): invisible, aligned, in an
//! incremental update, read back in logical order by this app, PDFium (what
//! Chrome copies) and, on macOS, PDFKit (Preview); a draft replaced by the final
//! text; an encrypted file. Every write passes `qpdf --check`.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::manifest;
use mirsam_core::ocr::{LayerLine, LayerQuality, LayerWord, PageLayer};
use mirsam_core::{Document, Engine, PdfiumSource};
use unicode_bidi::{BidiInfo, Level};
use unicode_normalization::UnicodeNormalization as _;

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mirsam-ocr-layer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf_check(bytes: &[u8], name: &str, password: Option<&str>) {
    let path = scratch(name);
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

/// Lines with whitespace collapsed, NFC, bidi formatting characters dropped.
fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            let line: String = line
                .chars()
                .filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
                .collect();
            line.split_whitespace().collect::<Vec<_>>().join(" ").nfc().collect::<String>()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

/// Lines laid out as a scan shows them: words in visual order, 7 pt a character.
fn layer(page: usize, quality: LayerQuality, texts: &[(&str, bool)], document: &Document) -> PageLayer {
    let lines = texts
        .iter()
        .enumerate()
        .map(|(row, (text, rtl))| {
            let words: Vec<&str> = text.split(' ').collect();
            let bidi = BidiInfo::new(text, Some(if *rtl { Level::rtl() } else { Level::ltr() }));
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
            let mut x = if *rtl { 545.0 - total } else { 50.0 };
            let top = 80.0 + row as f32 * 40.0;
            let mut boxes = vec![(0.0, 0.0); words.len()];
            for i in order {
                boxes[i] = (x, x + width(words[i]));
                x += width(words[i]) + 5.0;
            }
            LayerLine {
                rtl: *rtl,
                words: words
                    .iter()
                    .zip(boxes)
                    .map(|(w, (left, right))| LayerWord {
                        text: (*w).into(),
                        left,
                        top,
                        right,
                        bottom: top + 16.0,
                    })
                    .collect(),
            }
        })
        .collect();
    PageLayer {
        page,
        quality,
        lines,
        placement: document.ocr_placement(page).expect("placement"),
        clear: Default::default(),
    }
}

/// A blank A4 page.
fn blank() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << >> /Contents 4 0 R >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

const ARABIC: &[(&str, bool)] = &[
    ("الموظفين لا يعملون يوم الجمعة", true),
    ("بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ", true),
    ("صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥ في الرياض", true),
];
const MIXED: &[(&str, bool)] = &[
    ("اشتريت جهاز iPhone 15 Pro بسعر ٤٥٠٠ ريال", true),
    ("رقم الهاتف +966 11 234 5678 للاستفسار", true),
    ("The contract (العقد) starts on 1 March 2025.", false),
    ("The quick brown fox jumps over the lazy dog.", false),
];

#[test]
fn the_layer_reads_back_in_logical_order() {
    let document = engine().open_bytes(blank(), None).expect("opens");
    let all: Vec<(&str, bool)> = ARABIC.iter().chain(MIXED).copied().collect();
    let (with_layer, update) =
        document.with_text_layers(&[layer(0, LayerQuality::Final, &all, &document)]).expect("writes");
    assert!(update.len() < 8_000, "the font is tiny and the content compressed: {} bytes", update.len());
    qpdf_check(&with_layer.bytes(), "layer.pdf", None);
    let expected: Vec<String> = all.iter().map(|(t, _)| lines(t)[0].clone()).collect();

    // This app: every line exact.
    assert_eq!(lines(&with_layer.page_text(0).expect("text").text()), expected);

    // PDFium, as Chrome copies: Arabic lines exact; its own bidi rules reverse
    // English words inside Arabic lines (measured in ADR 0035).
    let pdfium = lines(&with_layer.page_text_raw(0).expect("raw"));
    for (text, _) in ARABIC {
        assert!(pdfium.contains(&lines(text)[0]), "PDFium: {text} not in {pdfium:?}");
    }
    assert!(pdfium.contains(&lines(MIXED[3].0)[0]));

    // PDFKit (Preview): every line exact.
    if cfg!(target_os = "macos") {
        let path = scratch("layer-pdfkit.pdf");
        std::fs::write(&path, &*with_layer.bytes()).expect("write");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/pdfkit_text.swift");
        let output = Command::new("swift").arg(script).arg(&path).output().expect("swift runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(lines(&String::from_utf8_lossy(&output.stdout)), expected);
    }

    // The page draws nothing new: the text is invisible.
    let before = document.render_page(0, 1.0).expect("renders");
    let after = with_layer.render_page(0, 1.0).expect("renders");
    assert!(before.as_bgra() == after.as_bgra(), "the layer must not be drawn");
}

#[test]
fn the_final_text_replaces_the_draft() {
    let document = engine().open_bytes(blank(), None).expect("opens");
    let (drafted, _) = document
        .with_text_layers(&[layer(0, LayerQuality::Draft, &[("نص المسودة", true)], &document)])
        .expect("draft");
    assert_eq!(drafted.text_layer_qualities().expect("layers"), [Some(LayerQuality::Draft)]);
    let (finished, _) = drafted
        .with_text_layers(&[layer(0, LayerQuality::Final, &[("النص النهائي", true)], &drafted)])
        .expect("final");
    qpdf_check(&finished.bytes(), "replaced.pdf", None);
    assert_eq!(finished.text_layer_qualities().expect("layers"), [Some(LayerQuality::Final)]);
    assert_eq!(lines(&finished.page_text(0).expect("text").text()), ["النص النهائي"]);
    // One font in the page's resources, not one per layer.
    let raw = String::from_utf8_lossy(&finished.bytes()).into_owned();
    let last_page = raw.rmatch_indices("/Type /Page ").next().map(|(at, _)| &raw[at..]).expect("page");
    assert_eq!(last_page.split("endobj").next().map(|p| p.matches("/MOCR").count()), Some(1));
}

#[test]
fn an_encrypted_file_gets_an_encrypted_layer() {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|f| f.name() == "qpdf/encrypted")
        .expect("qpdf/encrypted in the corpus: run `node fixtures/generate.mjs` (needs qpdf)");
    let password = fixture.password.as_deref();
    let bytes = std::fs::read(fixture.file()).expect("fixture readable");
    let document = engine().open_bytes(bytes, password).expect("opens");
    let (with_layer, _) = document
        .with_text_layers(&[layer(0, LayerQuality::Final, &[("نص مشفّر Encrypted", true)], &document)])
        .expect("writes");
    qpdf_check(&with_layer.bytes(), "encrypted.pdf", password);
    let text = with_layer.page_text(0).expect("text").text();
    assert!(lines(&text).iter().any(|l| l == "نص مشفّر Encrypted"), "{text}");
}
