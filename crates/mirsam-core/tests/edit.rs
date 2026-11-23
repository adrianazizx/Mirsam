//! Page edits as incremental updates (ADR 0014), on real files: every result
//! is reopened by PDFium and checked by `qpdf --check`, and the pages' text
//! shows where each page went.

mod common;

use std::process::Command;
use std::time::Instant;

use common::manifest;
use mirsam_core::edit::{PageEdit, page_edit_update};
use mirsam_core::{Document, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

/// The fixture's bytes and password.
fn fixture(name: &str) -> (Vec<u8>, Option<String>) {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|f| f.name() == name)
        .unwrap_or_else(|| panic!("fixture {name} missing: run `node fixtures/generate.mjs`"));
    (std::fs::read(fixture.file()).expect("fixture readable"), fixture.password.clone())
}

fn edited(bytes: &[u8], edit: &PageEdit) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out.extend(page_edit_update(bytes, edit).expect("the edit applies"));
    out
}

fn open(bytes: &[u8], password: Option<&str>) -> Document {
    engine().open_bytes(bytes.to_vec(), password).expect("PDFium reopens the edited file")
}

/// Each page's text, whitespace collapsed.
fn texts(document: &Document) -> Vec<String> {
    (0..document.page_count())
        .map(|i| {
            let text = document.page_text(i).expect("extracts").text();
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        })
        .collect()
}

/// `qpdf --check` on the bytes; qpdf is already needed to make the corpus.
fn qpdf_check(bytes: &[u8], password: Option<&str>) {
    let dir = std::env::temp_dir().join(format!("mirsam-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{}.pdf", bytes.len()));
    std::fs::write(&path, bytes).expect("write temp file");
    let mut command = Command::new("qpdf");
    if let Some(password) = password {
        command.arg(format!("--password={password}"));
    }
    let output = command.arg("--check").arg(&path).output().expect("qpdf runs (install qpdf)");
    let _ = std::fs::remove_file(&path);
    // Exit 3 is warnings only: still a valid file.
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "qpdf --check failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn rotating_turns_only_the_chosen_pages() {
    let (bytes, _) = fixture("qpdf/rotated");
    let before = open(&bytes, None);
    let sizes: Vec<_> = (0..4).map(|i| before.page_size(i).expect("size")).collect();

    let after_bytes = edited(&bytes, &PageEdit::Rotate { pages: vec![0, 3], quarter_turns: 1 });
    qpdf_check(&after_bytes, None);
    let after = open(&after_bytes, None);
    // Page 1 was at 0°, now 90°: its sides swap. Page 4 was at 270°, now 0°.
    let turned = after.page_size(0).expect("size");
    assert_eq!((turned.width, turned.height), (sizes[0].height, sizes[0].width));
    assert_eq!(after.page_size(1).expect("size"), sizes[1]);
    let back = after.page_size(3).expect("size");
    assert_eq!((back.width, back.height), (sizes[3].height, sizes[3].width));
    // Rotation does not change the text (ADR 0008).
    assert_eq!(texts(&after), texts(&before));
}

#[test]
fn deleting_moving_duplicating_and_inserting_reorder_the_pages() {
    let (bytes, _) = fixture("qpdf/page-labels");
    let original = texts(&open(&bytes, None));
    assert_eq!(original.len(), 3);
    let [a, b, c] = [&original[0], &original[1], &original[2]];

    let check = |edit: PageEdit, expected: &[&String]| {
        let out = edited(&bytes, &edit);
        qpdf_check(&out, None);
        let got = texts(&open(&out, None));
        let expected: Vec<String> = expected.iter().map(|s| (*s).clone()).collect();
        assert_eq!(got, expected, "{edit:?}");
    };
    check(PageEdit::Delete { pages: vec![1] }, &[a, c]);
    check(PageEdit::Move { pages: vec![2], to: 0 }, &[c, a, b]);
    check(PageEdit::Move { pages: vec![0, 1], to: 1 }, &[c, a, b]);
    check(PageEdit::Duplicate { pages: vec![0, 2] }, &[a, a, b, c, c]);

    let empty = String::new();
    check(PageEdit::InsertBlank { at: 1, width: 595.0, height: 842.0 }, &[a, &empty, b, c]);
    let with_blank = edited(&bytes, &PageEdit::InsertBlank { at: 3, width: 300.0, height: 200.0 });
    let size = open(&with_blank, None).page_size(3).expect("size");
    assert_eq!((size.width, size.height), (300.0, 200.0));
}

#[test]
fn edits_stack_and_dropping_the_last_update_undoes_it() {
    let (bytes, _) = fixture("qpdf/page-labels");
    let first = edited(&bytes, &PageEdit::Delete { pages: vec![0] });
    let second = edited(&first, &PageEdit::Rotate { pages: vec![0], quarter_turns: 2 });
    qpdf_check(&second, None);
    assert_eq!(open(&second, None).page_count(), 2);
    // Undo: the earlier revision is a prefix of the file.
    assert!(second.starts_with(&first) && first.starts_with(&bytes));
    assert_eq!(open(&second[..first.len()], None).page_count(), 2);
    assert_eq!(open(&second[..bytes.len()], None).page_count(), 3);
}

