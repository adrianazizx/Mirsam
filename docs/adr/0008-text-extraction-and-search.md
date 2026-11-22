# 0008 — Text extraction in reading order, and Arabic-aware search

Status: Accepted (Milestone 1). Date: 2026-11-22.

## Context

Spec §6.1: extract text in logical order — rebuild reading order from glyph positions, apply the
Unicode Bidirectional Algorithm, never trust content-stream order, never double-reverse, map
presentation forms to base letters, and handle broken `ToUnicode` maps gracefully (offer OCR).
Spec §6.2: search ignoring tashkeel/tatweel, unifying alef variants and ى/ي, optional ة/ه,
Arabic-Indic = Western digits. Budget (§11): search 500 pages in ≤ 1 s after the first index.

Baseline at the end of M0: PDFium's own page text reproduced 46 of the corpus' 118 expected
logical-order blocks, and none of the pure-Arabic ones.

## What the producers actually do (measured on the fixture corpus)

- **Chrome** (Skia PDF) writes Arabic left to right in *visual* order inside a `/ReversedChars`
  marked-content section, each letter wrapped in an `/ActualText` span, Noto Naskh's dots as
  separate glyphs from a second font, and a real space glyph at every word break.
- PDFium partially undoes that: it reorders characters, but its character list **drops most of
  the space glyphs** and reports the rest at wrong positions; reports the letters of a
  multi-letter glyph **reversed** (`ين` as `ني`) — except lam-alef; sometimes reports a cluster
  **twice** (once from `/ActualText`, once from a glyph's own mapping, occasionally the dot); and
  inserts line breaks inside words wherever cursive attachment shifts a glyph off the baseline.
- PDFium's character boxes are not reliable enough to find word gaps in Naskh: a glyph shifted by
  cursive attachment can show a larger gap *inside* a word (0.235 em) than a real word space
  (0.215 em).
- **CoreText** writes letters in logical order with positions and its own spaces; with Noto Naskh
  it maps each separately drawn dot to junk (`#`, `%`, `+`) and dotless bodies to `ٮ` — the
  Arabic is unrecoverable from the file's text layer; with Amiri, vowel marks land on ASCII codes.

## Decision

`safha_core::text` (pure layout logic in `layout.rs`, PDFium access in `glyphs.rs`):

1. Collect PDFium's non-generated characters, plus the page's **drawn space glyphs** from the
   page-object list (zero-width text objects with no or whitespace text) — the most reliable word
   break signal for Chrome output.
2. Group characters into **units** (one drawn glyph): same origin, close in the stream. Restore
   letter order inside a unit with Arabic **joining rules**: a letter that only joins backwards
   (ا د ذ ر ز و ة …) cannot start a ligature, so it shows whether a glyph's letters arrived
   reversed; the page votes, lam-alef excluded. Attach independently positioned marks to the letter
   under them. Drop duplicates that overlap a twin.
3. Build lines by baseline (tolerant of cursive shifts), split at column-sized gaps, order units by
   origin.
4. **Word spaces**: on pages that draw space glyphs, those alone decide breaks next to Arabic;
   elsewhere, a real space between two visual neighbours in the stream, or a gap wider than a
   per-script threshold (Arabic 0.25 em, others 0.15 em).
5. Group lines into blocks; a block's direction comes from its alignment within its column (an
   RTL paragraph hugs the right edge), else from the majority of strong characters. Page direction
   orders columns.
6. Convert each line visual → logical with `unicode-bidi`'s reordering (it is its own inverse for
   the classes involved), moving whole units so ligatures and marks keep their order; NFC per unit,
   NFKC for presentation forms; repair brackets and curly quotes mapped as drawn.
7. **Quality**: flag a page `Suspect` when its mapping is evidently broken (dotless letters, ASCII
   symbols drawn on Arabic letters, ASCII carrying Arabic vowel marks), so the UI can offer OCR
   (spec §6.1). `NoText` for image-only pages.

Output keeps per-unit spans (byte range + glyph box) for selection and highlighting.

`safha_core::search`: queries fold both sides (normalizations above, optional case folding,
presentation forms → base letters), keep a byte map back to the original, extend matches over
trailing marks, and return glyph boxes. Literal, whole-word, and regex (`regex` crate, on folded
text). `DocumentIndex` extracts every page once, cancellably.

## Results

| | Before (PDFium text) | After |
|---|---|---|
| Expected blocks reproduced verbatim, whole corpus | 46 / 118 | **83 / 118** |
| … on pages with a trustworthy mapping | 46 / 70 | **67 / 70** |
| Broken-mapping pages flagged for OCR | — | **6 / 6** (CoreText Naskh and Amiri) |
| Index 500 pages (release) | — | 0.54 s (≈1.1 ms/page) |
| Search 500 indexed pages (release) | — | **10 ms** (budget 1 s) |
| Index / search 1,000 pages (release, `examples/search_bench.rs`) | — | 1.09 s / 18 ms; index ≈ 34 MB resident |

All Chrome Naskh and Plex fixtures, the two-column contract, and CoreText's Plex page come out
exact. Remaining misses on trustworthy pages, recorded as floors in `crates/safha-core/tests/text.rs`:

- `(64-bit)` inside an Arabic line comes out as `bit-64(`: a neutral between two left-to-right
  runs inside right-to-left text has more than one logical source for the same visual order, and
  the reorder-is-its-own-inverse shortcut picks the wrong one. Needs a run-aware inverse.
- One vocalized Amiri cluster (`وَإِنَّ`) splits across two units at the same origin and comes out
  `إِوَنَّ`.
- The bilingual table's heading is read left to right (its alignment signal is lost against the
  table below).

### Amendment (session 3): duplicates by position, and rotated pages

Duplicate units were first found within 8 characters of each other in PDFium's stream. On a
rotated page PDFium orders characters by their *displayed* position, so a letter and its dot
glyph can be far apart in the stream and both survived (`بتتاريخ` on a page turned 180°).
Duplicates are now found by position alone (a sweep over units sorted by their left edge; same
text, same line, overlapping by half the narrower), keeping the one PDFium listed first; the kept
unit's display box grows to cover the dropped one. That also fixed the table cell's doubled
letter (83/118). Text from a page rotated by 0°, 90°, 180°, or 270° is now identical
(`tests/view.rs`). One difference remains on rotated pages: where PDFium itself drops one of a
letter's two glyphs, which one depends on the rotation, so ~6% of letter boxes cover the dots
rather than the body.

Geometry for the UI goes through `safha_core::view::ViewTransform`, read from PDFium's own
page-to-device mapping (three sampled points), so selection and highlights follow `/Rotate`
and crop boxes that do not start at the origin.

Regression tests: `tests/text.rs` (floors per fixture, OCR flagging, directions, span integrity,
damaged files, 1,000-page speed) and `tests/search.rs`; `examples/fixture_report.rs --verbose`
shows every miss.

## Consequences

- Selection (M1) maps through `TextLine::spans`; copy uses `TextLine::text` (done: ADR 0010).
- Suspect pages need a UI affordance ("This page's text can't be read reliably — run OCR?") — M4
  supplies the OCR.
- Only top-level page objects are scanned for space glyphs; text inside form XObjects falls back
  to stream/gap spacing.
- More producers (Word, LibreOffice, XeLaTeX) are not in the corpus yet; each will likely need
  its own observation. The thresholds live in one `Params` struct.
- Copy from *other* viewers is a different matter (ADR 0005): this is about reading.
