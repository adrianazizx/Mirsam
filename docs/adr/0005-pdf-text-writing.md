# 0005 — Writing text: our own shaping pipeline, emitted through PDFium's CID-font API

Status: Accepted (Milestone 0 spike), **amended 2026-11-22 (M3)**: PDFium's content regeneration
proved lossy, so text is now emitted by our own writer — see the amendment at the end. Date:
2026-11-22.

## Question

The spec (§6.3, §9) requires text to be written as pre-shaped glyph IDs in an Identity-H CID font
with an embedded **subset**, a correct ToUnicode CMap, and `/ActualText` where needed. It asks for
a spike: can PDFium's edit API place pre-shaped glyph IDs with custom fonts, or must we write text
objects ourselves?

The working assumption going in — in the spec's risk table and in this project's own early notes —
was that PDFium's API would be too limited. **The spike says otherwise.**

## Method

`cargo run -p spike-pdfium-edit` (18 s) writes four logical-order inputs — lam-alef ligatures,
fully vocalized text, a mixed Arabic/English/number line, and a Latin line with fi/ff ligatures —
in two fonts (Amiri, IBM Plex Sans Arabic) through sixteen variants of two routes, then for each
output: `qpdf --check`, sizes, a render through PDFium compared pixel by pixel, and text
extraction by two independent extractors (PDFium, which is also Chrome's and Edge's; and macOS
PDFKit) compared with the input string. It also inserts a line into seven existing fixtures
through both routes. Full tables and every mismatching string: `spikes/pdfium-edit/README.md`
and `spikes/pdfium-edit/results-2026-11-22.md`.

- **Route A — PDFium's edit API** via `pdfium-render`.
- **Route B — own writer** on `lopdf`, with `harfrust` shaping, `unicode-bidi`, and `subsetter`.

Both routes share the shaping, bidi, and subsetting code; they differ only in who emits the PDF.

## Findings

1. **The obvious PDFium route fails exactly as feared.** `FPDFText_LoadFont` + `FPDFText_SetText`
   draws unjoined letters in logical order, left to right — the Acrobat-style failure the product
   exists to avoid. `LoadFont` also embeds the whole font (431 KB for Amiri) and generates a
   ToUnicode map covering only cmap glyphs: of the 105 Amiri glyphs the shaper used, 33 mapped to
   the right text.
2. **A different PDFium route works.** `FPDFText_LoadCidType2Font` accepts *our* font subset, *our*
   ToUnicode CMap, and *our* CIDToGIDMap; `FPDFText_SetCharcodes` places glyph IDs;
   `FPDFText_SetPositions` (present in build 8125) sets per-glyph x positions. Vertical offsets
   need one text object per distinct y, which is how tashkeel gets positioned.
3. **Output quality is identical.** That recipe and the own writer render with no pixel differing
   by more than 32/255 at 3× on any page, in both fonts, and produce byte-identical extracted
   strings in both extractors. The embedded font is the 17–19 KB subset either way; files are
   14–16 KB against ~226 KB for the naive route.
4. **No `unsafe` is needed.** The whole recipe runs through `pdfium-render`'s safe wrappers
   (`load_cid_type2_from_bytes`, `set_charcodes`, `set_positions`); only content marks lack one.
5. **PDFium is the safer serializer for user files.** Inserting into existing PDFs:
   - PDFium kept an AES-256 file encrypted and refused a truncated file.
   - `lopdf` wrote the encrypted file back **unencrypted, silently**, and accepted the truncated
     file, producing output that passes `qpdf --check` but has lost its fonts.
   Both are unacceptable under "never lose or corrupt a file", and argue against a second
   general-purpose PDF parser in the save path.
6. **Costs of the PDFium route:** more text objects (11 for the vocalized line in Amiri, 63 in
   Plex) and 2–3× the content-stream bytes for heavily marked text; no control over the font
   descriptor; and `FPDF_INCREMENTAL` does not yield a real incremental update (it appends a full
   rewrite of every object).

### Extraction: the spec's assumption does not hold

The spec expects ToUnicode plus `/ActualText` to make our output copy correctly in other
viewers. Measured:

- **No variant on either route round-trips all four inputs in either extractor.** Lam-alef and
  Latin can be made exact; fully vocalized text is never exact (edit distance 19–145); the mixed
  line is exact in PDFKit but not PDFium.
- **`/ActualText` does not help.** PDFium honours it but reverses it inside RTL text (a span
  marked `PROBE` extracts as `EBORP`), so correct logical-order ActualText makes PDFium *worse*.
  PDFKit uses it only when a span is exactly one glyph and one character. `/ReversedChars` changes
  nothing in either.
- **The two extractors want opposite ToUnicode conventions** for ligatures and mirrored brackets.
  Mapping glyphs to source text is best for PDFKit; following the font's cmap is best for PDFium.
- **Two PDFium failures are beyond any writer's reach**: it reverses LTR word order inside RTL
  lines (`Pro 15 iPhone`), and GPOS-positioned marks break its line detection.
- For calibration: **Chrome's own PDFs of the same sentences extract worse** than ours in both
  extractors.

So "copies correctly in other viewers" is a property of the *reader*, and only partly ours to
give. Inside Safha, correct copy and search come from our own reading-order reconstruction (M1,
spec §6.1), not from trusting any extractor.

## Decision

1. **Own the text pipeline in `safha-core`**: itemize → shape (`harfrust`) → bidi
   (`unicode-bidi`) → subset (`subsetter`) → ToUnicode → positioned glyph runs. It is needed on
   either route and is where the Arabic correctness lives. Its output is a backend-neutral list
   of positioned glyph runs.
