//! Text geometry on the displayed page: rotation and the search index cache.

mod common;

use std::sync::Arc;

use common::manifest;
use safha_core::search::{IndexCache, Query, SearchOptions};
use safha_core::view::ViewRect;
use safha_core::{Document, Engine, PdfiumSource};

fn open(name: &str) -> Document {
    let manifest = manifest();
    let fixture = manifest.fixtures.iter().find(|f| f.name() == name).expect("fixture exists");
    Engine::global(&PdfiumSource::Auto)
        .expect("PDFium is available")
        .open_file(&fixture.file(), None)
        .expect("opens")
}

/// Every glyph box of page `index`, in displayed coordinates.
fn glyph_boxes(document: &Document, index: usize) -> Vec<ViewRect> {
    let (text, view) = document.page_text_view(index).expect("extracts");
    text.blocks
        .iter()
        .flat_map(|block| &block.lines)
        .flat_map(|line| &line.spans)
        .filter_map(|span| span.rect)
        .map(|rect| view.rect(rect))
        .collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn glyph_boxes_follow_the_page_rotation() {
    let document = open("qpdf/rotated");
    let upright = document.page_size(0).expect("size");
    let boxes = glyph_boxes(&document, 0);
    assert!(boxes.len() > 50, "the page has text");
    for rect in &boxes {
        assert!(rect.x >= -1.0 && rect.y >= -1.0, "inside the page: {rect:?}");
        assert!(rect.x + rect.width <= upright.width + 1.0 && rect.y + rect.height <= upright.height + 1.0);
    }

    let (w, h) = (upright.width, upright.height);
    // Where a box on the upright page is displayed once the page is turned
    // clockwise by 90, 180, and 270 degrees.
    let turned: [fn(&ViewRect, f32, f32) -> ViewRect; 3] = [
        |r, _w, h| ViewRect { x: h - r.y - r.height, y: r.x, width: r.height, height: r.width },
        |r, w, h| ViewRect { x: w - r.x - r.width, y: h - r.y - r.height, width: r.width, height: r.height },
        |r, w, _h| ViewRect { x: r.y, y: w - r.x - r.width, width: r.height, height: r.width },
    ];
    for (offset, expected_of) in turned.iter().enumerate() {
        let page = offset + 1;
        let size = document.page_size(page).expect("size");
        if page % 2 == 1 {
            assert!(close(size.width, h) && close(size.height, w), "page {page} is displayed sideways");
        }
        // The text itself does not depend on the rotation.
        let upright_text = document.page_text(0).expect("extracts").text();
        assert_eq!(document.page_text(page).expect("extracts").text(), upright_text, "page {page}");
        // PDFium lists a rotated page's characters in a slightly different
        // order (its text page follows the display orientation), so the boxes
        // are compared as sets.
        let rotated = glyph_boxes(&document, page);
        assert_eq!(rotated.len(), boxes.len(), "page {page} has the same glyphs");
        // Where PDFium kept both a letter and its dots on one page, the box
        // covers both: it contains the other page's box.
        let covers = |a: &ViewRect, b: &ViewRect| {
            a.x <= b.x + 0.05
                && a.y <= b.y + 0.05
                && a.x + a.width >= b.x + b.width - 0.05
                && a.y + a.height >= b.y + b.height - 0.05
        };
        let missing: Vec<ViewRect> = boxes
            .iter()
            .map(|upright_box| expected_of(upright_box, w, h))
            .filter(|expected| !rotated.iter().any(|actual| covers(actual, expected)))
            .collect();
        // Chrome draws an Arabic letter's dots as a second glyph with the same
        // character, and PDFium drops one of the two; which one depends on the
        // rotation. Those letters' boxes then cover the dots instead of the
        // body (about 6% of this page's glyphs). Everything else must match.
        assert!(
            missing.len() * 100 <= boxes.len() * 8,
            "page {page}: {} of {} glyphs misplaced, e.g. {:?}",
            missing.len(),
            boxes.len(),
            missing.first()
        );
    }
}

#[test]
fn the_index_is_built_once_and_shared() {
    let document = Arc::new(open("chrome/contract-two-column"));
    let cache = Arc::new(IndexCache::new());
    assert!(cache.ready().is_none());
    let searches: Vec<_> = (0..4)
        .map(|_| {
            let (document, cache) = (Arc::clone(&document), Arc::clone(&cache));
            std::thread::spawn(move || cache.get_or_build(&document).expect("builds").expect("not cancelled"))
        })
        .collect();
    let indexes: Vec<_> = searches.into_iter().map(|search| search.join().expect("no panic")).collect();
    assert!(indexes.windows(2).all(|pair| Arc::ptr_eq(&pair[0], &pair[1])), "one build, shared");

    let query = Query::new("الإيجار", SearchOptions::default()).expect("valid");
    let (hits, truncated) = indexes[0].search_limited(&query, 1);
    assert_eq!(hits.len(), 1);
    assert!(truncated, "the contract mentions the lease more than once");
}

#[test]
fn a_cancelled_build_yields_nothing() {
    let document = open("qpdf/bilingual-1000-pages");
    let cache = IndexCache::new();
    cache.cancel();
    assert!(cache.get_or_build(&document).expect("no engine error").is_none());
    assert!(cache.ready().is_none());
}
