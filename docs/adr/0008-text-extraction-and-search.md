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
exact. Remaining misses on trustworthy pages, recorded as floors in `crates/safha-core/tests/text.rs`
(the session 6 amendment below fixes the last two):

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

### Amendment (session 6): three of the remaining misses fixed, one ambiguity documented

The M1 exit test asks for correct Arabic copy and search on every §6.6 fixture. On the pages
whose text layer is trustworthy, three misses were left. Each was diagnosed from the glyphs
PDFium reports and the page's content stream:

- **The split Amiri cluster.** Fixed.
  - Cause: Chrome draws the waw and the hamza-below alef of `وَإِنَّ` from origins only
    0.05 pt apart, with the waw's ink to the right. Ordering by origin put the alef first.
  - Fix: units whose origins lie within 0.05 em of each other are ordered by their ink centres.
    Only exactly equal ones fall back to stream order.
- **The bilingual table's heading.** Fixed.
  - Cause: no other block overlaps the heading, so it formed its own column and gave no
    alignment signal. Its strong characters also tie, 13 Arabic to 13 Latin, and a tie meant
    left to right.
  - Fix: a block's direction now comes from the first signal that decides it:
    1. alignment within its column;
    2. a *clear* majority of strong characters (twice as many of one kind);
    3. alignment within the page's text area (the heading ends at its right edge);
    4. the page's majority.
- **Brackets.** Fixed.
  - Bracket pairs are now given their drawn shapes in *visual* order, where a pair always
    looks like `(…)`. They are mirrored after reordering where they sit at a right-to-left
    level (UAX #9 rule L4).
  - Before, the pairs were repaired after reordering. That failed when one bracket was mapped
    as typed and the other as drawn around left-to-right text: `bit-64( 2024)!`.
  - Unpaired brackets keep their mapping, as before.
- **`(64-bit)` inside an Arabic line comes out `(bit-64)`.** Not fixable from what the page
  shows.
  - Typed `64-bit` and typed `bit-64` are drawn identically in a right-to-left paragraph (UAX #9
    rule W7 makes the second one a single left-to-right run).
  - The same file's `INV-2026-00417` needs the other reading, and it is the more common shape
    (codes such as `COVID-19` and `ISO-9001`).
  - Chrome does draw each bidi run as its own `BT … ET`, which would tell the two apart.
    PDFium makes one text object per `Tj`, so that boundary is lost. Reading it would mean
    parsing content streams ourselves for one producer. Not done.

- **Chrome on Linux draws some word spaces inside another run.** Fixed.
  - What was seen: the first Linux CI run made the same fixtures with Linux Chrome. Vocalized
    Amiri lost five word spaces, all before `الْ`.
  - Cause: Linux Chrome draws a space in the same `Tj` as the glyph before it (`<00370001>`)
    wherever the space's offset equals that glyph's advance. Its float rounding makes this
    happen where macOS Chrome's does not.
  - Such a space is not a standalone space object, so it was not counted as a drawn space,
    and on a page that draws its spaces only drawn spaces break Arabic words.
  - Fix: PDFium reports such a space as a full-size character with a real advance, at exactly
    where macOS Chrome's separate space object sits. Spaces like that now count as drawn
    spaces too. The unreliable reports are the zero-width ones PDFium attaches to standalone
    space objects, and those are still ignored.

Results:
- 86 / 118 blocks verbatim on the macOS corpus (was 83), and **69 / 70 on pages with a
  trustworthy mapping** (was 67).
- 62 / 63 on the Linux-made corpus, which has no CoreText fixtures.
- Every miss left is either the ambiguity above or on a page flagged for OCR.
- CI runs the floors and the search test against each platform's own Chrome output.

## Consequences

- Selection (M1) maps through `TextLine::spans`; copy uses `TextLine::text` (done: ADR 0010).
- Suspect pages need a UI affordance ("This page's text can't be read reliably — run OCR?") — M4
  supplies the OCR.
- ~~Only top-level page objects are scanned for space glyphs~~ Form XObjects are scanned too,
  their children's bounds mapped through each form's matrix (ADR 0019). Flattened text boxes
  need this; the corpus is unchanged at 86 / 118.
- More producers (Word, LibreOffice, XeLaTeX) are not in the corpus yet; each will likely need
  its own observation. The thresholds live in one `Params` struct.
- Copy from *other* viewers is a different matter (ADR 0005): this is about reading.
