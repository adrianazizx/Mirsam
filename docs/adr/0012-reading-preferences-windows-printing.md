# 0012 — A document's reading preferences, several windows, and printing through the platform

Status: Accepted for reading preferences and windows. Printing is **Proposed**: built, but its
spike could not be observed on screen (see "Printing"). Date: 2026-11-22.

## Context

Three Milestone 1 items remained after ADRs 0010 and 0011:

- reading a document's `/ViewerPreferences /Direction` (spec section 6.5: right-to-left binding
  in two-page view);
- multiple windows (spec section 5.1);
- printing.

PDFium offers the viewer preferences only through `FPDF_VIEWERREF_GetName`, which pdfium-render
binds as an `unsafe fn`, and `unsafe` is denied in this workspace. PDFium has no portable print
API.

## Decision

### Reading preferences: our own read-only catalog reader (`safha_core::catalog`)

- About 670 lines of safe Rust plus 170 of tests: a PDF object parser and the cross-reference
  chain from the end of the file.
- What it handles:
  - classic tables;
  - cross-reference streams with PNG predictors;
  - hybrid files (`/XRefStm`) and incremental updates (`/Prev`);
  - Flate-compressed object streams, through `miniz_oxide` (MIT/Zlib/Apache-2.0, already in
    the tree), with a 64 MB cap on decompressed size.
- It reads `/ViewerPreferences /Direction` and `/PageLayout` and nothing else.
- Input is untrusted: nesting, the number of sections and the decompressed size are bounded, and
  any surprise yields "no preference". Examples: other filters, encrypted object streams (it
  does not decrypt), damage.
- It runs on the bytes `Engine::open_file` already reads, before PDFium takes them: no extra I/O.
- Defaults that follow from it:
  - `/Direction /R2L` → right-to-left page order;
  - `/PageLayout` → layout: `SinglePage` → single, `OneColumn` → continuous, `…Left` → spread,
    `…Right` → book (the first page alone).
- The user can still change either. A saved per-file position does not yet store layout or
  order.
- **Why not lopdf:** it parses every object of a 1,000-page file to read one dictionary, and it
  silently drops encryption when saving (session 1). The reader is also groundwork for the own
  writer ADR 0005 keeps as a second backend.
- **Tests:**
  - unit tests: classic, incremental, object streams, encrypted, hostile;
  - qpdf-made fixtures: `derived/rtl-book.pdf`, and the same book with its catalog in an object
    stream (`derived/rtl-book-objstm.pdf`);
  - every other fixture reports no preference.

### Windows

- **Ownership:** every document belongs to one window. The UI already lived per webview; the
  shell now records each document's window, and commands find their caller through the
  `WebviewWindow` argument.
- **Commands:** New Window (Cmd/Ctrl+N) and Move Tab to New Window. Opening a file already open
  in the same window switches to its tab; another window may open its own copy.
- **Closing a window closes its documents and drops its tabs from the session**, except while the
  app is quitting (`RunEvent::ExitRequested`) or when it is the last window: those tabs are what
  the next launch restores.
- **The session stores windows:**
  - schema version 2 adds a `window` column to `session_tabs`, migrated in place and tested from
    a version 1 database;
  - the main window's first `startup` reopens window 0's tabs and opens one window for each
    other saved window.
- **Settings follow across windows:** the shell emits `settings-changed` when any window stores a
  setting, and every window applies it.
- **Capability:** it covers `main` and `window-*`. Print windows (`print-*`) get no IPC access.

### Tabs

- Ctrl+Shift+PageUp / PageDown move the active tab earlier or later, and tabs can be dragged.
- The drop position is computed in reading order (`tabDrag.ts`), so dragging works the same in an
  Arabic UI, where the first tab is on the right.

### Printing: through the platform's own PDF viewer (proposed)

- **How it works:**
  - The `safha` scheme serves a document's PDF bytes (`/document?doc=N`; PDFium's save, so
    later edits will be included).
  - `print_document` opens a window showing that URL and calls `print()` once the page has
    loaded.
  - WebKit (macOS) and WebView2 (Windows) render PDFs with the platform's own viewer, so the
    system print panel would print the PDF itself: vector quality, its page sizes, its own page
    range and copies. No `unsafe`, no rasterizing.
- **Alternatives considered:**
  - **Rasterize pages into a print-only page and print the webview:** portable, but pages print
    as images, and a 1,000-page job means gigabytes of decoded images.
  - **Native APIs** (PDFKit's print operation, PDFium into a Windows printer context): the right
    quality, but they need `unsafe` FFI, the workspace forbids it, and each OS needs its own code.
- **Spike result:** inconclusive. In the real app the command returns in 49 ms and the window
  opens, but the session could not see the screen (the window reported itself hidden, and
  `screencapture` was refused). The app then stayed busy until stopped, consistent with a modal
  print panel but not proof of one.
- **What's untested:**
  - that WebKit shows the PDF;
  - that `print()` prints every page rather than the visible part;
  - encrypted documents: PDFium's save keeps the encryption, so the viewer may ask for the
    password again;
  - Linux: WebKitGTK has no PDF viewer. Linux prints through GTK instead (ADR 0013).

## Consequences

- `PROGRESS.md` asks the owner to try Print (Cmd/Ctrl+P) once. If it fails, the fallback is the
  raster path.
- The catalog reader is the first PDF-syntax code in the repository. Its parser is deliberately
  tolerant and read-only; the M2/M3 writer will need its own strict serializer.
