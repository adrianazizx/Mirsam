# 0011 — Tiles as images over a custom scheme, and compositor-only zoom gestures

Status: Accepted (Milestone 1). Supersedes ADR 0004's choice of transport (raw bytes in a command
response); ADR 0004's other findings stand. Date: 2026-11-22.

## Context

The scroll benchmark (ADR 0009 §12) first ran with a visible window this session. Debug build,
1,000-page file:

- **Blank time:** none. The 210–260 ms stalls ADR 0009 saw happened only in a hidden window.
- **Pinch zoom:** 24–48% of frames took over 20 ms.
- **Scrolling:** 1–2% of frames took over 20 ms.
- **Memory:** the shell and WebKit together reached **1.37 GB**, and WebKit's share stayed above
  1 GB after 15 s idle.

At the end only 3 pages and 17 tiles were on screen, about 50 MB of pixels. Sampling each process
showed that the WebKit content process holds the memory; the GPU process stays under 85 MB.

## Experiment: where the webview's memory goes

`apps/desktop/src/spikes/tileMemory.ts` delivers the same 2,000 distinct 512 px tiles (2 GB of
pixels), one fresh launch per variant, and reads WebKit's resident memory before and after. Each
row is 2–3 runs.

| Variant | WebKit growth |
|---|---|
| Command response (`ArrayBuffer`), not even drawn | +390 MB |
| Command response → `putImageData` (the product before this ADR) | +168–200 MB |
| Command response → `createImageBitmap` | +140 MB |
| BMP over a custom scheme → `fetch` → `Blob` → `createImageBitmap` | +118–126 MB |
| … the BMP with RGBA `BI_BITFIELDS` instead of plain `BI_RGB` | +115–126 MB |
| … the same, but plain `BI_RGB` with the bytes swapped in a slow debug loop (3× slower delivery) | +16–18 MB |
| **BMP over a custom scheme → `<img>` → `drawImage`** (adopted) | **+22–39 MB** |
| … plus a canvas per tile, 30 kept alive like the viewer; pooled or not | +32 MB |

Every route that hands the bytes to JavaScript (an `ArrayBuffer` or a `Blob`) leaves them to the
garbage collector, which reclaims them late and leaves WebKit's heap at its high-water mark. Only
the fast-delivery Blob variants show this, not the slow swap variant: slower delivery gave the
collector time to keep up. An image element loads and decodes natively and lets go when its
`src` is cleared. WebKit rejects a BMP with a `BITMAPV4` RGBA-bitfields header outright.
Canvas churn is not a factor.

## Decision

### Transport

- **Tiles are plain 32-bit BMPs served by the `safha` URI scheme:**
  - URL: `safha://localhost/tile?doc&lane&gen&page&scale&x&y&w&h&u&d`, or
    `http://safha.localhost/tile?…` on Windows.
  - The handler submits the request to the render queue exactly as the command did: same
    priority, generation, lane, cache and cancellation. It answers when the tile is done, and
    encodes it off the render thread.
  - The core now keeps bitmaps in PDFium's native **BGRA** order. `Bitmap::encode_bmp` is a
    54-byte header followed by one copy of the pixels, with no byte swap. `to_rgba()` serves PNG
    export and tests.
- **The UI loads tiles with an `<img>` element:**
  - It sets `crossOrigin="anonymous"`; the scheme answers with `Access-Control-Allow-Origin: *`,
    so canvases stay readable and the self-test's readback still works.
  - It calls `decode()`, draws the image into the tile's canvas, then clears `src`.
  - The CSP gains `img-src safha: http://safha.localhost`.
- **Undelivered tiles are classified by generation.**
  - An image element cannot read a response, so a cancelled tile (HTTP 409) and a failed one look
    alike (`tile_unavailable`).
  - The tile client treats one from an older generation as cancelled and re-requests it if still
    wanted. One from the current generation counts as failed.
  - A load that never settles times out after 10 s and is asked for again. That happened twice in
    about 40 experiment runs; the cause is unknown.
