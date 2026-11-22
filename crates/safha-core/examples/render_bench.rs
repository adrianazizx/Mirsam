//! Throughput and memory of the render service, without a webview.
//!
//!   cargo run -p safha-core --release --example render_bench -- <in.pdf> [cache-MB=64]
//!
//! Renders a low-resolution preview of every page, then full-resolution 512 px
//! tiles of a run of pages at a HiDPI fit-width scale, through one
//! `RenderService` exactly as the viewer requests them, and reports render
//! times, cache behaviour, and the process's resident memory once the tile
//! cache is full.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::Instant;

use safha_core::render::{Priority, RenderConfig, RenderRequest, TileSource, Urgency};
use safha_core::{Engine, PdfiumSource, RenderService, TileRequest};

const TILE: u32 = 512;

fn rss_mb() -> Option<f64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kb: f64 = String::from_utf8_lossy(&output.stdout).trim().parse().ok()?;
    Some(kb / 1024.0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = args.first().ok_or("usage: render_bench <in.pdf> [cache-MB=64]")?;
    let cache_mb: usize = args.get(1).map(|s| s.parse()).transpose()?.unwrap_or(64);

    let document = Arc::new(Engine::global(&PdfiumSource::Auto)?.open_file(Path::new(input), None)?);
    let started = Instant::now();
    let sizes = document.page_sizes()?;
    println!("{} pages, sizes read in {:.1} ms", sizes.len(), started.elapsed().as_secs_f64() * 1e3);
    println!("rss at start: {:.0} MB", rss_mb().unwrap_or(f64::NAN));

    let config = RenderConfig { cache_bytes: cache_mb * 1024 * 1024, ..RenderConfig::default() };
    let service = RenderService::new(config);
    service.open_document(1, Arc::clone(&document) as Arc<dyn TileSource>);

    // Submits every request, then waits for all answers (the queue orders them).
    let run = |requests: Vec<(usize, TileRequest)>, urgency: Urgency| -> Result<f64, String> {
        let (tx, rx) = channel();
        let count = requests.len();
        let started = Instant::now();
        for (index, (page, tile)) in requests.into_iter().enumerate() {
            let tx = tx.clone();
            let request = RenderRequest {
                document: 1,
                generation: 0,
                page,
                tile,
                priority: Priority::new(urgency, index as f32),
            };
            service.submit(request, move |outcome| {
                let _ = tx.send(outcome.map(|_| ()).map_err(|err| err.to_string()));
            });
        }
        for _ in 0..count {
            rx.recv().map_err(|err| err.to_string())??;
        }
        Ok(started.elapsed().as_secs_f64() * 1e3)
    };

    let preview_scale = 0.5_f32;
    let previews: Vec<_> = sizes
        .iter()
        .enumerate()
        .map(|(page, size)| {
            let width = (size.width * preview_scale).ceil() as u32;
            let height = (size.height * preview_scale).ceil() as u32;
            (page, TileRequest { scale: preview_scale, x: 0, y: 0, width, height })
        })
        .collect();
    let ms = run(previews, Urgency::Placeholder)?;
    let stats = service.stats();
    println!(
        "previews at {preview_scale} px/pt: {} in {ms:.0} ms ({:.2} ms each, engine {:.2} ms each); cache {:.0} MB in {} entries",
        stats.rendered,
        ms / stats.rendered as f64,
        stats.render_micros as f64 / 1e3 / stats.rendered as f64,
        stats.cache_bytes as f64 / 1048576.0,
        stats.cache_entries
    );
    println!("rss after previews: {:.0} MB", rss_mb().unwrap_or(f64::NAN));

    // Fit width on a 2x display in a 1280 px window: about 3.06 px/pt.
    let scale = 3.062_f32;
    for (label, pages) in [("tiles, 40 pages", 0..40usize), ("tiles, 120 more pages", 40..160usize)] {
        let before = service.stats();
        let mut tiles = Vec::new();
        for page in pages {
            let size = sizes[page];
            let (width, height) = ((size.width * scale).ceil() as u32, (size.height * scale).ceil() as u32);
            for y in (0..height).step_by(TILE as usize) {
                for x in (0..width).step_by(TILE as usize) {
                    let tile =
                        TileRequest { scale, x, y, width: TILE.min(width - x), height: TILE.min(height - y) };
                    tiles.push((page, tile));
                }
            }
        }
        let ms = run(tiles, Urgency::Visible)?;
        let after = service.stats();
        let rendered = after.rendered - before.rendered;
        println!(
            "{label} at {scale} px/pt: {rendered} tiles in {ms:.0} ms ({:.2} ms per tile, {:.1} ms per page); \
             {} page loads; cache {:.0} MB in {} entries, {} evictions",
            ms / rendered as f64,
            ms / (rendered as f64 / 24.0),
            after.page_loads - before.page_loads,
            after.cache_bytes as f64 / 1048576.0,
            after.cache_entries,
            after.cache_evictions
        );
        println!("rss: {:.0} MB", rss_mb().unwrap_or(f64::NAN));
    }
    Ok(())
}
