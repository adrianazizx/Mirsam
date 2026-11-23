# Spike: PDFium edit API vs. own writer for pre-shaped Arabic text

Throwaway M0 prototype. Question: can PDFium's own edit API (through `pdfium-render`) write
Arabic as pre-shaped glyph IDs (Identity-H CID font, embedded subset, correct ToUnicode,
`/ActualText` where needed, GPOS mark positioning), or do we have to write text objects and
content streams ourselves?

**Short answer.** PDFium's API can do it, but not the way the obvious calls suggest.
`FPDFText_LoadFont` + `FPDFText_SetText` fails completely for Arabic. `FPDFText_LoadCidType2Font`
(our own subset, ToUnicode and CIDToGIDMap) + `FPDFText_SetCharcodes` + `FPDFText_SetPositions`
produces output that renders pixel-identically to our own writer and extracts to byte-identical
strings in both extractors tested. What the API cannot do is place a glyph vertically inside a
text object, put a marked-content span around part of one, or let us choose operators and font
descriptor fields; the cost is many more text objects and 2 to 3 times the content bytes.

Text extraction, the acceptance test, does not separate the two routes. It is decided by the
content-stream recipe, and no recipe tried here round-trips all four inputs in either extractor.

A second experiment added the same line to page 1 of seven existing PDFs through both routes.
Both leave the existing content, pixels and PDFium text untouched. They differ at the edges:
PDFium keeps an encrypted file encrypted and refuses a truncated one; `lopdf` writes the
encrypted file back without encryption and accepts the truncated one, producing a file that
passes `qpdf --check` but has lost fonts.

Everything below was measured on 2026-11-22 with PDFium build 8125 (`third_party/pdfium/mac-arm64`),
pdfium-render 0.9.6, qpdf 12.4.2, macOS 26.5.2 (PDFKit), Swift 6.3.2.

