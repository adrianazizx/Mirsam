# Check: does PDFium regenerate page content faithfully?

Throwaway M3 check, the open item in ADR 0005 ("Before M3 commits"). Question: when existing
page objects are deleted or replaced through PDFium's edit API and `FPDFPage_GenerateContent`
rewrites the page's content stream, does everything else on the page survive — clipping,
z-order, marked content, unusual operators?

**Short answer: no.** On 27 of 34 pages the rewrite silently damages content the edit never
touched, and `qpdf --check` passes on every damaged file. Appending (the M0 spike's case) is
safe: PDFium leaves the existing stream alone and adds a new one.

Measured on 2026-11-22 with PDFium build 8125 (`third_party/pdfium/mac-arm64`), pdfium-render
0.9.6, qpdf 12.4.2.

## Run

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
node fixtures/generate.mjs                 # the corpus, if fixtures/out/ is missing
cargo run -p spike-pdfium-regenerate       # about 80 s in a debug build
```

Optional: `SPIKE_ONLY=<name part>` limits the inputs; `SPIKE_DUMP=<output name part>` also writes
the three renders (`original`, `intended`, `reloaded`) of matching outputs as `.ppm`.

Outputs land in `spikes/out/pdfium-regenerate/` (git-ignored): every edited PDF,
`report.json` (all measurements, including the marked-content runs before and after), and
`stress.pdf`.

## Method

Inputs: every fixture in `fixtures/out/manifest.json` (first three pages of multi-page files),
plus `stress.pdf`, a hand-written page with what the corpus lacks: a clip path around
overflowing text, text render mode 7 with a shading painted through it, `/P <</MCID 0>>` with a
structure tree, optional content by property-list name, an `/ActualText` span, an `/Artifact`,
an inline image, a form XObject, an ExtGState alpha, `Tc`/`Tw`/`Tz`/`TL`/`T*`/`'`, a dash pattern,
and `BX … EX` around an unknown operator.

Five edits per page, each on a fresh copy, with the page's regeneration strategy set to manual:

| Edit | What it does | Expected visible change |
|---|---|---|
| `dirty` | apply the identity matrix to the middle text object (marks the stream dirty) | none |
| `delete` | remove the middle text object | that object only |
| `reinsert` | remove it and insert the same object back at its index | none |
| `replace` | remove it and insert an opaque box at its index | that object only |
| `append` | add a box on top (the M0 spike's case) | the box only |

**The oracle.** PDFium renders a page from its in-memory object list, so the page rendered after
the edit but *before* saving is what PDFium believes the page is. Each edit is then regenerated,
saved with `save_to_bytes`, reopened, and compared with that:

- pixels at 2× (a channel difference above 32/255 counts, as in ADR 0005);
- the object list: type, bounds, and every clip path (segment count and bounding box);
- PDFium's page text, and `mirsam-core`'s logical text through the product's own extractor;
- the content stream read independently with `lopdf`: operators present, the marked-content
  path of every painting operator, the set of MCIDs, and whether every BDC property-list name
  is still defined in `/Properties`;
- `qpdf --check`.

## Results

"Intact" means the regenerated page renders exactly as PDFium intended and its object list,
clips, and text are unchanged.

| Pages | dirty | delete | reinsert | replace | append |
|---|---|---|---|---|---|
| Chrome, 8 | 2 intact | 2 | 2 | 2 | 8 (existing content untouched) |
| Chrome-made derived files (linearized, encrypted, 1,000-page, page labels, rotated), 11 | 0 | 0 | 0 | 0 | 11 |
| CoreText, 8 | 1 | 1 | 1 | 1 | 8 |
| `rtl-book` and its object-stream copy (Chrome pages: page 1 has Type 3 fonts, pages 2–3 only `Type0`), 6 | 4 | 4 | 4 | 4 | 6 |
| Stress page, 1 | 0 | 0 | 0 | 0 | 1 |
| **All, 34** | **7** | **7** | **7** | **7** | **34** |

The truncated fixture is refused by PDFium (`FormatError`), as it should be.

### What breaks, by cause

1. **Type 3 fonts: all text vanishes.** For every text object in a Type 3 font, the generator
   writes `q … BT <matrix> Tm` and stops: no `Tf`, no glyphs, no `ET`, no `Q`. Reopened, the page
   has **0 objects**. Chrome writes Type 3 fonts for the variable fonts in the corpus, so 19 of
   the 34 pages are hit. Chrome pages whose fonts are all `Type0`/`CIDFontType2`
   (`ar-sans`, `ar-vocalized`) survive.
2. **Simple fonts that share a `/BaseFont` are merged.** CoreText splits one face into several
   simple TrueType subsets with the same name and different encodings. The generator keys
   font resources by name and type, so the second subset's glyphs are drawn with the first
   subset's encoding: Arabic becomes tofu and wrong letters (7 CoreText pages; the English-only
   page, with one subset, survives).
3. **Stress page** (no font problem; Helvetica only):
   - the inline image is dropped;
   - `sh` is dropped, and text render mode 7 is lost, so the shading painted through "SHADE"
     disappears;
   - `Tc`, `Tw` and `Tz` are lost: letter-spaced, word-spaced, 150%-wide text comes back plain;
   - `BX … EX` and the unknown operator inside are dropped (harmless).

### What survives (on pages without the font problems)

- Clip paths: every surviving object keeps an identical clip.
- Z-order: `reinsert` and `replace` keep the object order exactly.
- Marked content: MCIDs (none lost on any page), optional content by property-list name with
  `/Properties` still defined, `/ActualText` spans, `/Artifact`.
- Form XObjects, ExtGState alpha, fill and stroke colours.
- `append`: the existing stream is left byte for byte (one content stream becomes two). The only
  pixel differences (at most 247) are anti-aliasing on the edge of the new box itself.

The regenerated stream wraps every object in its own `q … Q` with the full graphics state, so
it is about twice the size of the original (6.5 KB → 12.9 KB for `chrome/en-paragraphs`).

## Consequence

PDFium must never regenerate a content stream that holds existing content. Recorded in ADR 0005
(amendment of 2026-11-22) together with what that means for where text emission lives.
