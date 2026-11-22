# 0004 — Tile transport: native PDFium in the core, raw binary over Tauri IPC

Status: Accepted (Milestone 0 spike); the transport choice is **superseded by ADR 0011**. Tiles
now travel as BMP images over the `safha` URI scheme, loaded by image elements. This spike timed
delivery; it could not see the webview's memory, which is where raw command responses cost
200–400 MB. The rest of this record (native PDFium, no base64, no WebAssembly engine) stands.
Must be re-measured on Windows and Linux before M1 closes. Date: 2026-11-22.

## Question

The spec (§9, "Rendering design") requires tiles to reach the UI over a binary channel and asks
for a spike: Rust-side PDFium plus binary transfer, versus PDFium compiled to WebAssembly inside
the webview — chosen by scroll smoothness and memory.

## Method

`scripts/spike-tile-transport.sh` launches the real desktop shell (`devtools` build) on a fixture
and runs `apps/desktop/src/spikes/tileTransport.ts` inside the webview. The same PDFium build
(chromium/8125) renders the same tiles of the same page through each candidate; each tile is
drawn to a canvas with `putImageData`. 25 timed iterations after 3 warm-ups per case.

Candidates:

1. **invoke-raw** — command returning `tauri::ipc::Response` (raw bytes → `ArrayBuffer`).
2. **custom-protocol** — `fetch("safha-tile://…")` served by a registered URI scheme handler.
3. **channel** — bytes pushed over a `tauri::ipc::Channel`.
4. **base64-json** — the anti-pattern the spec forbids, as a control.
5. **wasm-main-thread** — `pdfium.wasm` in the webview, rendering straight into a canvas-backed view
   of the wasm heap (no transfer at all).

Environment: Apple-silicon MacBook, macOS 26, WKWebView, 2× display, window visible. Shell crate
in a debug build (dependencies at opt-level 2; PDFium itself is the same optimized binary in every
case). `performance.now()` in WKWebView has **1 ms resolution**, so UI-side numbers are ± 1 ms; the
render times come from Rust and are precise.

## Results

Median per tile, milliseconds. *Overhead* = total − engine render time: IPC, copy, decode,
`putImageData`.

| Tile (RGBA size) | Render | invoke-raw overhead | custom-protocol overhead | channel overhead | base64-json overhead | wasm total (render + paint) |
|---|---|---|---|---|---|---|
| 256×256 (0.26 MB) | 0.55 | 0.47 | 0.43 | 0.47 | 7.98 | < 1 |
| 512×512 (1.05 MB) | 0.74 | 0.32 | 0.29 | 1.21 | 29.2 | 1 |
| 1024×1024 (4.19 MB) | 1.58 | 1.44 | 1.44 | 1.49 | 116.6 | 2 |
| whole page 1785×2526 (18.0 MB) | 4.1 | 5.9 | 5.2 | 5.9 | 497.9 | 6 |

95th-percentile overhead for the three binary transports stays within 1 ms of the median except
the whole-page case (5.8–7.1 ms).

A viewport's worth of tiles at once (12 × 512 px, requested concurrently), median of 5 runs:

| Transport | All 12 painted after | Longest frame |
|---|---|---|
| invoke-raw | 11 ms | 22 ms |
| custom-protocol | 11 ms | 16 ms |
| channel | 12 ms | 16 ms |
| base64-json | 69 ms | 36 ms |
| wasm-main-thread | 200 ms* | 17 ms |

\* By construction: the wasm run yields to the browser after every tile (one tile per frame),
which is what a main-thread renderer must do to keep frames short. Back-to-back it would be
roughly 12 × 1 ms.

Memory, shell process resident size (each transport moved 659 MB of pixels in total):

| Point | RSS |
|---|---|
| start | 164 MB |
| after invoke-raw | 176 MB |
| after custom-protocol | 176 MB |
| after channel | 176 MB |
| after base64-json | **789 MB**, not released after 3 s idle |

The WebAssembly engine's heap was 22.6 MB with the document open; it initialized in 62 ms.
The webview content process could not be measured from inside the app.

## Findings

1. **Any raw binary route is fast enough.** A 512 px tile costs about 0.7 ms to render and
   0.3–1.2 ms to deliver; twelve of them arrive inside one 16 ms frame. The three binary
   transports are indistinguishable at this timer resolution, and none grows memory.
2. **Base64 in JSON is disqualifying on both axes**: 25–100× the delivery cost, dropped frames in
   the burst, and 600 MB of retained shell memory. The spec's prohibition is confirmed by
   measurement; `CLAUDE.md` carries it as a rule.
3. **PDFium-in-WebAssembly is not slower.** On these text pages it matched native render speed
   and, with zero transfer, had the lowest total latency. It is rejected for other reasons:
   - It renders on the UI thread. These pages take 1–6 ms; an image-heavy or pathological page
     takes hundreds, and every millisecond of that is a frozen UI. Moving it to a Web Worker fixes
     that but reintroduces a buffer hand-off, removing its one advantage.
   - The core needs native PDFium regardless (editing, saving, text analysis, OCR input, CLI).
     A second engine in the webview means two copies of every open document and a re-sync of the
     whole file after each edit — more memory and a new class of staleness bugs — to save about a
     millisecond per tile.
   - It keeps "page pixels come from the core, never from the webview", which is this project's
     main defence against per-OS webview differences (ADR 0001).
4. Rendering, not transport, will be the bottleneck on complex pages, and PDFium is
   single-threaded per process (ADR 0002). M1's effort belongs in scheduling, cancellation, and
   caching, not in the pipe.

## Decision

- Render in the core with native PDFium; deliver as **raw bytes through a Tauri command**
  (`tauri::ipc::Response`) — a 16-byte little-endian header (width, height, render µs, reserved)
  followed by straight RGBA, decoded without copying (`src/backend/bitmap.ts`). This is what
  `render_page` already does.
- The transport is hidden behind `Backend.renderPage`, so switching costs one file. The
  **custom protocol** is the designated alternative: equally fast, and it gives `fetch`
  semantics (`AbortController`, streaming). Revisit it in M1 if request cancellation is cleaner
  that way; it needs the scheme added to the CSP `connect-src`.
- No WebAssembly renderer in the desktop app. It stays the natural engine for the future *web*
  version (the same PDFium build exists as wasm, and it performed well here).

## Limits of this spike

- One machine, one OS. **WebView2 (Windows) and WebKitGTK (Linux) are unmeasured**, and Tauri's
  IPC path differs per platform. Re-run `scripts/spike-tile-transport.sh` on both before relying
  on these numbers there.
- Simple text pages only; the transport conclusions do not depend on page complexity, the
  wasm-versus-native render comparison might.
- 1 ms timer resolution; a debug build of the shell crate; webview process memory not measured.
- Spike code stays in the tree behind the `devtools` feature so the measurement is repeatable; it
  is not product code.