2. **Emit new text through PDFium's CID Type 2 font API**, using the safe wrappers. One engine
   parses, edits, and serializes user files.
3. **Write ToUnicode-only content; do not rely on `/ActualText`.** It costs complexity and
   measurably harms the most widely used extractor.
4. **Keep an own content-stream writer as a planned second backend**, not a rejected option. It
   is required for true incremental saves (existing digital signatures, M5) and for
   font-descriptor control (PDF/A). `lopdf` must not parse or re-serialize arbitrary user files
   without guards for encrypted and damaged input.
5. **Shaper: `harfrust`**, not `rustybuzz` (no release since November 2024).

This partly reverses the spec's starting choice ("write text objects ourselves"); it follows the
spec's own instruction to decide by spike.

## Before M3 commits

The spike only **appended** text. Not verified: deleting or replacing existing page objects, and
what `FPDFPage_GenerateContent` does to a page's existing content stream when it regenerates —
whether clipping, z-order, marked content, and unusual operators survive. In-place editing (spec
§7 step 5) depends on exactly that. Test it on the fixture corpus first; if regeneration is
lossy, in-place edits take the own-writer backend for that page, and decision 2 narrows to new
text only.

## Needs the project owner

- **Which readers matter most?** The ToUnicode convention trades accuracy in PDFium-based viewers
  (Chrome, Edge) against PDFKit (macOS Preview, iOS). Acrobat Reader — the reference in the
  target market — and pdf.js have **not been tested**. Recommendation: test those two, then pick
  the convention that serves Acrobat and Chrome/Edge, since the primary market is Windows.
- **The spec's §7 acceptance test** "copy-paste from the saved file in another viewer yields
  correct logical text" cannot be met for vocalized text in the two extractors measured. It needs
  rewording to name specific viewers and text classes once Acrobat is measured.

## Limits of this spike

Two fonts (both static TrueType); one line per input; no line breaking or script itemization;
two extractors; `qpdf --check` only (no PDF/A or PDF/UA validation); debug-build timings only.
Variable fonts, CFF fonts, Nastaliq-style cursive attachment, and kashida are untested.

## Amendment, 2026-11-22 (M3): content regeneration is lossy; text goes through our own writer

The check required by "Before M3 commits" was run: `spikes/pdfium-regenerate/` (README there has
the method and every number). Five edits per page — mark one text object dirty, delete it,
remove and reinsert it, replace it with a box, append a box — across the 33 corpus pages that
open plus a hand-written stress page. Each result was regenerated with `FPDFPage_GenerateContent`,
saved, reopened, and compared with the page PDFium rendered from its in-memory objects before
saving.

**Regeneration silently damages untouched content on 27 of 34 pages** (`qpdf --check` passes on
all of them):

- **Type 3 fonts:** every text object is written as `q … BT <matrix> Tm` with no font, no glyphs,
  no `ET`, no `Q`. All text disappears and the reopened page has no objects. Chrome writes Type 3
  for the corpus's variable fonts, so 19 pages are hit.
- **Simple fonts sharing a `/BaseFont`** (CoreText's per-encoding subsets of one face) are merged
  into one resource: Arabic is redrawn with the wrong encoding, as tofu (7 pages).
- **Operators lost** on the stress page: inline images, `sh`, text render mode 7 (text clipping),
  `Tc`/`Tw`/`Tz`, and `BX`/`EX` sections.
- **What survives** where fonts are not affected: clip paths, z-order (reinsert and replace),
  marked content (MCIDs, optional content with its `/Properties`, `/ActualText`, artifacts),
  form XObjects, transparency, colours.
- **Appending is safe:** the existing stream is left byte for byte and a new one is added. This
  is all the M0 spike exercised.

M2 has since decided (ADR 0014, ADR 0015) that every edit is an incremental update written by
our own code, that undo is a stack of those updates, and that signed files are saved by
appending them. PDFium's edit route cannot produce an update: its only output is a full rewrite
(`save_to_bytes`).

### Decision (replaces decision 2; decisions 1, 3 and 5 stand)

1. **PDFium never regenerates a page's content.** No delete, replace or modify of existing page
   objects through its edit API, on any page.
2. **Text is written by our own content-stream writer, as incremental updates** (ADR 0014):
   - new text: font objects (Type 0 / CIDFontType2, our subset, our ToUnicode, CIDToGIDMap)
     plus a new content stream appended to the page's `/Contents`, and the page dictionary with
     merged `/Resources`;
   - in-place edits: the page's stream copied byte for byte except the text-showing operators
     of the edited block, with the new text emitted at the same point in the stream, so it keeps
     its z-order, clip and marked-content context.
   Decision 4's "planned second backend" becomes the only one. It builds on M2's
   `safha_core::pdf` reader and writer, not on `lopdf` (finding 5 still stands).
3. **PDFium's role:** render, extract, verify (render and extract every write in tests), and the
   full rewrite of unsigned files on save (ADR 0015), which serializes objects and regenerates
   nothing.
4. **The text pipeline's output stays backend-neutral:** positioned glyph runs from the layout
   engine (ADR 0020). The PDFium CID route remains a fallback that consumes the same runs, should
   the own writer ever be blocked.

### Consequences

- The writer must handle what PDFium's route gave for free: font dictionaries, the CIDToGIDMap,
  and resource-name collisions. The M0 spike's Route B already wrote all of these with output
  identical to PDFium's (finding 3), so the recipe is known.
- **Encrypted files:** new objects in an update to an encrypted file must be encrypted with the
  file's key. M2's writer does not encrypt (it only rewrites existing objects). Until it does,
  text edits on encrypted files are refused with `edit_unsupported`.
- Text gains the undo stack and signature-safe saves that page edits already have.
