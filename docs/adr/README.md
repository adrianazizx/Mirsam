# Architecture Decision Records

One short record per significant decision: context, decision, evidence, consequences.
Records are immutable once accepted; a later decision supersedes an earlier one by
number and says so.

| # | Decision | Status |
|---|---|---|
| [0001](0001-app-shell-tauri.md) | App shell: Tauri 2 | Accepted (assumption D1) |
| [0002](0002-pdf-engine-pdfium.md) | Render/parse engine: PDFium via `pdfium-render`, dynamically loaded | Accepted |
| [0003](0003-ui-stack.md) | UI stack: React, React Aria Components, CSS-variable tokens, i18next | Accepted |
| [0004](0004-tile-transport.md) | Tile transport: native PDFium + raw binary IPC | Accepted (spike); transport superseded by 0011 |
| [0005](0005-pdf-text-writing.md) | Writing text: own shaping pipeline; PDFium's content regeneration is lossy, so text is emitted by our own writer as incremental updates (2026-11-22 amendment) | Accepted, amended in M3 |
| [0006](0006-ocr-evaluation.md) | OCR evaluation method and gates; engine choice deferred to M4 | Accepted |
| [0007](0007-fixtures-and-self-test.md) | Generated fixture corpus and in-app self-test harness | Accepted |
| [0008](0008-text-extraction-and-search.md) | Text extraction in reading order (geometry + bidi + producer quirks) and Arabic-aware search | Accepted |
| [0009](0009-viewer-rendering-pipeline.md) | Viewer: one render thread, viewport generations, 64 MiB tile cache, imperative tiled viewer | Accepted; frame-rate and blank-time budgets not yet measured |
| [0010](0010-navigation-search-selection-ui.md) | Navigation UI: side panel, search, visual-select/logical-copy selection, palette, reading themes, local state, password prompt | Accepted |
| [0011](0011-tiles-as-images-and-gesture-zoom.md) | Tiles as BMP images over the `safha` scheme (supersedes 0004's transport); compositor-only zoom gestures; fair frame criterion | Accepted; pinch-out and total memory still over budget |
| [0012](0012-reading-preferences-windows-printing.md) | Reading preferences from the catalog (own safe reader), several windows, tab reordering; printing through the platform's PDF viewer | Accepted; printing proposed (unverified) |
| [0013](0013-printing-on-linux.md) | Printing on Linux: GTK's print dialog, each page rendered by PDFium at the printer's resolution | Proposed; checked in CI by printing to a file |

Still to be decided, each with its own spike (see `docs/SPEC.md`): in-place editor surface (M3),
OCR engines (M4), crypto/PAdES library (M5), Office → PDF (post-1.0).
