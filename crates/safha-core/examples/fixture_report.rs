//! How well text extraction reproduces the fixture ground truth (docs/SPEC.md 6.1).
//!
//! For each fixture, counts the expected logical-order blocks that appear
//! verbatim (whitespace collapsed, NFC) in
//!   raw    — PDFium's own page text, the Milestone 0 baseline, and
//!   ours   — `Document::page_text`, our geometry + bidi reconstruction,
//! plus, for ours, how many match once spaces are ignored (separating word-space
//! errors from ordering errors).
//!
//!   cargo run -p safha-core --example fixture_report [-- --verbose]

#[path = "../tests/common/mod.rs"]
mod common;

use safha_core::text::TextQuality;
use safha_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let verbose = std::env::args().any(|arg| arg == "--verbose");
    let engine = Engine::global(&PdfiumSource::Auto)?;
    let manifest = common::manifest();

    println!("{:<30} {:>9} {:>9} {:>10}  quality", "fixture", "raw", "ours", "no-spaces");
    let (mut raw_total, mut ours_total, mut loose_total, mut blocks_total) = (0, 0, 0, 0);
    for fixture in manifest.fixtures.iter().filter(|f| !f.damaged && !f.expected.is_empty()) {
        let document = engine.open_file(&fixture.file(), fixture.password.as_deref())?;
        let raw = common::comparable(&document.page_text_raw(0)?);
        let page = document.page_text(0)?;
        let ours = common::comparable(&page.text());
        let ours_loose = common::without_spaces(&ours);

        let (mut raw_found, mut ours_found, mut loose_found) = (0, 0, 0);
        for block in &fixture.expected {
            let expected = common::comparable(&block.text);
            raw_found += usize::from(raw.contains(&expected));
            let found = ours.contains(&expected);
            ours_found += usize::from(found);
            loose_found += usize::from(ours_loose.contains(&common::without_spaces(&expected)));
            if verbose && !found {
                println!("    missing [{}] {expected}", block.dir);
            }
        }
        if verbose {
            println!("    --- ours:\n{}\n    ---", page.text());
        }
        let n = fixture.expected.len();
        raw_total += raw_found;
        ours_total += ours_found;
        loose_total += loose_found;
        blocks_total += n;
        let quality = match page.quality {
            TextQuality::Good => "good".to_string(),
            TextQuality::NoText => "no text".to_string(),
            TextQuality::Suspect { suspicious_glyphs, .. } => format!("SUSPECT ({suspicious_glyphs} glyphs)"),
        };
        println!(
            "{:<30} {:>4}/{:<4} {:>4}/{:<4} {:>5}/{:<4}  {quality}",
            fixture.name(),
            raw_found,
            n,
            ours_found,
            n,
            loose_found,
            n
        );
    }
    println!(
        "\nverbatim blocks: raw {raw_total}/{blocks_total}, ours {ours_total}/{blocks_total} \
         ({loose_total}/{blocks_total} ignoring spaces)"
    );
    Ok(())
}
