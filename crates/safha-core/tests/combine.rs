//! Pages across files (ADR 0016): extract, split, and insert from another PDF.

mod common;

use std::process::Command;

use common::manifest;
use safha_core::edit::EditHistory;
use safha_core::{Document, Engine, Error, PdfiumSource};

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
    let path = std::env::temp_dir().join(format!("safha-combine-{}-{}.pdf", std::process::id(), bytes.len()));
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
