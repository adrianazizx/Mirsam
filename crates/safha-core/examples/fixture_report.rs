//! Baseline for Milestone 1's bidi-aware extraction (docs/SPEC.md section 6.1).
//!
//! For each fixture, checks whether PDFium's *raw* page text already contains
//! each expected block in logical order. Where it does not, the producer stored
//! glyphs in visual order (or split/merged runs) and our own reordering has to
//! fix it. This reports; it asserts nothing.
//!
//!   cargo run -p safha-core --example fixture_report

#[path = "../tests/common/mod.rs"]
mod common;

use safha_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let manifest = common::manifest();

    println!("{:<32} {:>9}  first mismatch (expected logical text)", "fixture", "verbatim");
    let (mut found_total, mut blocks_total) = (0, 0);
    for fixture in manifest.fixtures.iter().filter(|f| !f.damaged && !f.expected.is_empty()) {
        let document = engine.open_file(&fixture.file(), fixture.password.as_deref())?;
        let raw = common::normalize_whitespace(&document.page_text_raw(0)?);
        let missing: Vec<&str> = fixture
            .expected
            .iter()
            .map(|block| block.text.as_str())
            .filter(|text| !raw.contains(&common::normalize_whitespace(text)))
            .collect();
        let found = fixture.expected.len() - missing.len();
        found_total += found;
        blocks_total += fixture.expected.len();
        let first: String = missing.first().map(|text| text.chars().take(48).collect()).unwrap_or_default();
        println!("{:<32} {:>4}/{:<4}  {}", fixture.name(), found, fixture.expected.len(), first);
    }
    println!("\n{found_total}/{blocks_total} expected blocks appear verbatim in PDFium's raw text.");
    Ok(())
}
