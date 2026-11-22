//! The catalog reader on the real corpus: files that state reading
//! preferences (written by qpdf, classic and with object streams) and every
//! other fixture, which states none.

mod common;

use common::manifest;
use safha_core::catalog::{CatalogHints, PageLayout, ReadingDirection, catalog_hints};
use safha_core::{Engine, PdfiumSource};

#[test]
fn reads_the_books_reading_preferences_however_they_are_stored() {
    let manifest = manifest();
    for id in ["rtl-book", "rtl-book-objstm"] {
        let fixture =
            manifest.fixtures.iter().find(|f| f.id == id).expect("fixture exists (regenerate the corpus)");
        let bytes = std::fs::read(fixture.file()).expect("reads");
        let expected = CatalogHints {
            direction: Some(ReadingDirection::RightToLeft),
            page_layout: Some(PageLayout::TwoPageRight),
        };
        assert_eq!(catalog_hints(&bytes), expected, "{id}");
        // And through a document, as the app gets it.
        let document = Engine::global(&PdfiumSource::Auto)
            .expect("PDFium is available")
            .open_file(&fixture.file(), None)
            .expect("opens");
        assert_eq!(document.catalog_hints(), expected, "{id}");
        assert_eq!(document.page_count(), 4);
    }
}

#[test]
fn every_other_fixture_states_no_preference_and_none_breaks_the_reader() {
    for fixture in manifest().fixtures.iter().filter(|f| !f.id.starts_with("rtl-book")) {
        let Ok(bytes) = std::fs::read(fixture.file()) else { continue };
        assert_eq!(catalog_hints(&bytes).direction, None, "{}", fixture.name());
    }
}
