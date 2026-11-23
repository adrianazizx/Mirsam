//! Search index cost on a document: build time, memory, and search time.
//!
//! Usage: `cargo run --release -p safha-core --example search_bench -- file.pdf [--budget-ms N] [query...]`
//! (release builds need `SAFHA_PDFIUM_DIR`; see CLAUDE.md).
//!
//! `--budget-ms N` fails (exit status 1) when a search takes longer than `N`
//! milliseconds once the index is built: spec section 11 gives 1,000 ms for
//! 500 pages. Each query runs three times and its slowest run is judged.
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
    let path = args.next().ok_or("usage: search_bench <file.pdf> [--budget-ms N] [query...]")?;
    let mut budget_ms: Option<f64> = None;
    let mut queries: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--budget-ms" {
            budget_ms = Some(args.next().ok_or("--budget-ms needs a number")?.parse()?);
        } else {
            queries.push(arg);
        }
    }
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
    let mut over = Vec::new();
    for query in queries {
        let compiled = Query::new(&query, SearchOptions::default())?;
        let mut slowest = 0.0_f64;
        let mut found = (0, false);
        for _ in 0..3 {
            let started = Instant::now();
            let (hits, truncated) = index.search_limited(&compiled, 5_000);
            slowest = slowest.max(started.elapsed().as_secs_f64() * 1000.0);
            found = (hits.len(), truncated);
        }
        println!(
            "{query:?}: {} hits{} in {slowest:.1} ms (slowest of 3)",
            found.0,
            if found.1 { "+" } else { "" }
        );
        if budget_ms.is_some_and(|budget| slowest > budget) {
            over.push(format!("{query:?} took {slowest:.1} ms"));
        }
    }
    if let Some(budget) = budget_ms {
        if !over.is_empty() {
            eprintln!("over the {budget} ms search budget: {}", over.join("; "));
            std::process::exit(1);
        }
        println!("every search within {budget} ms");
    }
    Ok(())
}
