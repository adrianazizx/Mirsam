//! Search index cost on a document: build time, memory, and search time.
//!
//! Usage: `cargo run --release -p safha-core --example search_bench -- file.pdf [query...]`
//! (release builds need `SAFHA_PDFIUM_DIR`; see CLAUDE.md).
//!
//! Memory is the process's resident size before and after building, read with
//! `ps`, so it includes allocator slack: an upper bound on the index itself.

use std::time::Instant;

use safha_core::search::{DocumentIndex, Query, SearchOptions};
use safha_core::{Engine, PdfiumSource};

fn rss_kb() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: search_bench <file.pdf> [query...]")?;
    let queries: Vec<String> = args.collect();
    let queries = if queries.is_empty() {
        vec!["Agreement".to_string(), "الإيجار".to_string(), "4,500".to_string()]
    } else {
        queries
    };

    let engine = Engine::global(&PdfiumSource::Auto)?;
    let document = engine.open_file(std::path::Path::new(&path), None)?;
    let before = rss_kb();
    let started = Instant::now();
    let index = DocumentIndex::build(&document, || false)?.ok_or("cancelled")?;
    let built = started.elapsed();
    let after = rss_kb();
    println!(
        "{} pages indexed in {:.0} ms; resident {} -> {} MB",
        index.page_count(),
        built.as_secs_f64() * 1000.0,
        before.map_or("?".into(), |kb| format!("{:.0}", kb as f64 / 1024.0)),
        after.map_or("?".into(), |kb| format!("{:.0}", kb as f64 / 1024.0)),
    );
    for query in queries {
        let compiled = Query::new(&query, SearchOptions::default())?;
        let started = Instant::now();
        let (hits, truncated) = index.search_limited(&compiled, 5_000);
        println!(
            "{query:?}: {} hits{} in {:.1} ms",
            hits.len(),
            if truncated { "+" } else { "" },
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