## Run

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
cargo run -p spike-pdfium-edit            # from the repo root; about 18 s after the build
cargo run -p spike-pdfium-edit -- shape   # optional: dump the shaper's clusters and offsets
```

Needs `qpdf` and `swift` on `PATH`. Everything lands in `spikes/out/pdfium-edit/` (git-ignored):

- `<variant>-<font>.pdf`: four pages, one input line per page, 18 pt, RTL lines right-aligned.
- `<variant>-<font>.png`: the four pages stacked, rendered by PDFium through `mirsam-core` at 3x.
- `compare-<font>-<input>.png`: the same line in the six variants that can render differently,
  top to bottom: `a1-settext`, `a2-charcodes`, `a3-positions`, `a4-cid2`, `b-src`, `b-src-td`.
- `insert-src-<fixture>.pdf` and `insert-{a,a-inc,b}-<fixture>.pdf`: existing fixtures before and
  after a line was added (needs `fixtures/out`; cases whose fixture is missing are skipped).
- `ref-chrome-*.pdf`: Chrome's fixtures, copied so the same extractors run over them.
- `qdf/*.qdf.pdf`: `qpdf --qdf` copies for reading objects by eye.
- `results.md`, `results.json`: every number in this README, plus every mismatching string in full,
  the ToUnicode gaps, the `/ActualText` probes, the insertion measurements and Chrome's fixtures
  through the same extractors.

The generator binds PDFium directly (the edit API is not exposed by `mirsam-core`) and runs
rendering and PDFium text extraction in a child process through `mirsam-core`, because
pdfium-render allows one binding per process. PDFKit extraction is `scripts/pdfkit_extract.swift`.

## What was built

Shared by both routes (`shape.rs`, `fontinfo.rs`, `pdfparts.rs`): `unicode-bidi` for visual run
order, `harfrust` for shaping each run (cluster level "monotone characters", so marks keep their
own cluster where the font allows), `subsetter` for the font subset, a glyph to Unicode assignment
for the ToUnicode CMap, and a plan of which glyph ranges get an `/ActualText` span.

Route A, `route_a.rs`, PDFium edit API (raw bindings, except `a4-safe`):

| variant | calls |
|---|---|
| `a1-settext` | `FPDFText_LoadFont(cid=1)` + `FPDFText_SetText(logical string)` |
| `a2-charcodes` | `FPDFText_LoadFont(cid=1)` + `FPDFText_SetCharcodes(shaped glyph IDs)`, one object per line |
| `a3-positions` | A2 + `FPDFText_SetPositions` for x + a new text object at every change of y offset |
| `a4-cid2` | `FPDFText_LoadCidType2Font(subset, our ToUnicode, CIDToGIDMap)` + positions as A3 |
| `a4-cid2-at` | A4 + `/Span <</ActualText>>` on every base+marks group, via `FPDFPageObj_AddMark` + `FPDFPageObjMark_SetBlobParam` (UTF-16BE) |
| `a4-cid2-at8` | the same spans via `FPDFPageObjMark_SetStringParam` (UTF-8 with BOM) |
| `a4-safe` | `a4-cid2` again through pdfium-render's safe wrappers only, no `unsafe` |

Route B, `route_b.rs`, own writer on `lopdf`: Type0 font, CIDFontType2 descendant, Identity-H,
`/CIDToGIDMap /Identity`, FontDescriptor + FontFile2, `/W`, ToUnicode, one `BT..ET` per line.

| variant | recipe |
|---|---|
| `b-src` | `TJ` numbers for every x deviation, `Ts` for y offsets; ToUnicode maps each glyph to its source text |
| `b-src-rc` | + `/ReversedChars BMC..EMC` around RTL runs (as Skia writes) |
| `b-src-td` | `Td` before every repositioned glyph instead of `TJ`/`Ts` (as Skia writes) |
| `b-src-cluster` | + `/ActualText` only on clusters ToUnicode cannot reproduce |
| `b-src-grapheme` | + `/ActualText` on every base+marks group |
| `b-src-word`, `b-src-line` | + `/ActualText` per word, per line |
| `b-src-tuned` | + `/ActualText` pre-reversed and pre-mirrored for PDFium's extractor (not what the spec means) |
| `b-cmap` | ToUnicode from the font's cmap wherever the glyph is encoded (what PDFium's `LoadFont` and Skia do) |

Glyph IDs and Identity-H: `subsetter` renumbers glyphs (0 = .notdef, then in order of first use).
Both routes use the new ID as the CID. Route B declares `/CIDToGIDMap /Identity`; Route A has to
pass an explicit identity table because `FPDFText_LoadCidType2Font` requires map data.

## Results

All 44 generated PDFs (32 below plus 12 probe files) pass `qpdf --check`.

### Files

| variant | font | qpdf | file bytes | font program / original TTF | subset | text objects, 4 pages | content bytes, 4 pages |
|---|---|---|---:|---:|---|---:|---:|
| a1-settext | amiri | pass | 225,982 | 431,116 / 431,116 | NO | 4 | 1,493 |
| a2-charcodes | amiri | pass | 226,058 | 431,116 / 431,116 | NO | 4 | 1,457 |
| a3-positions | amiri | pass | 226,885 | 431,116 / 431,116 | NO | 20 | 4,636 |
| a4-cid2 | amiri | pass | 16,266 | 17,048 / 431,116 | yes | 20 | 4,176 |
| a4-cid2-at | amiri | pass | 16,524 | 17,048 / 431,116 | yes | 59 | 7,956 |
| a4-cid2-at8 | amiri | pass | 16,559 | 17,048 / 431,116 | yes | 59 | 7,795 |
| a4-safe | amiri | pass | 16,275 | 17,048 / 431,116 | yes | 20 | 4,189 |
| b-src | amiri | pass | 14,701 | 17,048 / 431,116 | yes | 4 | 1,731 |
| b-src-rc | amiri | pass | 14,796 | 17,048 / 431,116 | yes | 4 | 1,917 |
| b-src-td | amiri | pass | 14,809 | 17,048 / 431,116 | yes | 4 | 2,429 |
| b-src-cluster | amiri | pass | 14,755 | 17,048 / 431,116 | yes | 4 | 1,890 |
| b-src-grapheme | amiri | pass | 14,887 | 17,048 / 431,116 | yes | 4 | 3,394 |
| b-src-word | amiri | pass | 15,106 | 17,048 / 431,116 | yes | 4 | 3,608 |
| b-src-line | amiri | pass | 15,175 | 17,048 / 431,116 | yes | 4 | 2,895 |
| b-src-tuned | amiri | pass | 14,940 | 17,048 / 431,116 | yes | 4 | 3,508 |
| b-cmap | amiri | pass | 14,700 | 17,048 / 431,116 | yes | 4 | 1,731 |
| a1-settext | plex | pass | 114,621 | 235,924 / 235,924 | NO | 4 | 1,490 |
| a2-charcodes | plex | pass | 114,643 | 235,924 / 235,924 | NO | 4 | 1,421 |
| a3-positions | plex | pass | 115,421 | 235,924 / 235,924 | NO | 72 | 7,387 |
| a4-cid2 | plex | pass | 15,781 | 18,808 / 235,924 | yes | 72 | 7,387 |
| a4-cid2-at | plex | pass | 16,023 | 18,808 / 235,924 | yes | 85 | 9,763 |
| a4-cid2-at8 | plex | pass | 16,059 | 18,808 / 235,924 | yes | 85 | 9,602 |
| a4-safe | plex | pass | 15,789 | 18,808 / 235,924 | yes | 72 | 7,420 |
| b-src | plex | pass | 14,049 | 18,808 / 235,924 | yes | 4 | 2,382 |
| b-src-rc | plex | pass | 14,141 | 18,808 / 235,924 | yes | 4 | 2,568 |
| b-src-td | plex | pass | 14,202 | 18,808 / 235,924 | yes | 4 | 2,760 |
| b-src-cluster | plex | pass | 14,049 | 18,808 / 235,924 | yes | 4 | 2,382 |
| b-src-grapheme | plex | pass | 14,220 | 18,808 / 235,924 | yes | 4 | 3,889 |
| b-src-word | plex | pass | 14,460 | 18,808 / 235,924 | yes | 4 | 4,235 |
| b-src-line | plex | pass | 14,525 | 18,808 / 235,924 | yes | 4 | 3,546 |
| b-src-tuned | plex | pass | 14,367 | 18,808 / 235,924 | yes | 4 | 4,540 |
| b-cmap | plex | pass | 14,074 | 18,808 / 235,924 | yes | 4 | 2,382 |

Font program sizes are decoded bytes; the files are smaller because streams are compressed.
"Text objects" counts `BT` operators. In Route A that equals the PDFium page objects created.

### Rendering and extraction per input

Each cell is `render · PDFium extraction · PDFKit extraction`.

- Render: `ok` = joined letters, lam-alef formed, marks on their letters, mixed-direction order
  right. `ok*` = right glyphs in the right order but not the shaper's result (see the notes under
  the table). `NO` = wrong.
- Extraction: `yes` = the page text equals the logical input string exactly, after dropping the
  trailing line terminator. `NO (n)` = it differs; n is the edit distance in characters.

| variant | font | lam-alef | vocalized | mixed | latin |
|---|---|---|---|---|---|
| a1-settext | amiri | NO · NO (42) · NO (47) | NO · NO (73) · NO (78) | NO · NO (54) · NO (57) | ok* · yes · yes |
| a2-charcodes | amiri | NO · NO (42) · NO (47) | NO · NO (65) · NO (68) | NO · NO (42) · NO (47) | NO · NO (13) · NO (13) |
| a3-positions | amiri | ok · NO (29) · NO (42) | ok · NO (81) · NO (48) | ok · NO (36) · NO (34) | ok · NO (13) · NO (13) |
| a4-cid2 | amiri | ok · yes · yes | ok · NO (74) · NO (19) | ok · NO (10) · yes | ok · yes · yes |
| a4-cid2-at | amiri | ok · yes · yes | ok · NO (77) · NO (26) | ok · NO (10) · yes | ok · yes · yes |
| a4-cid2-at8 | amiri | ok · yes · yes | ok · NO (77) · NO (26) | ok · NO (10) · yes | ok · yes · yes |
| a4-safe | amiri | ok · yes · yes | ok · NO (74) · NO (19) | ok · NO (10) · yes | ok · yes · yes |
| b-src | amiri | ok · yes · yes | ok · NO (74) · NO (19) | ok · NO (10) · yes | ok · yes · yes |
| b-src-rc | amiri | ok · yes · yes | ok · NO (74) · NO (19) | ok · NO (10) · yes | ok · yes · yes |
| b-src-td | amiri | ok · yes · yes | ok · NO (77) · NO (26) | ok · NO (10) · yes | ok · yes · yes |
| b-src-cluster | amiri | ok · yes · yes | ok · NO (75) · NO (19) | ok · NO (10) · yes | ok · yes · yes |
| b-src-grapheme | amiri | ok · yes · yes | ok · NO (77) · NO (26) | ok · NO (10) · yes | ok · yes · yes |
| b-src-word | amiri | ok · NO (30) · yes | ok · NO (27) · NO (19) | ok · NO (38) · yes | ok · yes · yes |
| b-src-line | amiri | ok · NO (30) · yes | ok · NO (27) · NO (19) | ok · NO (44) · yes | ok · yes · yes |
| b-src-tuned | amiri | ok · yes · yes | ok · NO (77) · NO (26) | ok · NO (10) · NO (2) | ok · yes · yes |
| b-cmap | amiri | ok · yes · yes | ok · NO (74) · NO (19) | ok · NO (8) · NO (2) | ok · yes · yes |
| a1-settext | plex | NO · NO (42) · NO (42) | NO · NO (73) · NO (76) | NO · NO (54) · NO (54) | ok* · yes · yes |
| a2-charcodes | plex | ok · yes · NO (29) | NO · NO (19) · NO (37) | ok* · NO (8) · NO (27) | ok* · yes · yes |
| a3-positions | plex | ok · yes · NO (29) | ok · NO (142) · NO (48) | ok · NO (8) · NO (27) | ok · yes · yes |
| a4-cid2 | plex | ok · NO (14) · yes | ok · NO (145) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| a4-cid2-at | plex | ok · NO (14) · yes | ok · NO (113) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| a4-cid2-at8 | plex | ok · NO (14) · yes | ok · NO (113) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| a4-safe | plex | ok · NO (14) · yes | ok · NO (145) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| b-src | plex | ok · NO (14) · yes | ok · NO (145) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| b-src-rc | plex | ok · NO (14) · yes | ok · NO (145) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| b-src-td | plex | ok · NO (14) · yes | ok · NO (144) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| b-src-cluster | plex | ok · NO (14) · yes | ok · NO (145) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| b-src-grapheme | plex | ok · NO (14) · yes | ok · NO (113) · NO (23) | ok · NO (14) · yes | ok · yes · yes |
| b-src-word | plex | ok · NO (30) · yes | ok · NO (35) · NO (23) | ok · NO (38) · yes | ok · yes · yes |
| b-src-line | plex | ok · NO (30) · yes | ok · NO (72) · NO (23) | ok · NO (44) · yes | ok · yes · yes |
| b-src-tuned | plex | ok · NO (10) · yes | ok · NO (113) · NO (23) | ok · NO (14) · NO (2) | ok · yes · yes |
| b-cmap | plex | ok · yes · NO (29) | ok · NO (145) · NO (48) | ok · NO (8) · NO (27) | ok · yes · yes |

How the render column was filled:

- I looked at `b-src` for all eight font and input combinations (the `compare-*.png` sheets, and
  6x crops of the vocalized lines). The Amiri vocalized line matches the same sentence in Chrome's
  `fixtures/out/chrome/ar-vocalized.pdf` rendered by the same PDFium.
- Every other variant except A1 and A2 was compared with `b-src` pixel by pixel on the 3x renders.
  **No pixel differs by more than 32/255 in any channel on any page**, for A3, A4 and all Route B
  variants, in both fonts.
- A1 draws unjoined isolated letters, left to right in logical order, with marks at their default
  positions, and overflows the page. Its Latin line is correct but has no `fi`/`ff` ligatures (`ok*`).
- A2 draws the right glyphs, but every glyph outside the font's cmap advances by a full em.
  Amiri is unusable on all four lines, including gaps around the Latin ligatures. Plex, whose
  contextual forms are all encoded, is identical to `b-src` on lam-alef, differs only in spacing
  on mixed and latin (no GPOS kerning, `ok*`), and has misplaced marks on vocalized.

`a4-cid2`, `a4-safe` and `b-src` extract to byte-identical strings in both extractors for every
input and both fonts. So do `a4-cid2-at` and `b-src-grapheme`.

## Findings per claim

**"FPDFText_SetText does no shaping."** Confirmed. A1 comes out unjoined and visually reversed
(`compare-*-vocalized.png`, top row). PDFium extracts `.قلقلل يعاد الف ،بعلملا يف نآلا دالوألا نأل ؛سأب ال`
for the lam-alef line: it sees right-to-left letters laid out left to right and "corrects" them.

**"FPDFText_LoadFont embeds the whole font."** Confirmed: 431,116 of 431,116 bytes for Amiri,
235,924 of 235,924 for Plex. Two further problems came with it:

- Its `/W` array only covers glyphs the cmap reaches; everything else falls back to the default
  width of 1000. That is what breaks A2's spacing. A3 hides it because `FPDFText_SetPositions`
  makes PDFium emit compensating `TJ` numbers, for example `<06f0> 814 <005e>`.
- Its ToUnicode has an entry only for cmap-encoded glyphs: 1,622 entries for Amiri's 6,710 glyphs.
  Of the 105 Amiri glyphs the shaper used, 47 have an entry and 33 map to the text they stand
  for. Glyphs without one extract as their glyph number read as a code point: `ሒᇭ ا৊ৎܹأص، ڣఈః داሒᇼ`.
  Plex encodes its contextual forms, so 84 of 86 used glyphs have an entry, but only 44 map to the
  source text: the rest are presentation forms (U+FExx), `fi` as U+FB01, and swapped brackets.

**"FPDFText_SetCharcodes may allow glyph IDs."** Confirmed, it does. With a CID font the charcode
is the CID, and every call returned success.

**"...but gives no per-glyph offsets."** Wrong for x, right for y. `FPDFText_SetPositions` exists
in this build (pdfium-render gates it to PDFium 7881 and later) and takes one x position per glyph
after the first, in points. There is no y. The workaround of one text object per stretch of equal
y offset works, at this cost for the vocalized line:

| | Amiri | Plex |
|---|---:|---:|
| text objects, `a4-cid2` | 11 | 63 |
| text objects, `a4-cid2-at` | 50 | 76 |
| content bytes, `a4-cid2` | 2,070 | 5,318 |
| content bytes, `b-src` (one `BT`) | 943 | 1,583 |

PDFium wraps every object in `q .. BT <matrix> Tm /F Tf 0 Tr [..] TJ ET Q` and repeats the colour
operators. The mixed line has 7 objects in Route A only because the spike starts a new one per
bidi run; that part is my choice, not an API limit.

**"ToUnicode coverage for ligature/contextual glyphs is doubtful."** True for `FPDFText_LoadFont`
(above). Not a limit of the API as a whole: `FPDFText_LoadCidType2Font` takes our own ToUnicode
CMap, CIDToGIDMap and font program and writes them through. All 105 (Amiri) and 86 (Plex) used
glyphs are mapped to their source text, and the embedded program is our 17,048 / 18,808 byte
subset. This was the result that was better than expected.

Limits of that call: the font dictionary is PDFium's. `/BaseFont /Amiri-Regular` has no subset tag,
`/CapHeight` equals `/Ascent`, `/Flags 32`, `/StemV 70`, and none of it is settable.

**/ActualText through the API.** `FPDFPageObj_AddMark("Span")` + `FPDFPageObjMark_SetBlobParam`
writes `/Span <</ActualText<feff06290650>>> BDC`. A mark belongs to a whole page object, so a span
around three glyphs costs a separate text object; `FPDFPageObj_AddExistingMark` lets consecutive
objects share one span. `FPDFPageObjMark_SetStringParam` cannot carry UTF-16 because the binding
passes a C string. A UTF-8 string with BOM (`<efbbbf...>`) extracted identically to the UTF-16
blob in both extractors.

