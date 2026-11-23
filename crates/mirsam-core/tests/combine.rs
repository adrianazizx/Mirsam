//! Pages across files (ADR 0016): extract, split, and insert from another PDF.

mod common;

use std::process::Command;

use common::manifest;
use mirsam_core::edit::EditHistory;
use mirsam_core::{Document, Engine, Error, PdfiumSource};

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

fn reopen(bytes: &[u8]) -> Document {
    engine().open_bytes(bytes.to_vec(), None).expect("PDFium opens the result")
}

fn texts(document: &Document) -> Vec<String> {
    (0..document.page_count())
        .map(|i| document.page_text(i).expect("text").text().split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

fn qpdf_check(bytes: &[u8]) {
    let path =
        std::env::temp_dir().join(format!("mirsam-combine-{}-{}.pdf", std::process::id(), bytes.len()));
    std::fs::write(&path, bytes).expect("write");
    let output = Command::new("qpdf").arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(output.status.code(), Some(0 | 3)), "{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn extracting_copies_the_chosen_pages_in_the_chosen_order() {
    let document = open("qpdf/page-labels");
    let original = texts(&document);
    let bytes = document.extract_pages(&[2, 0]).expect("extracts");
    qpdf_check(&bytes);
    assert_eq!(texts(&reopen(&bytes)), vec![original[2].clone(), original[0].clone()]);
    assert!(matches!(document.extract_pages(&[]), Err(Error::InvalidEdit(_))));
    assert!(matches!(document.extract_pages(&[3]), Err(Error::PageOutOfRange { .. })));
}

#[test]
fn splitting_makes_parts_of_the_given_size() {
    let document = open("qpdf/page-labels");
    let original = texts(&document);
    let parts = document.split_every(2).expect("splits");
    assert_eq!(parts.len(), 2);
    for part in &parts {
        qpdf_check(part);
    }
    assert_eq!(texts(&reopen(&parts[0])), original[..2].to_vec());
    assert_eq!(texts(&reopen(&parts[1])), original[2..].to_vec());
    assert!(matches!(document.split_every(0), Err(Error::InvalidEdit(_))));
}

#[test]
fn pages_of_an_encrypted_file_can_be_extracted() {
    // The reader opened it with the password; the copy is a new, open file.
    let bytes = open("qpdf/encrypted").extract_pages(&[0]).expect("extracts");
    assert_eq!(reopen(&bytes).page_count(), 1);
}

#[test]
fn inserting_another_file_merges_it_and_undo_restores_the_original() {
    let document = open("qpdf/page-labels");
    let other = open("chrome/mixed-inline");
    let outline = document.outline().len();
    let mut history = EditHistory::new(document.bytes());

    let (merged, bytes) = document.with_pages_from(&[&other, &other], 1).expect("inserts");
    history.push_snapshot(bytes);
    qpdf_check(&merged.bytes());
    let original = texts(&document);
    let inserted = texts(&other)[0].clone();
    assert_eq!(
        texts(&merged),
        vec![original[0].clone(), inserted.clone(), inserted, original[1].clone(), original[2].clone()]
    );
    // The document's own outline and labels stay.
    assert_eq!(merged.outline().len(), outline);
    assert_eq!(merged.page_label(0).expect("label").as_deref(), Some("i"));

    let undone = merged.revision(history.undo().expect("undo")).expect("reopens");
    assert_eq!(texts(&undone), original);
    assert!(matches!(document.with_pages_from(&[&other], 4), Err(Error::PageOutOfRange { .. })));
}

/// A five-page file whose outline has top-level entries on pages 2, 3 (twice,
/// the second a duplicate) and 5, a child entry on page 4, and one entry that
/// points nowhere. Each page draws its own number.
fn bookmarked_pdf() -> Vec<u8> {
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 14 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R 7 0 R] /Count 5 >>".into(),
    ];
    // Pages 3–7, their contents 8–12, the font 13.
    for page in 0..5 {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents {} 0 R /Resources << /Font << /F1 13 0 R >> >> >>",
            8 + page
        ));
    }
    for page in 0..5 {
        let text = format!("BT /F1 24 Tf 20 100 Td (Page {}) Tj ET", page + 1);
        objects.push(format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    // The outline, 14–20.
    let dest = |page: usize| format!("/Dest [{} 0 R /XYZ null null null]", 3 + page);
    objects.extend([
        "<< /Type /Outlines /First 15 0 R /Last 19 0 R /Count 5 >>".into(),
        format!("<< /Title (Intro) /Parent 14 0 R /Next 16 0 R {} >>", dest(1)),
        format!("<< /Title <FEFF06270644064106350644002006270644062306480644> /Parent 14 0 R /Prev 15 0 R /Next 17 0 R {} >>", dest(2)),
        format!("<< /Title (Duplicate) /Parent 14 0 R /Prev 16 0 R /Next 18 0 R {} >>", dest(2)),
        format!(
            "<< /Title (Chapter 2) /Parent 14 0 R /Prev 17 0 R /Next 19 0 R /First 20 0 R /Last 20 0 R /Count 1 {} >>",
            dest(4)
        ),
        "<< /Title (Nowhere) /Parent 14 0 R /Prev 18 0 R >>".into(),
        format!("<< /Title (Section 2.1) /Parent 18 0 R {} >>", dest(3)),
    ]);
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

#[test]
fn splitting_at_top_level_bookmarks() {
    let document = reopen(&bookmarked_pdf());
    let parts = document.bookmark_parts();
    // Pages before the first entry go with it; the duplicate, the child, and
    // the entry that points nowhere add no part.
    assert_eq!(
        parts.iter().map(|(title, range)| (title.as_str(), range.clone())).collect::<Vec<_>>(),
        [("Intro", 0..2), ("الفصل الأول", 2..4), ("Chapter 2", 4..5)]
    );
    let ranges: Vec<_> = parts.into_iter().map(|(_, range)| range).collect();
    let files = document.split_ranges(&ranges).expect("splits");
    let texts: Vec<Vec<String>> = files
        .iter()
        .map(|bytes| {
            qpdf_check(bytes);
            texts(&reopen(bytes))
        })
        .collect();
    assert_eq!(texts, [vec!["Page 1", "Page 2"], vec!["Page 3", "Page 4"], vec!["Page 5"]]);

    // One top-level entry gives no split.
    assert_eq!(open("chrome/en-paragraphs").bookmark_parts().len(), 1);
}

#[test]
fn splitting_by_size_fills_each_file_up_to_the_limit() {
    let document = open("qpdf/rtl-book");
    let count = document.page_count();
    let size = |range: std::ops::Range<usize>| {
        document.extract_pages(&range.collect::<Vec<_>>()).expect("extracts").len()
    };
    let limit = size(0..2);
    let parts = document.split_by_size(limit).expect("splits");
    // In order, every page once, each part within the limit (or one page),
    // and none could have taken its next page.
    assert_eq!(parts.first().map(|(range, _)| range.start), Some(0));
    assert_eq!(parts.last().map(|(range, _)| range.end), Some(count));
    for window in parts.windows(2) {
        assert_eq!(window[0].0.end, window[1].0.start);
    }
    for (range, bytes) in &parts {
        assert!(bytes.len() <= limit || range.len() == 1, "{range:?}: {} > {limit}", bytes.len());
        assert_eq!(reopen(bytes).page_count(), range.len());
        if range.end < count {
            assert!(size(range.start..range.end + 1) > limit, "{range:?} could have held one more page");
        }
    }
    assert!(parts[0].0.len() >= 2, "the first two pages fit by construction");

    // A limit below any page: one page per file. Above the whole: one file.
    assert_eq!(document.split_by_size(1).expect("splits").len(), count);
    let whole = document.split_by_size(usize::MAX).expect("splits");
    assert_eq!(whole.len(), 1);
    qpdf_check(&whole[0].1);
}
