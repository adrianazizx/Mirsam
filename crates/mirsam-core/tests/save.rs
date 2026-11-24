//! Saving (ADR 0015): what reaches the disk after edits.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::manifest;
use mirsam_core::edit::PageEdit;
use mirsam_core::save::{SaveMode, save};
use mirsam_core::{Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn fixture(name: &str) -> (PathBuf, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|f| f.name() == name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (fixture.file(), fixture.password.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mirsam-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

fn qpdf(args: &[&str]) -> std::process::Output {
    Command::new("qpdf").args(args).output().expect("qpdf runs (install qpdf)")
}

fn qpdf_check(path: &std::path::Path, password: Option<&str>) {
    let password = password.map(|p| format!("--password={p}"));
    let mut args: Vec<&str> = password.iter().map(String::as_str).collect();
    let path = path.to_string_lossy();
    args.extend(["--check", &path]);
    let output = qpdf(&args);
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check failed:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn a_deleted_page_is_gone_from_the_saved_file() {
    let (source, _) = fixture("qpdf/page-labels");
    // The first page's content stream, compressed, as it sits in the file.
    let raw = qpdf(&["--show-object=6", "--raw-stream-data", &source.to_string_lossy()]).stdout;
    assert!(raw.len() > 400, "the fixture's first page content is object 6");

    let document = engine().open_file(&source, None).expect("opens");
    let (edited, _) = document.with_page_edit(&PageEdit::Delete { pages: vec![0] }).expect("edits");
    // Before saving, the deleted page is still in the bytes (an earlier revision).
    assert!(edited.bytes().windows(300).any(|w| w == &raw[100..400]));

    let target = scratch("deleted.pdf");
    assert_eq!(save(&edited, &target).expect("saves"), SaveMode::Rewritten);
    qpdf_check(&target, None);
    let saved = std::fs::read(&target).expect("read");
    assert!(!saved.windows(300).any(|w| w == &raw[100..400]), "the deleted page survived the save");
    assert_eq!(engine().open_file(&target, None).expect("reopens").page_count(), 2);
}

#[test]
fn an_encrypted_file_stays_encrypted() {
    let (source, password) = fixture("qpdf/encrypted");
    let password = password.as_deref();
    let document = engine().open_file(&source, password).expect("opens");
    let (edited, _) =
        document.with_page_edit(&PageEdit::Rotate { pages: vec![0], quarter_turns: 1 }).expect("edits");
    let target = scratch("encrypted.pdf");
    save(&edited, &target).expect("saves");
    qpdf_check(&target, password);
    assert!(matches!(engine().open_file(&target, None), Err(Error::PasswordRequired)));
    let reopened = engine().open_file(&target, password).expect("opens with the password");
    let size = reopened.page_size(0).expect("size");
    assert!(size.width > size.height, "the rotation was saved");
}

/// A one-page PDF with a signature field whose value is a (dummy) signature:
/// enough for PDFium to count the document as signed.
fn signed_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /SigFlags 3 >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /Rect [0 0 0 0] /P 3 0 R /V 5 0 R >>",
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> >>",
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

#[test]
fn a_signed_file_is_saved_incrementally() {
    let original = signed_pdf();
    let document = engine().open_bytes(original.clone(), None).expect("opens");
    assert!(document.is_signed());
    let (edited, _) = document
        .with_page_edit(&PageEdit::InsertBlank { at: 1, width: 200.0, height: 200.0 })
        .expect("edits");
    let target = scratch("signed.pdf");
    assert_eq!(save(&edited, &target).expect("saves"), SaveMode::Incremental);
    let saved = std::fs::read(&target).expect("read");
    // Every byte the signature covers is still there, unchanged.
    assert!(saved.starts_with(&original));
    assert_eq!(engine().open_file(&target, None).expect("reopens").page_count(), 2);
}

#[test]
fn saving_over_the_open_file_replaces_it() {
    let (source, _) = fixture("qpdf/page-labels");
    let target = scratch("in place.pdf");
    std::fs::copy(&source, &target).expect("copy");
    let document = engine().open_file(&target, None).expect("opens");
    let (edited, _) = document.with_page_edit(&PageEdit::Move { pages: vec![2], to: 0 }).expect("edits");
    save(&edited, &target).expect("saves");
    let reopened = engine().open_file(&target, None).expect("reopens");
    assert_eq!(reopened.page_count(), 3);
    // The page that was third is now first.
    let first = reopened.page_text(0).expect("text").text();
    let third = document.page_text(2).expect("text").text();
    assert_eq!(first, third);
}
