# 0050 — Headers, footers, page numbers, watermarks; image and text export; compression

Status: Accepted (Milestone 6, the items that do not depend on M4 or M5). Date: 2026-11-22.

## Context

Spec §5.3 (P1): headers/footers, page numbers (Western or Arabic-Indic digits), watermarks.
Spec §5.9: PDF → PNG/JPEG per page (P0), PDF → plain text and Markdown with RTL preserved (P1),
compress/optimize with image downsampling, object cleanup and a size preview (P1). §6 asks for the
digit choice and right-to-left placement. ADR 0005's amendment rules out PDFium regenerating page
content, and the spec's licence list (§10) rules out every JPEG encoder on offer (owner item 15).

## Decision

### 1. Stamps: one new content stream per page, one undo step (`safha_core::stamp`)

A header or footer (a *band*: start, centre and end text along the top or bottom edge), page
numbers (a band with one slot), and a watermark are all *stamps*, written as one incremental update
(ADR 0014) the way new text is (ADR 0021): the page's own content wrapped once in `q … Q`
(`/MirsamWrap`), a new stream after it (over the page) or just inside the wrapper's `q` (behind it),
the page's own `/Resources` with the new fonts, image and transparency state under names it does not
use. Nothing on the page is regenerated or touched.

- **Placement is on the page as displayed**, through `ViewTransform`: a footer stands upright at the
  bottom of a page turned 90°, and a watermark turns about the displayed centre.
- **Right to left.** `start`/`end` are logical: with `rtl` the start slot sits on the right. Each
  piece of text is laid out by the text engine with its own direction (first strong character), so a
  Latin footer in an Arabic document shapes and orders correctly.
- **Digits.** `{page}` and `{pages}` are replaced in Western or Arabic-Indic digits (U+0660–0669),
  counting from a chosen first number over the stamped pages. The UI defaults to Arabic-Indic digits,
  right to left and Naskh in the Arabic UI.
- **Marked as pagination artifacts** (`/Artifact << /Type /Pagination /Subtype /Header|/Footer|
  /Watermark /MirsamStamp true >> BDC`), so tagged-PDF readers skip them and a later version can find
  and remove them.
- **One font set per band.** Every distinct text a band draws is laid out once; the font subsets
  are made from all of them joined as paragraphs (each shaped as it is alone) and embedded once.
  Page numbers on the 1,000-page fixture: **1.0 MB, 0.49 s** (release); with a subset per page they
  were 5.3 MB, 0.85 s.
- **Headers and footers are text**: they copy and search (page numbers are worth finding).
- **A text watermark is drawn as glyph outlines** (`pdf::outline`: the embedded subsets' `glyf`
  outlines, quadratic curves as cubic, composite glyphs followed), not as text. It looks the same, but
  it is decoration: as text, "CONFIDENTIAL" across every page would be read into copy, search and text
  export, and at 45° it reads back as scattered letters ("L IA T N E ID F N O C").
- **An image watermark** is a PNG (alpha becomes a soft mask; `png`, MIT/Apache, already in the tree
  through Tauri) or a JPEG embedded as it is (`DCTDecode`; only its frame header is read). The image is
  picked and read by the shell; the UI gets its file name.
- **Opacity** is an `ExtGState` (`/ca`, `/CA`).

### 2. Page images: PDFium renders, our encoders write

A page is rendered at 36–600 dpi (within the 8,192-pixel bitmap limit) and written as PNG or as
JPEG by **our own baseline encoder** (`safha_core::jpeg`, ~300 lines: 8×8 DCT, the standard Annex K
tables, 4:4:4 so thin Arabic strokes keep their colour). It needs no licence decision and is checked
against a real decoder: a JPEG it wrote is embedded in a PDF and PDFium's decoder reads its colours
back within 12 levels. One page goes to a file the user names; several go to a folder, never over an
existing file.

### 3. Text and Markdown: logical order, no control characters

The text is the extractor's (spec §6.1): blocks in reading order, lines in logical order, which is
what Arabic is stored and typed in, so any editor applying the Unicode bidirectional algorithm shows
it right. No directional marks are added; they would travel with copied passages. Plain text separates
pages with a form feed. Markdown writes each block as a paragraph (lines joined), escapes what would
become markup, marks pages with `<!-- page N -->` and `---`, and wraps a block whose direction
differs from the document's in `<div dir="rtl">` (or `ltr`), which Markdown renderers that take
inline HTML honour. Headings are not detected: the extractor reports no font sizes.

### 4. Compression: our own full rewrite, previewed before saving (`safha_core::compress`)

PDFium's save keeps every object it read and writes no object streams, so compression writes the
file itself from its newest revision:

1. only what the catalog and the information dictionary reach is written, renumbered from 1 (earlier
   revisions, deleted pages and orphaned resources go);
2. images are reduced to a target resolution worked out from how large each is **drawn** (page
   content and form XObjects walked with their matrices; an image is reduced only when every placement
   shows it finer than 1.5× the target), averaged over each new pixel's area, written as Flate, or as
   JPEG (our encoder) for colour images when asked, and kept only when smaller;
3. unfiltered streams are Flate-compressed (XMP metadata is left readable);
4. objects other than streams are packed into object streams with a cross-reference stream
   (PDF 1.5), or written with a classic table when that option is off.

The result is opened by PDFium and must have every page, or it is refused. It is **a copy**: the
dialog shows the sizes before and after (the size preview), and "Save compressed copy…" writes it.
Saving the document itself still goes through PDFium (ADR 0015), which would undo the object streams.
Signed files are refused (a rewrite breaks signatures), and so are encrypted ones (this reader does
not decrypt). Measured: the 1,000-page fixture 571 KB → 170 KB in 0.24 s; single-page Chrome
fixtures 12–26 % smaller; a 1,600-pixel image drawn 4 in wide at 100 dpi target: the file more than
halves and the page looks the same at 72 dpi.

### 5. UI

Six commands open one dialog each (`pages.headerFooter`, `pages.pageNumbers`, `pages.watermark`,
`file.exportImages`, `file.exportText`, `file.compress`); the dialog runs `output.*` commands with what
was chosen, so everything goes through the registry. Every dialog takes all pages, the current page,
or a typed range ("1-3, 5, 8-"; Arabic-Indic digits, the Arabic comma and dashes are read too).
Stamps are one undo step; exports and the compressed copy are written by the shell after a save
dialog. All strings are in English and Arabic; the dialogs use logical CSS properties only.

## Consequences and open points

- **Rotated text reads back badly.** The text extractor clusters lines in page space, so text that
  runs vertically in page space (an upright footer on a page turned 90°) extracts letter by letter.
  The stamp is right on screen and in print; making the extractor follow the text matrix is extractor
  work, not stamp work.
- **JPEG images are not downsampled** (nor JPEG 2000, CCITT, JBIG2, 16-bit, masked or colour-keyed
  images): reducing them needs a JPEG decoder, owner item 15. The compress report counts them.
  Scanned documents are mostly JPEG, so this is the biggest gap in compression.
- **No font subsetting of existing fonts and no duplicate-object merging** yet; both are further
  size wins.
- Long exports and compression run off the UI thread but have no progress bar or cancel yet
  (spec §4 asks for both).
- Removing a stamp later (finding `/MirsamStamp`) and Bates numbering (P2) are not built.
