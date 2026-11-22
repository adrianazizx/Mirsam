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
| [0014](0014-edits-as-incremental-updates.md) | Edits as incremental updates written by our own writer; undo as a stack of them; documents keep their bytes (and, in memory, their password) | Accepted for page edits and annotations (0017) |
| [0015](0015-saving-and-unsaved-changes.md) | Saving (signed files incrementally, others rewritten in full by PDFium; validated; atomic rename); asking before closing a modified tab or window, or quitting | Accepted |
| [0016](0016-pages-across-files.md) | Pages across files: extract, split, insert and merge through PDFium's page import; snapshot steps in the edit history | Accepted |
| [0017](0017-markup-annotations.md) | Markup annotations (highlight, underline, strikethrough, squiggly) as incremental updates with appearance streams; encrypting new objects; repainting instead of rebuilding | Accepted |
| [0018](0018-ink-and-notes.md) | Ink with pressure and an eraser, sticky notes (upright on turned pages, text in `/Contents`), the Comment tool and its note editor | Accepted |
| [0019](0019-text-boxes-and-the-text-writer.md) | Text boxes drawn with shaped text and embedded font subsets (`pdf::text`, the text half of the content-stream writer); word spaces read inside form XObjects | Accepted |
| [0020](0020-text-layout-engine.md) | Text layout engine `safha_core::typeset`: bidi + script + font itemization, harfrust shaping, UAX #14 breaking, per-line reordering; backend-neutral positioned glyph runs; font subsets (variable fonts instanced) and font matching | Accepted (M3 groundwork) |
| [0040](0040-autosave-and-recovery.md) | Autosave: each modified document's current bytes in a recovery file 2 s after its last change; restored at startup (undo back to the original when unchanged, Save As when it changed); encrypted ones ask for the password | Accepted |
| [0041](0041-comments-panel.md) | Comments panel: threads read by our parser (`/IRT`, review states) with PDFium's text; replies and resolve as hidden-appearance annotations; authors only by the user's chosen name; filter, sort, go to | Accepted |

Still to be decided, each with its own spike (see `docs/SPEC.md`): in-place editor surface (M3),
OCR engines (M4), crypto/PAdES library (M5), Office → PDF (post-1.0).
