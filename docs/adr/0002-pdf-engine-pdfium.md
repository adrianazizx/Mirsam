# 0002 — Render/parse engine: PDFium via `pdfium-render`, dynamically loaded

Status: Accepted. Date: 2026-11-22.

## Context

We need a permissively licensed engine that renders Arabic identically to reference viewers,
parses hostile files robustly, and exposes text with character geometry. Licence policy
(spec §10) rules out MuPDF, Poppler, and Ghostscript.

## Decision

- **PDFium** (BSD-3-Clause / Apache-2.0; Chrome's engine), through the **`pdfium-render`** crate,
  pinned `=0.9.6` (MIT OR Apache-2.0; last release 2026-11-21, actively maintained).
- **Prebuilt binaries** from `bblanchon/pdfium-binaries`, build **chromium/8125**, the newest API
  level `pdfium-render` 0.9.6 binds (`pdfium_8125` feature). `scripts/fetch-pdfium.sh` downloads
  per platform and verifies a pinned SHA-256. The non-V8 build is used: no JavaScript engine is
  present at all, which satisfies "no JavaScript execution from PDFs" by construction.
- **Dynamic loading** (`libloading`), not static linking: no C++ toolchain in our build, and the
  same binaries serve macOS, Windows, and Linux. The engine looks for the library in
  `SAFHA_PDFIUM_DIR`, then bundle locations next to the executable, then (debug builds only)
  `third_party/pdfium/`.
- The crate's `thread_safe` feature is on. PDFium is not re-entrant; every call is serialized
  behind one mutex. `Engine` is therefore a process-wide singleton and `Document` is `Send + Sync`.

## Evidence

- `crates/safha-core/tests/`: 11 tests pass. Every fixture from both producers opens, reports
  the right page count, and renders; the AES-256 fixture requires its password; the truncated
  fixture is recovered or rejected without a crash.
- 1,000-page fixture: open 9.6 ms, first render of page 1,000 in 19.7 ms (debug build) — opening
  does not scale with page count.
- A4 page at 1824×2582 px renders in 7–12 ms.
- Arabic shaping, lam-alef, tashkeel, and mixed-direction lines render correctly from both
  Chrome- and CoreText-produced PDFs (inspected renders).
- **Text extraction is not usable as-is for Arabic**: PDFium's raw page text reproduces 46 of 118
  expected logical-order blocks, and none of the pure-Arabic ones
  (`cargo run -p safha-core --example fixture_report`). This is expected — it is the M1 work in
  spec §6.1 — and now has a measured baseline.

## Consequences

- **No in-process render parallelism.** The spec's "worker pool" for tiles cannot be threads over
  one PDFium. M1 must choose between a single prioritized render queue with cancellation (likely
  sufficient: a 512 px tile renders in under 1 ms, ADR 0004) and multiple renderer processes.
  The same process boundary would also give the sandboxing the spec wants for P1.
- `pdfium-render` honours a render origin only on its form-data path; `render_tile` depends on
  that (see the comment in `document.rs`). A crate upgrade must re-run the tile tests.
- Bumping PDFium means bumping the build number, six checksums, and the `pdfium_NNNN` feature
  together.
- Bundling is configured for macOS arm64 only so far (`tauri.macos.conf.json`); Intel/universal
  macOS, Windows, and Linux bundle paths are still to do.
- PDFium is also the emitter for new text, through its CID Type 2 font API with our own shaping,
  subset, and ToUnicode (ADR 0005) — not through `FPDFText_SetText`, which does no shaping.