- **What was given up:**
  - Per-tile render time and cache-hit flags in the UI. The engine's counters (`render_stats`)
    still report both.
  - The `render_tile` command and its 16-byte header format. The M0 spike keeps the format in
    `spikes/rawBitmap.ts`.

### Zoom gestures

- **Pinch and Ctrl/Cmd+wheel scale the page area with a CSS transform** (with
  `will-change: transform`) about the point under the pointer.
- **A real relayout happens only in three cases:**
  - the scale has moved beyond 0.5–1.5× since the last layout;
  - 250 ms have passed since the last layout;
  - the gesture has ended (no event for 140 ms).
- **Why:** resizing every page on every wheel event made WebKit repaint dozens of stretched
  canvases and blurred shadows per frame, and grew the webview by 160 MB per pinch. Our own work
  was about 1 ms per frame; the timing counters added to the viewer (`ViewerController.profile`)
  showed that.
- **Without `will-change`** the transform made WebKit re-rasterize at each new scale: +187 MB per
  pinch-in.
- Mounting a whole viewport beyond the visible area covers what a 0.5× scale-down reveals.

### Measuring frames fairly

The checker used to require a p95 frame interval of at most 17.5 ms. An idle WKWebView already
shows a p95 of 18–19 ms, because its timestamps jitter by 1–2 ms. The rule is now:

- at most 1% of all frames **dropped** (over 25 ms: a missed 60 Hz vsync);
- a p95 of at most 20 ms in every phase of 40 frames or more.

The benchmark now records memory after every phase, and the viewer's own time per phase.

## Measured (Apple-silicon MacBook, 2× display, window visible, page area 960 × 716 CSS px)

Release build, 1,000-page file:

| | Before (debug) | After: debug | After: release | Budget |
|---|---|---|---|---|
| Dropped frames, all phases | — | 6 / 1,368 | **3 / 1,369 (0.2%)** | ≤ 1% ✓ |
| Scroll 4,000 / 12,000 px/s, fling | 1–2% over 20 ms | 0 dropped | **0 dropped** | ✓ |
| Pinch in (×6) | 24% over 20 ms | 0 dropped | **0 dropped**, p95 18 ms | ✓ |
| Pinch out (×1/20 in 75 frames) | 47% over 20 ms | — | **3 dropped, p95 25 ms** | ✗ |
| Longest blank page while scrolling | 0 ms | 0 ms | 12 ms | ≤ 100 ms ✓ |
| Random jump → preview (p95 / max) | 26 / 27 ms | 17 / 21 ms | **15 / 16 ms** | ≤ 100 ms ✓ |
| Shell resident at end | 359 MB | 374 MB | **339 MB** | < 500 MB ✓ |
| WebKit (content + GPU + network) at end | 1,006–1,050 MB | 612 MB | **387 MB** | — |
| Shell + WebKit at end | 1,368 MB | 987 MB | **726 MB** | < 500 MB ✗ |

Run-to-run variation is large for WebKit's memory (±150 MB between identical debug runs: garbage
collection runs on its own schedule), small for frames.

## Open

- **Pinch-out still drops frames** in this extreme case: from 7× to 0.35× in 1.2 s, 3 frames
  drop. The remaining cost is the relayouts themselves (pages resized and repainted). Ideas not
  yet tried:
  - mount fewer pages per frame during a gesture;
  - a cheaper page shadow;
  - fewer relayouts by time.
- **Memory over 500 MB counts WebKit.** WebKit's own floor is about 155 MB in release (290 MB in
  debug) with only the first page on screen. The shell alone stays under 500 MB. Whether the spec's budget
  covers WebKit's processes is the owner's call (PROGRESS.md). The shell's growth to 339 MB is
  64 MiB of tile cache plus allocator retention from about 3,400 renders and their response
  copies. A response body must be an owned buffer, so the copy cannot go.
- **The release bundle sometimes opens hidden** under the benchmark (3 of 5 launches), with the
  screen unlocked and the window moved to the display under the pointer. Such runs are rejected
  as invalid, as designed.
- **Windows and Linux** are untested. The scheme URL differs on Windows (`convertFileSrc`
  handles it), and the CSP lists both forms.
