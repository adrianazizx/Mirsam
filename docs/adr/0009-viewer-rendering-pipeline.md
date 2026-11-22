# 0009 — Viewer rendering pipeline: one render thread, generations, and an imperative viewer

Status: Accepted. The frame and blank-time budgets were measured with a visible window in
session 4; that run changed the tile transport and the zoom gesture (ADR 0011). Date: 2026-11-22.

## Context

M1 replaces the single-page canvas with a continuous, virtualized, tiled viewer (spec §5.1, §9
"Rendering design"). Constraints from earlier records: PDFium is single-threaded per process
(ADR 0002); tiles travel as raw bytes over Tauri commands (ADR 0004). Budgets (§11): 60 fps, no page
blank for more than 100 ms, < 500 MB on the 1,000-page file, first page ≤ 300 ms.

## Decision

### Core: `safha_core::render::RenderService`

1. **One render thread** (`safha-render`) owns every render, fed by one queue. No pool of threads
   (no parallelism is available over one PDFium) and no pool of processes yet: with parsed pages
   kept open, a fit-width A4 page costs about 4 ms of rendering (24 tiles of 512 px at 0.15–0.17 ms).
2. **Priority**: placeholder previews, then visible tiles, then on-screen thumbnails, then
   prefetch; within a class, nearest to the viewport centre; then arrival order. Identical requests merge into one render, queued or
   running. The queue is bounded (4,096); overflow is cancelled, not queued.
3. **Cancellation by viewport generation.** The UI calls `update_viewport(document, generation,
   keep)` as the viewport moves: kept requests are re-prioritized, every other queued request from an
   older generation is cancelled, and requests that arrive already stale are cancelled on arrival. A
   render already running finishes and is cached. Every caller gets exactly one answer, including
   on close, invalidation, and shutdown. Generations start from the clock, so they stay monotonic
   across tab switches and UI reloads.
4. **Cache**: byte-capped LRU keyed by (document, revision, page, scale, rectangle). The service owns
   the revision; `invalidate_document` (for M2/M3 edits) discards old renders. **Default 64 MiB**:
   measured resident memory is ~96 MB at 64 MiB but ~300 MB at 128 MiB and ~475 MB at 256 MiB, as
   the allocator stops reusing freed bitmaps (`examples/render_bench.rs`). `SAFHA_TILE_CACHE_MB`
   overrides it.
5. **Parsed pages stay open** on the render thread (4 by default); a `LoadedPage` holds an
   `Arc<Document>`, so closing a tab cannot free a page mid-render. PDFium renders directly into our
   buffer. Re-parsing per tile had cost 3× (0.52 vs 0.17 ms per tile).

### Shell

6. `render_tile` keeps ADR 0004's raw response; the header's spare word becomes flags (bit 0: served
   from cache). Requests carry a **lane** (`main` or `thumbnails`), registered with the service as
   two documents sharing one `Arc<Document>`, so each has its own viewport generation: scrolling
   the thumbnail strip never cancels the page view's tiles, nor the reverse (ADR 0010). Cancellation is the error code `cancelled`. Page sizes are sent as f64 so the UI's
   f32 pixel arithmetic matches the engine's exactly. The old `render_page`/`page_size` commands,
   which bypassed the queue, are gone.

### UI: `src/viewer/`

7. **An imperative viewer** (`controller.ts`) updated synchronously in the scroll event; React owns
   only the container. Pages are mounted one viewport beyond the visible area; full-resolution tiles
   are requested a quarter viewport beyond.
8. **Never blank**: each page first gets a whole-page preview at 0.25 × dpr px/pt, under its
   512-device-px tiles. Tiles are positioned in percentages, so during a zoom the old ones stretch
   until new ones arrive; tile requests pause until 140 ms after the last zoom event. Discarded
   canvases are shrunk to 0 × 0 to release their memory.
9. **Layouts**: continuous, single, two-page spread, and book (cover on the recto side), with page
   order an explicit setting — left-to-right or right-to-left — independent of UI language (spec
   §6.5). Fit-width uses the median row width, so an occasional landscape page scrolls sideways
   instead of shrinking every page.
10. **Commands** may take an argument (`view.goToPage {page}`); the page field accepts Arabic-Indic
    digits. Next/previous page step by rows, so a spread moves two pages.
11. **Continuous gestures are not registry commands.** Pinch and Ctrl/Cmd+wheel zoom are handled by
    the viewer itself, anchored at the cursor; the discrete zoom commands (keyboard, buttons, palette)
    remain. A gesture has a continuous parameter and no meaningful "title"; forcing it through the
    registry would add latency to the one path that has to track the fingers. This is the only
    exception to "every action is a command" (CLAUDE.md).

### Verification

12. `scripts/scroll-benchmark.sh` runs the real shell on the 1,000-page file: 40 random jumps timed
    by viewer events, then frame-driven phases (4,000 and 12,000 px/s scrolls, a 30,000 px/s fling,
    scrollbar jumps, pinch in/out, fit width), recording frame gaps, per-page blank time, and shell and
    webview memory. `scripts/check-scroll-benchmark.mjs` asserts p95 frame interval ≤ 17.5 ms, ≤ 1 %
    of frames over 20 ms, no page blank > 100 ms, memory < 500 MB — and declares the run **invalid**
    if the window was not visible, because a hidden or locked WKWebView gets no animation frames.

## Measured (Apple-silicon MacBook, 1280 × 820 window, dpr 2)

| | Debug | Release | Budget |
|---|---|---|---|
| First page after open: preview / sharp | 28–43 / 52–67 ms | 15–29 / 29–41 ms | ≤ 300 ms ✓ |
| 1,000 page sizes (engine) | — | 8.6–11.3 ms | — |
| Shell RSS after 40 random jumps | 293 MB | 272 MB | < 500 MB ✓ |
| Frame pacing while scrolling and zooming | see ADR 0011 | see ADR 0011 | 60 fps |
| Longest blank page while scrolling | see ADR 0011 | see ADR 0011 | ≤ 100 ms |

The frame-driven phases could not run here: the screen was locked for the whole measurement
window. ADR 0011 has the visible-window numbers.

## Open

- ~~Blank time on random jumps (hidden window).~~ Resolved. The 210–260 ms stalls happened only
  in a hidden window. With a visible window the debug build showed a preview within 27 ms, and
  after ADR 0011 the release build does so within 16 ms.
- ~~WebKit's memory.~~ Confirmed with a visible window (above 1 GB). Traced to tile buffers handed
  to JavaScript, and fixed by delivering tiles as images (ADR 0011). Whether the 500 MB budget
  covers WebKit's own processes is still open.
- `/ViewerPreferences /Direction` is not read yet (pdfium-render exposes it only through an `unsafe`
  call); page order defaults to left-to-right.
- Windows and Linux, and a physical trackpad pinch, are untested.
