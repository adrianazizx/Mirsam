//! Measures M6 output on PDFs (ADR 0050): page numbers stamped on every page
//! (time, update size), and compression (time, sizes before and after).
//!
//! `cargo run --release -p safha-core --example m6_report -- a.pdf [b.pdf …]`
//! (release builds need `SAFHA_PDFIUM_DIR`).

use std::time::Instant;

use safha_core::compress::CompressOptions;
use safha_core::fonts::FontFamily;
use safha_core::stamp::{Band, Digits, Edge, Stamp, StampKind};
use safha_core::{Engine, PdfiumSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::global(&PdfiumSource::Auto)?;
    println!(
        "file\tpages\tstamp ms\tstamp bytes\tcompress ms\tbefore\tafter\timages reduced\tobjects removed"
    );
    for path in std::env::args().skip(1) {
        let document = engine.open_bytes(std::fs::read(&path)?, None)?;
        let pages: Vec<usize> = (0..document.page_count()).collect();
        let band = Band {
            edge: Edge::Bottom,
            start: String::new(),
            center: "صفحة {page} من {pages}".into(),
            end: String::new(),
            rtl: true,
            family: FontFamily::Sans,
            size: 10.0,
            color: [0.0; 3],
            margin: 30.0,
            digits: Digits::ArabicIndic,
            first_number: 1,
        };
        let started = Instant::now();
        let (_, update) =
            document.with_stamp(&Stamp { pages: pages.clone(), kind: StampKind::Band(band) })?;
        let stamped = started.elapsed().as_millis();
        let started = Instant::now();
        let compressed = document.compressed(CompressOptions::default());
        let compress_ms = started.elapsed().as_millis();
        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned());
        match compressed {
            Ok((_, report)) => println!(
                "{}\t{}\t{stamped}\t{}\t{compress_ms}\t{}\t{}\t{}\t{}",
                name.unwrap_or_default(),
                pages.len(),
                update.len(),
                report.before,
                report.after,
                report.images_reduced,
                report.objects_removed
            ),
            Err(err) => println!(
                "{}\t{}\t{stamped}\t{}\t-\t{err}",
                name.unwrap_or_default(),
                pages.len(),
                update.len()
            ),
        }
    }
    Ok(())
}
