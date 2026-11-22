//! Search over real fixture text: normalization, highlight boxes, and the
//! spec's 500-page budget.

mod common;

use std::time::Instant;

use common::manifest;
use safha_core::search::{DocumentIndex, Query, SearchOptions};
use safha_core::{Document, Engine, PdfiumSource};

fn open(name: &str) -> Document {
    let manifest = manifest();
    let fixture = manifest.fixtures.iter().find(|f| f.name() == name).expect("fixture exists");
    Engine::global(&PdfiumSource::Auto)
        .expect("PDFium is available")
        .open_file(&fixture.file(), None)
        .expect("opens")
}

fn hits(document: &Document, query: &str, options: SearchOptions) -> Vec<String> {
    let page = document.page_text(0).expect("extracts");
    let query = Query::new(query, options).expect("valid");
    query
        .find_on_page(&page)
        .into_iter()
        .map(|hit| page.blocks[hit.block].lines[hit.line].text[hit.start..hit.end].to_string())
        .collect()
}

#[test]
fn finds_arabic_without_typing_hamza_or_tashkeel() {
    let plain = open("chrome/ar-sans");
    assert_eq!(hits(&plain, "الاسبوع", SearchOptions::default()), ["الأسبوع"]);

    let vocalized = open("chrome/ar-vocalized");
    assert_eq!(hits(&vocalized, "المدرسة", SearchOptions::default()), ["الْمَدْرَسَةِ"]);
}

#[test]
fn finds_numbers_in_either_digit_style() {
    let mixed = open("chrome/mixed-inline");
    assert_eq!(hits(&mixed, "1234", SearchOptions::default()), ["١٢٣٤"]);
    assert_eq!(hits(&mixed, "INV-2026-00417", SearchOptions::default()), ["INV-2026-00417"]);
}

#[test]
fn hits_carry_highlight_boxes_on_the_page() {
    let document = open("chrome/ar-plain");
    let size = document.page_size(0).expect("page size");
    let page = document.page_text(0).expect("extracts");
    let query = Query::new("التقرير", SearchOptions::default()).expect("valid");
    let found = query.find_on_page(&page);
    assert!(!found.is_empty());
    for hit in found {
        assert!(!hit.rects.is_empty());
        for rect in hit.rects {
            assert!(
                rect.left >= 0.0 && rect.right <= size.width && rect.bottom >= 0.0 && rect.top <= size.height
            );
        }
    }
}

#[test]
fn whole_document_search_over_500_pages() {
    let document = open("qpdf/bilingual-1000-pages");
    // Index 500 pages: the budget's size.
    let started = Instant::now();
    let pages: Vec<_> = (0..500).map(|i| document.page_text(i).expect("extracts")).collect();
    let indexed = started.elapsed();

    let query = Query::new("الايجار", SearchOptions::default()).expect("valid");
    let started = Instant::now();
    let count: usize = pages.iter().map(|page| query.find_on_page(page).len()).sum();
    let searched = started.elapsed();

    println!("500 pages: index {indexed:?}, search {searched:?}, {count} hits");
    // The contract page has "الإيجار" in two blocks; every other page is a contract page.
    assert!(count >= 250, "{count} hits");
    // Spec budget (release build, mid-range laptop): ≤ 1 s after the first index.
    // Debug builds are ~10x slower, so only the release run is held to it.
    if !cfg!(debug_assertions) {
        assert!(searched.as_millis() <= 1000, "search took {searched:?}");
    }
}

#[test]
fn index_build_can_be_cancelled() {
    let document = open("qpdf/bilingual-1000-pages");
    let built = DocumentIndex::build(&document, || true).expect("no error");
    assert!(built.is_none());
}