**pdfium-render.** Its safe API wraps `load_cid_type2_from_bytes`, `set_charcodes` and
`set_positions`. `a4-safe` uses only those and matches `a4-cid2`: same object counts, no pixel
difference, identical extracted strings. So the A4 recipe needs no `unsafe`, which matters
because the workspace denies `unsafe_code`. There is no wrapper for adding marks or setting
their parameters, and the `FPDF_*` handles are private, so safe and raw calls cannot be mixed on
one object: `/ActualText` spans would need raw calls. `FPDF_FONT_TRUETYPE` is not re-exported.

## What extraction actually does

This section matters more than the route choice. Full strings are in `results.md`.

### Baseline: Chrome's own PDFs do worse

The three Arabic inputs appear verbatim in `fixtures/out/chrome/{ar-lam-alef,ar-vocalized,mixed-inline}.pdf`.
Through the same extractors (pages hold several paragraphs, so this is a qualitative comparison):

- PDFium, lam-alef sentence: split over six lines, `فلا يعاد لل` / `يف بعلملا،` / `ال أولاد آلا` ...
- PDFium, vocalized sentence: `َة مِفْتَحاُ ا ْلَمْعِر َفِة.` / `َء` / `إِ نَّ ا ْلِق رَا` / `، َوٌرقَلاَ ا ْلُمَعِّل ُم: إِنَّ ا ْلِعْل َم نُو`
- PDFKit, vocalized sentence: mostly spaces and marks, the base letters are missing.

