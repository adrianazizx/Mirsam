//! The render service against real PDFium and the generated corpus: it must
//! return exactly the pixels a direct render produces, and page sizes must be
//! available for the whole 1,000-page document without loading pages.

mod common;

use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use common::manifest;
use safha_core::render::{Priority, RenderOutcome, RenderRequest, TileSource, Urgency};
use safha_core::{Document, Engine, PdfiumSource, RenderService, TileRequest};

fn engine() -> &'static Engine {
    Engine::global(&PdfiumSource::Auto).expect("PDFium is available (run scripts/fetch-pdfium.sh)")
}

fn thousand_pages() -> Arc<Document> {
    let manifest = manifest();
    let fixture = manifest
        .fixtures
        .iter()
        .find(|fixture| fixture.path.ends_with("bilingual-1000-pages.pdf"))
        .expect("the 1,000-page fixture is in the manifest");
    Arc::new(engine().open_file(&fixture.file(), None).expect("opens"))
}

fn render(service: &RenderService, page: usize, tile: TileRequest) -> RenderOutcome {
    let (tx, rx) = channel();
    let request = RenderRequest {
        document: 1,
        generation: 0,
        page,
        tile,
        priority: Priority::new(Urgency::Visible, 0.0),
    };
    service.submit(request, move |outcome| {
        let _ = tx.send(outcome);
    });
    rx.recv_timeout(Duration::from_secs(10)).expect("the service answers")
}

/// Pixel size of the whole page at `scale`, computed the way the UI does.
fn page_pixels(width: f32, scale: f32) -> u32 {
    (width * scale).ceil() as u32
}

#[test]
fn page_sizes_cover_the_whole_document_without_loading_pages() {
    let document = thousand_pages();
    let started = Instant::now();
    let sizes = document.page_sizes().expect("page sizes");
    let elapsed = started.elapsed();
    assert_eq!(sizes.len(), 1000);
    // Generous bound for a debug build on a slow CI machine; measured ~13 ms.
    assert!(elapsed < Duration::from_secs(2), "page_sizes took {elapsed:?}");
    for (index, size) in sizes.iter().enumerate().step_by(37) {
        assert_eq!(*size, document.page_size(index).expect("page size"), "page {index}");
        // A loaded page reports the same geometry the tiler validates against.
        assert_eq!(document.load_page(index).expect("loads").size(), *size, "page {index}");
    }
}

#[test]
fn service_tiles_match_direct_renders_pixel_for_pixel() {
    let document = thousand_pages();
    let service = RenderService::default();
    service.open_document(1, Arc::clone(&document) as Arc<dyn TileSource>);

    let scale = 2.6667_f32;
    let size = document.page_size(3).expect("size");
    let (width, height) = (page_pixels(size.width, scale), page_pixels(size.height, scale));
    // An interior tile and the clamped bottom-right edge tile.
    let edge_x = (width - 1) / 512 * 512;
    let edge_y = (height - 1) / 512 * 512;
    for tile in [
        TileRequest { scale, x: 512, y: 512, width: 512, height: 512 },
        TileRequest { scale, x: edge_x, y: edge_y, width: width - edge_x, height: height - edge_y },
    ] {
        let via_service = render(&service, 3, tile).expect("renders");
        let direct = document.render_tile(3, tile).expect("renders directly");
        assert_eq!(via_service.bitmap.as_rgba(), direct.as_rgba(), "{tile:?}");
        assert!(!via_service.from_cache);
        assert!(render(&service, 3, tile).expect("cached").from_cache);
    }
}

#[test]
fn whole_page_previews_render_at_ui_computed_sizes_for_every_page_shape() {
    let document = thousand_pages();
    let service = RenderService::default();
    service.open_document(1, Arc::clone(&document) as Arc<dyn TileSource>);
    let sizes = document.page_sizes().expect("page sizes");
    for scale in [0.25_f32, 0.375, 1.0 / 3.0] {
        for (page, size) in sizes.iter().enumerate().step_by(97) {
            let tile = TileRequest {
                scale,
                x: 0,
                y: 0,
                width: page_pixels(size.width, scale),
                height: page_pixels(size.height, scale),
            };
            let preview = render(&service, page, tile).unwrap_or_else(|err| panic!("page {page}: {err}"));
            assert_eq!((preview.bitmap.width(), preview.bitmap.height()), (tile.width, tile.height));
        }
    }
    let stats = service.stats();
    assert_eq!(stats.failed, 0);
    assert!(stats.page_loads <= stats.rendered);
}