#[test]
fn the_outline_follows_its_pages() {
    // One page with outline entries; duplicate it and move the original last.
    let (bytes, _) = fixture("chrome/contract-two-column");
    let outline = open(&bytes, None).outline();
    assert!(!outline.is_empty(), "the fixture has an outline");
    let doubled = edited(&bytes, &PageEdit::Duplicate { pages: vec![0] });
    let moved = edited(&doubled, &PageEdit::Move { pages: vec![0], to: 1 });
    qpdf_check(&moved, None);
    let after = open(&moved, None).outline();
    assert_eq!(after.len(), outline.len());
    assert!(after.iter().all(|item| item.page == Some(1)), "{after:?}");
}

#[test]
fn encrypted_linearized_and_object_stream_files_stay_valid() {
    let (encrypted, password) = fixture("qpdf/encrypted");
    let password = password.as_deref();
    let out = edited(&encrypted, &PageEdit::InsertBlank { at: 0, width: 200.0, height: 200.0 });
    let out = edited(&out, &PageEdit::Rotate { pages: vec![1], quarter_turns: 1 });
    qpdf_check(&out, password);
    let document = open(&out, password);
    assert_eq!(document.page_count(), 2);
    assert_eq!(texts(&document)[1], texts(&open(&encrypted, password))[0]);

    for name in ["qpdf/linearized", "qpdf/rtl-book-objstm", "qpdf/rtl-book"] {
        let (bytes, _) = fixture(name);
        let before = texts(&open(&bytes, None));
        let out = edited(&bytes, &PageEdit::Move { pages: vec![0], to: before.len() - 1 });
        qpdf_check(&out, None);
        let mut expected = before.clone();
        expected.rotate_left(1);
        assert_eq!(texts(&open(&out, None)), expected, "{name}");
        // The reading preferences in the catalog are untouched.
        assert_eq!(open(&out, None).catalog_hints(), open(&bytes, None).catalog_hints(), "{name}");
    }
}

#[test]
fn a_thousand_page_document_edits_quickly() {
    let (bytes, _) = fixture("qpdf/bilingual-1000-pages");
    let started = Instant::now();
    let update = page_edit_update(&bytes, &PageEdit::Move { pages: vec![999], to: 0 }).expect("applies");
    let elapsed = started.elapsed();
    let mut out = bytes.clone();
    out.extend(&update);
    let document = open(&out, None);
    assert_eq!(document.page_count(), 1000);
    eprintln!("1,000 pages: move in {elapsed:?}, update {} bytes", update.len());
    // Generous for debug builds; the update is the new page list, not a copy of the file.
    assert!(elapsed.as_millis() < 2_000, "{elapsed:?}");
    assert!(update.len() < 64 * 1024);
}

#[test]
fn impossible_edits_are_refused() {
    let (bytes, _) = fixture("qpdf/page-labels");
    assert!(matches!(
        page_edit_update(&bytes, &PageEdit::Delete { pages: vec![0, 1, 2] }),
        Err(Error::InvalidEdit(_))
    ));
    assert!(matches!(
        page_edit_update(&bytes, &PageEdit::Rotate { pages: vec![3], quarter_turns: 1 }),
        Err(Error::PageOutOfRange { index: 3, count: 3 })
    ));
    assert!(matches!(
        page_edit_update(&bytes, &PageEdit::InsertBlank { at: 0, width: 0.0, height: 10.0 }),
        Err(Error::InvalidEdit(_))
    ));
    let (truncated, _) = fixture("qpdf/truncated");
    assert!(matches!(
        page_edit_update(&truncated, &PageEdit::Rotate { pages: vec![0], quarter_turns: 1 }),
        Err(Error::EditUnsupported(_))
    ));
}

#[test]
fn a_document_edits_undoes_and_redoes_through_its_history() {
    use mirsam_core::edit::EditHistory;

    // The encrypted fixture: each revision must reopen with the password.
    let (bytes, password) = fixture("qpdf/encrypted");
    let document = open(&bytes, password.as_deref());
    let mut history = EditHistory::new(document.bytes());

    let (edited, update) = document
        .with_page_edit(&PageEdit::InsertBlank { at: 1, width: 100.0, height: 100.0 })
        .expect("the edit applies");
    history.push(update);
    assert_eq!(edited.page_count(), 2);
    assert!(history.is_modified());

    let undone = edited.revision(history.undo().expect("undo")).expect("reopens");
    assert_eq!(undone.page_count(), 1);
    let redone = undone.revision(history.redo().expect("redo")).expect("reopens");
    assert_eq!(redone.page_count(), 2);
    assert_eq!(redone.page_size(1).expect("size").width, 100.0);
}