So "exact match" is a much higher bar than what Chrome's Skia backend reaches, and `b-src`
already clears it for unvocalized Arabic.

### PDFium (`page.text().all()`)

1. It assumes right-to-left glyphs are in visual order and reverses them. With visual-order glyphs
   and one glyph per character this is exact: Amiri lam-alef matches in every ToUnicode-only variant.
2. A ToUnicode entry with several characters is reversed along with everything else. Plex's seven
   lam-alef ligature glyphs mapped to `لا` come back as `ال`: `ال بأس؛ ألن األوالد اآلن يف الملعب، فال داعي للقلق.`
   Mapping the glyph to its presentation-form code point instead (U+FEFB, what `b-cmap` and
   `FPDFText_LoadFont` do) is exact, because PDFium normalises presentation forms.
3. It mirrors brackets in right-to-left text, so it wants the drawn bracket in ToUnicode.
   `b-src` (typed bracket) gives `)شاملة الضريبة(`; `b-cmap` gives `(شاملة الضريبة)`.
4. It reverses the order of left-to-right words inside a right-to-left line: `Pro 15 iPhone`, in
   every variant and in Chrome's own fixture. That alone is the remaining distance of 8 on `mixed`
   with `b-cmap`. Nothing the writer does fixed it.
5. Marks positioned by GPOS break it. The Amiri vocalized line gains 3 line breaks and 8 extra
   spaces (16 instead of 8); the Plex one gains 41 line breaks. The breaks sit at marks with a
   vertical offset, the spaces at marks reached by a horizontal `TJ` jump. The same glyphs
   without positions (`a2-charcodes`, Plex) extract on one line with distance 19. Correct mark
   placement and clean PDFium extraction pull against each other; `Td` instead of `TJ`/`Ts`
   was no better.
