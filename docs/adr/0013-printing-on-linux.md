# 0013 — Printing on Linux: GTK's print dialog, pages drawn by PDFium

Status: Proposed. Built; checked in CI only by printing to a file (see "Verification").
Date: 2026-11-22.

## Context

ADR 0012 prints on macOS and Windows by showing the PDF in the platform's own viewer and calling
`print()`, so the system prints the PDF itself. On Linux that route does not exist: WebKitGTK has
no PDF viewer.

Spec §5.9 lists "print with page scaling, booklet, selected pages" as P0. The workspace denies
`unsafe`. On Linux, Tauri already builds on GTK 3 through the `gtk` 0.18 crate.

## Options

| Option | Quality | Memory | Dialog |
|---|---|---|---|
| **GTK `PrintOperation`, each page rendered by PDFium when GTK asks for it** (chosen) | raster at the printer's resolution | one page at a time | GTK's own: printer, range, copies, scale, pages per sheet |
| A print-only web page of page images, then `print()` in WebKitGTK | raster | every page decoded at once (gigabytes for 1,000 pages) | GTK's, via WebKit |
| Send the PDF to CUPS with `lp`, behind a print dialog of our own | vector (CUPS converts the PDF) | none | ours to build, in two directions; no preview |
| PDFium into a cairo PDF or PostScript surface through FFI | vector | low | — (needs `unsafe`) |

## Decision

- **`print_linux.rs`:**
  - `print_document` runs a `gtk::PrintOperation` on the main thread, parented to the calling
    window, and resolves when the dialog closes.
  - For each page GTK asks for (`draw-page`), PDFium renders the page at the print context's
    resolution, clamped to 150–600 dpi and to at most 48 megapixels. The page is fitted to the
    printable area and centred.
  - The bitmap goes to cairo as an `ARGB32` image surface without a copy. PDFium's BGRA is that
    layout on little-endian machines, and pages render opaque.
  - A page wider than it is tall turns the paper (`request-page-setup`).
- **The dialog's own settings apply on top:** printer, page range, copies, scale, pages per
  sheet, odd or even pages.
- **Errors** reach the UI as `print_failed`, with its own message in English and Arabic.
  Cancelling is not an error.
- **Dependency:** `gtk` 0.18 (MIT), Linux only. Tauri's window toolkit already uses this exact
  crate and version, so the build gains no new code. The GTK 3 bindings are no longer developed
  upstream; that is Tauri's dependency first, and it moves when Tauri moves.

## Consequences

- **Raster output:**
  - Text prints as an image at printer resolution (300–600 dpi), not as vector glyphs. That is
    sharp on paper but gives larger print jobs.
  - Vector output on Linux would need either our own print dialog over CUPS or `unsafe` FFI.
    Revisit if users print text-heavy documents in volume.
- **Booklet printing** is not in GTK's dialog. CUPS offers it as a printer option on some
  drivers. A booklet mode of our own (imposition before printing) is M6 work along with the
  other page-arrangement features.
- **Rendering runs on the main thread** while GTK spools; the UI waits for the dialog anyway.

## Verification

- **CI (Linux, under Xvfb):**
  - With `SAFHA_PRINT_EXPORT=<file>`, which is read only in `devtools` builds, the operation
    exports to a PDF instead of opening the dialog.
  - The self-test prints `derived/rotated.pdf` (four pages, turned 0°, 90°, 180° and 270°) and
    checks that four pages come out.
  - It uploads small previews, rendered by our own `render_png`.
- **Not verified:** the interactive dialog with a real printer. It needs a Linux desktop.
