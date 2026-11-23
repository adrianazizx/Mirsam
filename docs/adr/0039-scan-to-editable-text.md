# 0039 — Scan to editable text

Status: Accepted (M6). Date: 2026-11-23.
Builds on ADR 0021 (text in page content), ADR 0023 (in-place editing), ADR 0035 (the text
layer), ADR 0037 (review), ADR 0051 (page objects).

## Context

Spec §5.8, P1: "Convert scan to editable text (real text objects with matched fonts)". The
M6 task adds:

- the result is **paragraph blocks the M3 editor can open**;
- the scan is **kept behind or removed**, as the user chooses;
- the text is **in logical order**;
- nothing uncertain is silently made into text.

A recognised page has an invisible layer: words with boxes, lines in reading order (ADR 0035).
The M3 editor reads visible text (render modes 0 and 2) and rebuilds paragraphs from its glyphs
(ADR 0023). Invisible OCR text is skipped there, by design.

## Decision

### 1. Paragraphs from the layer's lines (`ocr::convert::paragraphs`)

- **Each line is reduced to its band.** That is the median top and bottom of its words (the
  layer's own rule, ADR 0035), its two ends, and its words joined in logical order.
- **A line joins the paragraph before it** when:
  - it has the same direction;
  - its height is 0.7–1.4 of the paragraph's;
  - the gap above it is −0.3 to 1.1 heights;
  - it overlaps the paragraph by half the narrower width;
  - the line before did not end over three heights short of the paragraph's end side (right in
    LTR, left in RTL).
- **Alignment.** Both edges flush (the last line excepted) is justified, unless both are flush
  everywhere; then the start edge's. Centred lines are centred, and one flush edge is that
  edge's.
- **The text** of a paragraph is its lines joined by spaces: it reflows like the M3 editor's
  paragraphs. No characters are changed, so a hyphen at a line's end stays.

### 2. Type

- **The family.** Paragraphs are set by the text engine in a bundled family:
  - **Naskh** unless the person chooses Sans: Amiri has matching Latin letters, a serif like most
    printed documents;
  - **Sans** is IBM Plex Sans / Plex Sans Arabic.

  The scan's typeface is not identified. That is an honest limit of "matched fonts", and the
  Edit tool changes a paragraph's family afterwards.
- **The size.**
  - **First guess:** the mean of what the ink says (line ink height / 0.75) and, with two lines
    or more, what the pitch says (pitch / 1.25), between 4 and 120 pt.
  - **Shrunk to fit:** by 5% steps while the paragraph takes more lines than the scan did, to
    0.6 of the first guess.
  - **Line pitch:** the scan's.
  - **First baseline:** at the scan's first line's ink bottom, 0.2 em above it for Arabic
    (descending letters).
- **One subset per page.** The paragraphs' layouts are joined into one layout of all their glyph
  runs. That makes one subset per face, shared by every paragraph of the page, so a page
  embeds each face once.
- **Colour.** Ink and paper are sampled from the page rendered at 100 dpi: the mean of the
  darkest twelfth and of the lightest 30%. Faint or low-contrast ink is written black.

### 3. The scan

- **Kept (the default).** The image stays as it is: it is never altered (spec §8).
  - **Patches:** each converted line gets a patch in the paper's colour over its ink, 0.3 of a
    line's height beyond it, drawn before the text, so the scan's glyphs do not show through.
  - **What stays visible:** figures, stamps and signatures outside the text are untouched.
  - **Undo** removes the patches with the text.
- **Removed.** The page's images covering at least half of it are deleted by byte range (ADR
  0051's object delete), topmost first.

### 4. One update (`Document::with_conversion`)

1. **The scan,** if it is to go: removed.
2. **Another tool's invisible text:** removed (ADR 0038).
3. **One stream with the patches and the paragraphs** (`/MirsamText` marked content, as the
   add-text tool writes) comes; **our OCR layer** goes from `/Contents`. The page's text is page
   content now, so a layer would make search find every word twice.
4. **The stream's dictionary** says `/MirsamConverted <digest>`: the digest of the layer it came
   from.

The whole conversion is one edit (one Undo). The written text reads back in logical order, as
the add-text writer's does (ADR 0021): the ToUnicode map, and `/ActualText` for clusters of
several glyphs.

### 5. Nothing uncertain becomes page text without a decision

- **A page with an undecided uncertain word**, or an unchecked cut-off reading, is refused:
  `ocr_convert_unreviewed`, with the page numbers. The panel says which pages to review first.
- **Corrected, reordered and low-confidence words** do not block: the engines agreed, or nothing
  contradicted the reading. They stay on the review list.
- **After conversion,** the review still finds the page through `/MirsamConverted` (ADR 0037's
  resolution). It is shown as converted: its words are fixed with the Edit tool, and review
  accepts or checks them but no longer writes a layer (`ocr_review_converted`).
- **Converting everything** (`ocr.convertDocument`) takes every page with a final layer.

## Evidence

`tests/ocr_review.rs`, `a_recognised_page_becomes_paragraphs_the_editor_opens`.
- **The page.** A scan with another tool's layer, replaced by ours:
  - an Arabic paragraph of three lines;
  - an English paragraph of two;
  - a mixed line ("اشتريت جهاز iPhone 15 Pro بسعر ٤٥٠٠ ريال").
- **Converted,** keeping the scan and removing it. Both pass `qpdf --check`.
- **The text:**
  - 3 paragraphs are found;
  - the page's text is the three in logical order;
  - the other tool's text is gone;
  - the layer state says converted, from the right digest.
- **The editor:** the M3 editor lists them as editable blocks (the Arabic one RTL), and an edit
  of the Arabic paragraph ("الرياض" → "جدة") works and reads back.
- **The scan:** one image when kept, none when removed.
- **A rendering** of the kept-scan page shows the Naskh paragraphs at the scan's lines, with
  patches under them.

UI tests:
- conversion is refused until the unsure word is decided, with a notice naming the page;
- after it, the page is listed as converted and review points to the Edit tool.

## Known limits

- **The typeface is not matched,** only the script's family (Naskh or Sans) and the size.
- **Tables, columns of numbers and forms** become paragraphs line by line where lines do not
  join. Cells are not rebuilt (table recognition is P2).
- **Word positions inside a reflowed line** follow the type, not the scan, so a paragraph whose
  typeface is much wider or narrower than the scan's moves its line breaks.
- **Removing the scan removes page-sized images only.** Smaller images (a logo) stay.
- **Encrypted files:** conversion follows the M3 editor's limits; resources holding strings in
  encrypted files are refused.