6. `/ActualText` is honoured for a span of any length, but its text goes through the same
   visual-to-logical pass. The probe files show it: a line-level `/ActualText (PROBE)` extracts as
   `EBORP` on the three right-to-left pages and `PROBE` on the Latin one. So spec-conformant,
   logical-order ActualText makes PDFium worse: `b-src-word` turns the exact Amiri lam-alef line
   into `ال ؛سأب نأل دالوألا ...` (distance 30). Per-group spans (`b-src-grapheme`) did not remove the
   line breaks and added swaps such as `قَلاَ` for `قَالَ`.
7. Pre-reversing the ActualText for PDFium (`b-src-tuned`) is not reliable either: it fixed 2 of
   the 7 Plex ligatures.
8. `/ReversedChars` changes nothing: `b-src-rc` extracts to the same strings as `b-src`.

### PDFKit (`PDFPage.string`)

1. Visual-order glyphs with source-text ToUnicode are exact for lam-alef, mixed and latin in both
   fonts, including multi-character entries for ligatures and the left-to-right run.
2. It does not mirror brackets, so it wants the typed bracket: the opposite of PDFium. `b-cmap`
   gives `)شاملة الضريبة(` here.
3. It does not normalise presentation forms: `b-cmap` with Plex returns `ﻻ ﺑﺄس؛ ﻷن ...`.
4. The vocalized line comes back in the right order with extra spaces around repositioned marks:
   `ق َالَ الْم ُعَلِّمُ: إ ِ نّ َ ...` (21 spaces instead of 8, distance 19).
