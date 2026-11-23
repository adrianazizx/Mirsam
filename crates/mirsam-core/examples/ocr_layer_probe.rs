//! Which order should the invisible OCR text layer be written in? (ADR 0035)
//!
//!     cargo run -p mirsam-core --example ocr_layer_probe -- <out dir>
//!
//! Writes the same lines (Arabic with ligatures and tashkeel, mixed Arabic and
//! English with numbers, English) as a text layer on a blank page, once per
//! [`Order`], and reads each file back with this app's extractor, PDFium's own
//! text (what Chrome copies), PDFKit (Preview; macOS only) and Poppler
//! (`pdftotext`, when installed). Prints, per reader and order, how many lines
//! come out exactly in logical order, and every line that does not.

use std::path::{Path, PathBuf};
use std::process::Command;

use mirsam_core::ocr::{LayerLine, LayerQuality, LayerWord, Order, PageLayer, text_layer_update_with};
use mirsam_core::{Engine, PdfiumSource};
use unicode_bidi::{BidiInfo, Level};

const LINES: &[(&str, bool)] = &[
    ("الموظفين لا يعملون يوم الجمعة", true),
    ("بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ", true),
    ("اشتريت جهاز iPhone 15 Pro بسعر ٤٥٠٠ ريال", true),
    ("الإصدار 2024 (64-bit) متوفر الآن", true),
    ("رقم الهاتف +966 11 234 5678 للاستفسار", true),
    ("صدر القرار رقم ١٢٣٤ لسنة ٢٠٢٥ في الرياض", true),
    ("The contract (العقد) starts on 1 March 2025.", false),
    ("The quick brown fox jumps over the lazy dog.", false),
];

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "target/ocr-layer-probe".into()));
    std::fs::create_dir_all(&out).expect("out dir");
    let engine = Engine::global(&PdfiumSource::Auto).expect("PDFium (scripts/fetch-pdfium.sh)");
    let blank = blank_pdf();
    let document = engine.open_bytes(blank.clone(), None).expect("blank opens");
    let placement = document.ocr_placement(0).expect("placement");
    let layer = PageLayer {
        page: 0,
        quality: LayerQuality::Final,
        lines: lines(),
        placement,
        clear: Default::default(),
    };
    let expected: Vec<String> = probe_lines().iter().map(|(text, _)| squash(text)).collect();

    for order in [
        Order::LineVisualRuns,
        Order::LineVisual,
        Order::WordVisual,
        Order::WordLogicalMirrored,
        Order::WordLogical,
    ] {
        let update =
            text_layer_update_with(&blank, None, std::slice::from_ref(&layer), order).expect("writes");
        let mut bytes = blank.clone();
        bytes.extend_from_slice(&update);
        let path = out.join(format!("{order:?}.pdf"));
        std::fs::write(&path, &bytes).expect("write");
        let qpdf = Command::new("qpdf").arg("--check").arg(&path).output().expect("qpdf");
        let reopened = engine.open_bytes(bytes, None).expect("reopens");
        let ours = reopened.page_text(0).expect("text").text();
        let mut readers = vec![("this app", ours), ("PDFium", reopened.page_text_raw(0).expect("raw"))];
        if let Some(text) = pdfkit(&path) {
            readers.push(("PDFKit", text));
        }
        if let Some(text) = poppler(&path) {
            readers.push(("Poppler", text));
        }
        println!("== {order:?} (qpdf {})", if qpdf.status.success() { "ok" } else { "FAILED" });
        for (reader, text) in readers {
            let got: Vec<String> = text.lines().map(squash).filter(|l| !l.is_empty()).collect();
            let exact = expected.iter().filter(|line| got.contains(line)).count();
            println!("  {reader:9} {exact}/{} lines exact", expected.len());
            for line in got.iter().filter(|l| !expected.contains(l)) {
                println!("      got: {line}");
            }
        }
    }
}

/// Whitespace collapsed, and bidi formatting characters (Poppler wraps runs in
/// embeddings) removed.
fn squash(text: &str) -> String {
    let text: String = text
        .chars()
        .filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .collect();
    use unicode_normalization::UnicodeNormalization as _;
    text.split_whitespace().collect::<Vec<_>>().join(" ").nfc().collect()
}

/// The probe lines laid out as a scan would show them: words in visual order,
/// 7 pt per character, 5 pt gaps, Arabic lines against the right margin.
fn probe_lines() -> Vec<(String, bool)> {
    match std::env::var("PROBE_LINES") {
        Ok(path) => std::fs::read_to_string(path)
            .expect("PROBE_LINES file")
            .lines()
            .filter_map(|l| l.split_once('|').map(|(d, t)| (t.to_owned(), d == "rtl")))
            .collect(),
        Err(_) => LINES.iter().map(|(t, r)| ((*t).to_owned(), *r)).collect(),
    }
}

fn lines() -> Vec<LayerLine> {
    probe_lines()
        .iter()
        .enumerate()
        .map(|(row, (text, rtl))| {
            let words: Vec<&str> = text.split(' ').collect();
            let text = text.as_str();
            let bidi = BidiInfo::new(text, Some(if *rtl { Level::rtl() } else { Level::ltr() }));
            let paragraph = &bidi.paragraphs[0];
            let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
            let mut visual = Vec::new();
            for run in runs {
                let mut offsets: Vec<usize> = (run.clone()).filter(|at| text.is_char_boundary(*at)).collect();
                if levels[run.start].is_rtl() {
                    offsets.reverse();
                }
                visual.extend(offsets);
            }
            let mut starts = Vec::new();
            let mut at = 0;
            for word in &words {
                starts.push(at..at + word.len());
                at += word.len() + 1;
            }
            let mut order: Vec<usize> = (0..words.len()).collect();
            order.sort_by_key(|&i| visual.iter().position(|v| starts[i].contains(v)).unwrap_or(0));
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
                rtl: *rtl,
            }
        })
        .collect()
}

fn pdfkit(path: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/pdfkit_text.swift");
    let output = Command::new("swift").arg(script).arg(path).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn poppler(path: &Path) -> Option<String> {
    let output = Command::new("pdftotext").arg("-enc").arg("UTF-8").arg(path).arg("-").output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A one-page A4 PDF with an empty page.
fn blank_pdf() -> Vec<u8> {
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
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}
