//! Runs the Milestone 0 engine surface over the whole fixture corpus: every
//! producer's output must open, report the right page count, and render ink.

mod common;

use std::time::Instant;

use common::manifest;
use safha_core::{Bitmap, Engine, Error, PdfiumSource};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

/// Fraction of pixels that are not near-white.
fn ink_ratio(bitmap: &Bitmap) -> f64 {
    let (pixels, _) = bitmap.as_rgba().as_chunks::<4>();
    let ink = pixels.iter().filter(|px| px[0] < 200 || px[1] < 200 || px[2] < 200).count();
    ink as f64 / pixels.len() as f64
}

#[test]
fn every_readable_fixture_opens_and_renders() {
    let manifest = manifest();
    assert!(manifest.fixtures.len() >= 10, "corpus looks incomplete");

    for fixture in manifest.fixtures.iter().filter(|fixture| !fixture.damaged) {
        let name = fixture.name();
        let document = engine()
            .open_file(&fixture.file(), fixture.password.as_deref())
            .unwrap_or_else(|err| panic!("{name}: failed to open: {err}"));

        if let Some(pages) = fixture.pages {
            assert_eq!(document.page_count(), pages, "{name}: page count");
        }
        let size = document.page_size(0).unwrap_or_else(|err| panic!("{name}: page size: {err}"));
        // Every fixture is A4 portrait.
        assert!((size.width - 595.0).abs() < 2.0 && (size.height - 842.0).abs() < 2.0, "{name}: {size:?}");

        let bitmap = document.render_page(0, 1.0).unwrap_or_else(|err| panic!("{name}: render: {err}"));
        let ink = ink_ratio(&bitmap);
        assert!((0.002..0.5).contains(&ink), "{name}: implausible ink ratio {ink:.4}");

        if !fixture.expected.is_empty() {
            let text = document.page_text_raw(0).unwrap_or_else(|err| panic!("{name}: text: {err}"));
            assert!(!text.trim().is_empty(), "{name}: no extractable text");
        }
    }
}

#[test]
fn encrypted_fixture_needs_its_password() {
    let manifest = manifest();
    let fixture = manifest.fixtures.iter().find(|f| f.id == "encrypted").expect("encrypted fixture exists");

    let without = engine().open_file(&fixture.file(), None);
    assert!(matches!(without, Err(Error::PasswordRequired)), "{without:?}");
    let wrong = engine().open_file(&fixture.file(), Some("not-the-password"));
    assert!(matches!(wrong, Err(Error::PasswordRequired)), "{wrong:?}");
    assert!(engine().open_file(&fixture.file(), fixture.password.as_deref()).is_ok());
}

#[test]
fn damaged_fixture_fails_cleanly_or_recovers() {
    let manifest = manifest();
    for fixture in manifest.fixtures.iter().filter(|fixture| fixture.damaged) {
        // PDFium may rebuild the cross-reference table and recover some pages, or
        // give up. Either is acceptable; crashing or hanging is not.
        match engine().open_file(&fixture.file(), None) {
            Ok(document) => {
                for index in 0..document.page_count() {
                    let _ = document.render_page(index, 0.5);
                }
            }
            Err(Error::InvalidPdf | Error::Pdfium(_)) => {}
            Err(other) => panic!("{}: unexpected error {other}", fixture.name()),
        }
    }
}

#[test]
fn thousand_page_document_opens_without_touching_every_page() {
    let manifest = manifest();
    let fixture =
        manifest.fixtures.iter().find(|f| f.id == "bilingual-1000-pages").expect("1,000-page fixture exists");

    let started = Instant::now();
    let document = engine().open_file(&fixture.file(), None).expect("opens");
    let opened = started.elapsed();
    assert_eq!(document.page_count(), 1000);

    let started = Instant::now();
    let last = document.render_page(999, 1.0).expect("renders the last page");
    let rendered = started.elapsed();
    assert!(ink_ratio(&last) > 0.002);

    println!("1000-page fixture: open {opened:?}, first render of page 1000 {rendered:?}");
    // Generous ceiling (debug build, CI runners): opening must not scale with page count.
    assert!(opened.as_millis() < 1000, "open took {opened:?}");
}