5. `/ActualText` is used only when the span holds exactly one glyph and the text is exactly one
   character. Probe evidence: per-line and per-word markers are ignored; with one span per cluster,
   one-glyph clusters come back as `x` while two-glyph clusters keep their ToUnicode text; a
   five-character marker on one glyph is ignored.
6. `/ReversedChars` changes nothing.

### Consequences

- ToUnicode has to carry the text. ActualText cannot repair order or ligatures in either
  extractor, and in PDFium it does damage inside right-to-left text.
- The two extractors want opposite ToUnicode values for mirrored brackets and for ligature glyphs.
  `b-src` is exact in PDFKit on three of four inputs for both fonts. `b-cmap` is best in PDFium.
  No single mapping satisfied both.
- Fonts that shape lam-alef as two glyphs (Amiri) sidestep the ligature problem.
- For vocalized text and for left-to-right runs inside right-to-left lines, PDFium's raw text
  will not match whatever we write. The product's own bidi-aware extraction (planned for M1)
  has to handle those, as it must for third-party PDFs.

## Inserting into an existing page

The recommendation depends on how new text gets into a document the editor already has open, so
both routes also added the vocalized line (Amiri, 14 pt) near the bottom of page 1 of seven
fixtures this spike did not write: two Chrome prints, one CoreText print, and the linearized,
AES-256 encrypted, truncated and 1,000-page derived files.

- Route A: `FPDF_LoadMemDocument`, the A4 calls, `FPDFPage_GenerateContent`, `FPDF_SaveAsCopy`
  with `FPDF_NO_INCREMENTAL` and again with `FPDF_INCREMENTAL`.
- Route B: `lopdf` parses the file, the font objects and one content stream are appended
  (a `q` stream in front of the page's own streams and `Q` at the start of ours, so the page's
  leftover graphics state cannot reach the new text), and the whole file is written out.

| source | route | result | bytes before → after | ms | qpdf | content streams, page 1 | text objects added | output encrypted |
|---|---|---|---|---:|---|---:|---:|---|
| chrome-vocalized | A4, full save | ok | 18,988 → 25,630 | 1 | pass | 2 | 11 | no |
| chrome-vocalized | A4, incremental save | ok | 18,988 → 44,612 | 1 | pass | 2 | 11 | no |
| chrome-vocalized | B, lopdf load + save | ok | 18,988 → 24,894 | 0 | pass | 3 | 1 | no |
| chrome-table | A4, full save | ok | 67,598 → 72,943 | 4 | pass | 2 | 11 | no |
| chrome-table | A4, incremental save | ok | 67,598 → 140,536 | 3 | pass | 2 | 11 | no |
| chrome-table | B, lopdf load + save | ok | 67,598 → 71,432 | 1 | pass | 3 | 1 | no |
| coretext-vocalized | A4, full save | ok | 18,685 → 25,195 | 1 | pass | 2 | 11 | no |
| coretext-vocalized | A4, incremental save | ok | 18,685 → 43,874 | 1 | warning | 2 | 11 | no |
| coretext-vocalized | B, lopdf load + save | ok | 18,685 → 24,749 | 0 | pass | 3 | 1 | no |
| linearized | A4, full save | ok | 36,197 → 41,294 | 2 | pass | 2 | 11 | no |
| linearized | A4, incremental save | ok | 36,197 → 77,483 | 2 | pass | 2 | 11 | no |
| linearized | B, lopdf load + save | ok | 36,197 → 40,606 | 0 | pass | 3 | 1 | no |
| encrypted | A4, full save | ok | 38,757 → 44,572 | 7 | pass | n/a | 11 | yes |
| encrypted | A4, incremental save | ok | 38,757 → 83,323 | 6 | pass | n/a | 11 | yes |
| encrypted | B, lopdf load + save | ok | 38,757 → 40,653 | 5 | pass | 3 | 1 | no |
| truncated | A4, full save | failed: `FPDF_LoadMemDocument` error 3 | 63,575 → n/a | 0 | n/a | | | |
| truncated | A4, incremental save | failed: `FPDF_LoadMemDocument` error 3 | 63,575 → n/a | 0 | n/a | | | |
| truncated | B, lopdf load + save | ok | 63,575 → 70,632 | 1 | pass | 3 | 1 | no |
| 1000-pages | A4, full save | ok | 570,899 → 549,513 | 35 | pass | 2 | 11 | no |
| 1000-pages | A4, incremental save | ok | 570,899 → 1,120,408 | 25 | pass | 2 | 11 | no |
| 1000-pages | B, lopdf load + save | ok | 570,899 → 530,302 | 13 | pass | 3 | 1 | no |

Times are wall-clock for load + insert + save, one run each, in a debug build of the spike with
optimised dependencies. They vary by a millisecond or two between runs, as do the encrypted
sizes by a byte. Treat them as an order of magnitude.

What held in every case where a route produced a file and PDFium could open the source (18 of 19
outputs):

- The page's decoded content stream bytes are still present verbatim. PDFium adds one stream
  after the original; it did not rewrite it. (Not checkable for the encrypted A outputs, which
  `lopdf` could not read back with the password.)
- No pixel above the new line changed (threshold 32/255, 3x render). 6,837 pixels changed in the
  line's own band in every output, and the full-page PNGs of the A and B outputs for
  `chrome-vocalized` are byte-identical. The line is upright although Chrome's page leaves a
  flipped CTM behind.
- PDFium's extracted text of the page still contains the original text as one contiguous piece.
- PDFKit's does too, except on `chrome-table`, where all three outputs return the same lines in a
  different order. The table cells move in PDFKit's reading order when a line is added below them,
  identically for both routes.

Where the routes differ:

- **Encryption.** PDFium's output is still AES-256 encrypted and opens with the same password.
  `lopdf` decrypts on load and wrote a file with no encryption. It did so silently; re-encrypting
  is possible in its API but was not tried.
- **Damaged input.** PDFium refuses the truncated file (error 3, bad format), which is also what
  the product's engine does today. `lopdf` loads it and writes a one-page file that passes
  `qpdf --check` and that PDFium then opens, with the Latin text coming out as `Æ- ÅÌ ËÂ15 - Ï Ì`:
  the fonts that were cut off are gone. With Route B the two parsers disagree about what is a
  valid document, so the editor would have to make PDFium's verdict the gate.
- **Incremental save.** With `FPDF_INCREMENTAL` the output is the original bytes followed by as
  many object definitions as the full save writes in total (39 and 39 on `chrome-vocalized`,
  1,358 and 1,358 on the 1,000-page file), so the file doubles instead of growing by a small
  delta. On the CoreText source the result made qpdf warn "object has offset 0". This call did
  not give a usable incremental update in any of the six cases.
- **Size and object count.** Route B adds one text object against eleven and less content; its
  single-page outputs are 0.4 to 1.5 KB smaller than PDFium's full save.

Limits of this experiment: it appends one line. It does not modify or remove existing page
objects, which is where `FPDFPage_GenerateContent` has to rewrite existing streams.

## Effort and sharp edges

Code size, formatted, comments included:

| part | lines | needed by |
|---|---:|---|
| `shape.rs` (bidi + shaping to visual-order glyphs) | 134 | both |
| `fontinfo.rs` (metrics, reverse cmap) | 106 | both |
| `pdfparts.rs` (subset, ToUnicode assignment, span planning) | 329 | both, from A4 on |
| `route_a.rs`: raw writer for all A modes 115, positioned placement 110, insertion 110, safe variant 55, types 65 | 456 | A |
| `route_b.rs`: font objects 45, new-document writer 55, insertion 60, content emitter 185, types 50 | 396 | B |

The two emitters are comparable in size. Everything hard (shaping, bidi, subsetting, deciding
what each glyph maps back to) is shared and has to be ours on either route.

Route A:

- The raw calls are all `unsafe` FFI, including a C callback for `FPDF_SaveAsCopy`. The safe
  wrappers cover the A4 recipe except content marks.
- No operator-level control: no `Ts`, no span inside an object, no choice of `TJ` against `Td`,
  no font descriptor fields, no subset tag.
- f32 positions leave noise such as `-.00021192763` in the `TJ` arrays.
- Editing needs the process's single PDFium binding, so `mirsam-core` would have to expose it.
- `FPDF_INCREMENTAL` does not give a small incremental update (see above).

Route B:

- I had to learn that `TJ` numbers are in thousandths of an em and move left when positive, and
  track the viewer's pen (which advances by `/W`, not by the shaper's advance) to place marks.
- The first ToUnicode assignment mapped Amiri's shadda glyph to fatha, because both sat in one
  cluster. Consulting the font's cmap before splitting cluster text fixed it. A real implementation
  needs a deliberate rule here.
- `lopdf` brings AES, MD5, SHA-2 and `rand` in unconditionally.
- `lopdf` drops encryption on save and accepts a truncated file without complaint (see above).

Spike shortcuts a real engine cannot take: one font per line with no fallback, runs split by bidi
level only (no script itemisation), no line breaking, no justification or kashida, `/BaseFont`
from the file name with a fixed subset tag, CFF fonts untested.

## Recommendation

1. **Own the font and glyph pipeline regardless of route**: shaping, bidi, subsetting, ToUnicode
   assignment. `FPDFText_LoadFont` and `FPDFText_SetText` are not usable for Arabic at all.
2. **Emit through PDFium's edit API with the A4 recipe** (`load_cid_type2_from_bytes` +
   `set_charcodes` + `set_positions` + one object per y offset), using pdfium-render's safe
   wrappers. Evidence: it matches the own writer pixel for pixel and string for string, it needs
   no `unsafe`, it adds text to existing pages without disturbing them, and it keeps the document
   in the one parser the product already trusts (encryption preserved, damaged input refused).
   The price is more text objects, 2 to 3 times the content bytes for vocalized text, and a font
   dictionary we do not control.
3. **Do not rely on /ActualText for extraction.** Write ToUnicode-only content in visual order.
   ActualText repaired nothing in either extractor and made PDFium worse. That also removes the
   one thing the safe wrappers cannot do.
4. **Keep the pipeline backend-neutral** (a line of positioned glyphs plus a font bundle in, an
   emitter behind it) and keep the own writer as the fallback. Switch to it if we need control
   PDFium does not give: font descriptor and subset tag (PDF/A), spans or structure finer than a
   page object, a different positioning recipe once more extractors are tested, or a true
   incremental save. If we do, `lopdf` must not be the thing that decides whether a file is
   valid, and its encryption handling has to be dealt with first.

Caveats:

- The insertion test only appended a line. Editing or deleting existing text through PDFium, and
  what `FPDFPage_GenerateContent` does to existing streams then, was not tested. That is the next
  thing to check before committing M3 to this route.
- The ToUnicode policy (source text against cmap convention) is an open product decision between
  PDFium and PDFKit behaviour; see "Consequences".
- Everything here is one PDFium build, two fonts, four lines of text.

## Not verified

- Acrobat, Poppler, pdf.js or any extractor other than PDFium 8125 and macOS 26.5.2 PDFKit.
- Modifying or removing existing page objects; undo; saving the same document repeatedly.
- Performance beyond the single-run timings above; memory.
- Re-encrypting with `lopdf`, and `lopdf` on other kinds of damage than this one truncated file.
- The size of the horizontal jump at which the extractors insert a space. The spaces line up with
  the larger jumps in the Amiri stream, but I did not measure a threshold.
- Other fonts (variable fonts, CFF outlines, fonts with cursive attachment such as Nastaliq),
  kashida, vertical text, text longer than one line.
- PDF/A or PDF/UA conformance of either output; only `qpdf --check` was run.

## Dependencies added

Direct, in `spikes/pdfium-edit/Cargo.toml`:

| crate | version | licence | used for |
|---|---|---|---|
| harfrust | 0.16.1 | MIT | shaping (rustybuzz was not needed) |
| unicode-bidi | 0.3.18 | MIT OR Apache-2.0 | bidi runs |
| subsetter | 0.2.6 (default features off) | MIT OR Apache-2.0 | font subset |
| lopdf | 0.47.0 (default features off) | MIT | Route B writer, and reading every output back |
| png | 0.18.1 | MIT OR Apache-2.0 | PNG output (already in the lock file) |
| serde_json | 1.0.154 (workspace) | MIT OR Apache-2.0 | result files (already in the lock file) |

New transitive entries in the root `Cargo.lock` (which this spike necessarily changed):

- MIT OR Apache-2.0: aes 0.9.4, block-padding 0.4.2, cbc 0.2.1, chacha20 0.10.2, cipher 0.5.2,
  core_detect 1.0.0, cpubits 0.1.1, ecb 0.2.1, font-types 0.12.7, inout 0.2.6, md-5 0.11.0,
  multiversion_no_op 1.0.0, rand 0.10.4, rand_core 0.10.1, rangemap 1.8.0, read-fonts 0.47.0,
  simdutf8 0.1.5, stringprep 0.1.5, weezl 0.2.1
- MIT: nom 8.0.0
- Zlib OR Apache-2.0 OR MIT: bytemuck_derive 1.12.1
- (Apache-2.0 OR MIT) AND BSD-3-Clause: encoding_rs 0.8.44
- **(MIT OR Apache-2.0) AND Unicode-3.0: unicode-properties 0.1.5**, pulled in by
  lopdf -> stringprep. Unicode-3.0 is permissive but not on the allowed list as written. The lock
  file already held 20 crates with a Unicode licence term before this spike (the `icu_*` family,
  `unicode-ident`, `unicode-normalization` and others), so this is a question about the list, not
  a new kind of dependency. It only matters if `lopdf` goes into the product.

No GPL, LGPL or AGPL code. Only the Route B writer and the spike's own output inspection need
`lopdf`; the recommended route needs harfrust, unicode-bidi and subsetter.
