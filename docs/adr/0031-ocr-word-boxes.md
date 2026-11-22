# 0031 — Word boxes for VLM OCR text

Status: Proposed (M4 spike, measured on a synthetic set only). Date: 2026-11-22.
Builds on ADR 0030 (Nuqta as the recogniser) and ADR 0008 (bidi order).

## Context

Spec §8 wants an invisible text layer "precisely aligned to the image (word-level boxes at
minimum)", stored in logical order. The recogniser chosen in ADR 0030, Nuqta-Arabic-OCR, writes
plain text with no positions. The spec expects this: pair such models "with a detector/aligner to
get word positions". This spike asks whether PP-OCRv5's text detector, plus an alignment of the
VLM's text to what it finds, puts each word where it is on the page, and at what cost.

Nothing here is integrated into the app. The harness is `spikes/ocr-engines/` (`wb_*.py`,
`word_boxes_truth.mjs`).

## Method

**Ground truth.** The eval set had no boxes. Chrome laid the pages out, so
`word_boxes_truth.mjs` re-renders each page's HTML with the same content, face and page rules,
and reads every word's rectangle from the DOM (`Range.getClientRects`, scaled to the image's
dpi). An overlay on the 300 dpi mixed page matches the image exactly. Each box is then tightened to
the ink inside it, since a DOM box spans the font's whole line height. Only pages whose
degradation keeps geometry are used: clean, low resolution, defocus blur, plain fax. That gives
26 pages and 1,377 words, in eight content types (table below). Skewed, warped and photocopied
pages have no boxes yet.

**Detector.** `PP-OCRv5_server_det` (Apache-2.0, 84 MiB) through PaddleOCR 3.7 on PaddlePaddle's
CPU backend, input capped at 2048 px on the long side as in ADR 0030. It returns text-line
polygons, not words. Its companion Arabic recogniser (`arabic_PP-OCRv5_mobile_rec`, 7.8 MiB)
reads each line, and that text serves only as a *guide*: it is never shown or kept. PaddleOCR's
own word boxes (`return_word_box`) are no use for Arabic: on a mixed page it returned one box for
a whole Arabic line, with the words run together ("اشتريتأجهزة").

**Alignment** (`wb_align.py`). The VLM's words go into the detected lines, then each line's ink
extent is divided among its words:

1. **Line order.** Lines are ordered by rows (right to left within a row) or, when the page has a
   wide empty vertical band, by columns in either order. The order kept is the one whose guide
   text best matches the VLM's text.
2. **Words to lines.** The candidates measured:
   - **M0, width only:** cumulative estimated word widths against line widths. No recogniser.
   - **M1, guide anchors:** a character alignment of the VLM's text to the guide text votes each
     word into a line. Words with no vote fill the gap between anchors by each line's capacity.
   - **M3, width fit:** a dynamic program cuts the word sequence into one run per line, fitting
     estimated run widths to line widths (a line never spans two of the VLM's paragraphs). No
     recogniser.
   - **M4, width fit with anchors:** M3, with a penalty for moving a word away from the line the
     guide puts it on.
   - **M5, auto:** M1's lines when the guide matches at least half of the VLM's characters,
     otherwise M4's.
3. **Visual order inside a line.** The line's words are put in left-to-right order by UAX #9 at
   word level: each word gets one type from its letters; then W7, N1/N2, I1/I2 and L2 apply, with
   the space between words as a neutral. That space matters. A first version that ran consecutive
   numbers together drew "+966 11 234 5678" in the wrong order inside an Arabic line. The line's
   base direction comes from which margin of its column it touches, or else from its first strong
   letter.
4. **Dividing the line.** Boundaries are first placed in proportion to estimated word widths (by
   character class). Except in M0 and M1, a second dynamic program then snaps each boundary to a
   real gap in the line's ink profile, trading closeness to the estimate against gap width. Each
   box is finally tightened to its ink.

**Scores.** The VLM's words are paired, in order, with truth words they equal. For each pair the
spike measures whether the word went into the line holding the true word, and how much the box
overlaps the true box: horizontally (IoU-x, what selection and search highlights need) and in full
(IoU).

## Results

All pages, VLM text from Nuqta 4B Q8_0 through llama.cpp (ADR 0030 follow-up). Reports, per page
and per method: `ocr-eval/baselines/word-boxes/ppocrv5-det+nuqta-*.set-v2.json`.

| Method | Right line | IoU-x mean | IoU-x ≥ 0.5 | IoU-x ≥ 0.7 | IoU ≥ 0.5 |
|---|---|---|---|---|---|
| M0 width only | 74.8% | 0.17 | 17.7% | 11.5% | 15.3% |
| M1 guide anchors, no snapping | 97.0% | 0.70 | 80.1% | 61.4% | 77.3% |
| M2 guide anchors + snapping | 97.0% | 0.90 | 90.3% | 88.2% | 90.0% |
| M3 width fit + snapping | 70.4% | 0.22 | 22.7% | 19.3% | 21.1% |
| M4 width fit + anchors + snapping | 98.3% | 0.91 | 92.4% | 90.2% | 91.6% |
| **M5 auto** | **99.0%** | **0.92** | **93.0%** | **90.8%** | **92.5%** |

Nuqta 0.8B Q8_0's text gives nearly the same M5 result (98.5% / 0.91 / 92.3%), and so does the 4B
at Q4_K_M. Alignment quality does not depend on which Nuqta reads the page.

By content type: right line / mean IoU-x, 4B text.

| Pages | Words | M0 | M2 | M3 | M4 | M5 |
|---|---|---|---|---|---|---|
| Arabic prose, four faces, low-res/blur/fax (7) | 427 | 89.7% / 0.24 | 100% / 1.00 | 87.1% / 0.23 | 100% / 1.00 | 100% / 1.00 |
| Lam-alef ligatures (7) | 238 | 67.2% / 0.05 | 99.2% / 0.94 | 70.6% / 0.07 | 94.5% / 0.76 | 99.2% / 0.94 |
| Vocalized (3) | 87 | 79.3% / 0.15 | 58.6% / 0.23 | 79.3% / 0.20 | 90.8% / 0.63 | 90.8% / 0.63 |
| English (1) | 39 | 89.7% / 0.45 | 100% / 1.00 | 94.9% / 0.64 | 100% / 1.00 | 100% / 1.00 |
| Mixed Arabic/English lines (3) | 244 | 86.9% / 0.20 | 100% / 0.87 | 89.3% / 0.28 | 100% / 0.96 | 100% / 0.87 |
| Two-column bilingual (2) | 220 | 60.0% / 0.11 | 99.1% / 0.90 | 8.2% / 0.05 | 98.7% / 0.93 | 99.1% / 0.90 |
| Bilingual table (2) | 56 | 17.9% / 0.13 | 96.4% / 0.84 | 42.9% / 0.28 | 100% / 0.95 | 96.4% / 0.84 |
| Arabic form (1) | 66 | 43.9% / 0.04 | 100% / 0.99 | 97.0% / 0.81 | 100% / 0.99 | 100% / 0.99 |

**Detector:** every true word's centre fell inside a detected line on all 26 pages. Cost per
page, on the CPU through PaddlePaddle (M3 Max, 10 threads, nothing else running): **10.1 s** at
2048 px, **4.1 s** at 1280 px, with the same word recall on 9 pages tried. The guide recogniser
adds about 2 s. The alignment itself (M5, ink profile included) takes 0.07 s per page in Python.

## Findings

1. **The detector is not the problem.** PP-OCRv5's server detector found every line, on clean,
   low-resolution, blurred and fax pages, in Arabic, English, mixed lines, two columns, a table
   and a form.
2. **Text alone cannot place words on lines.** Without a recogniser (M0, M3) only 70–75% of words
   land on the right line. Estimated word widths are not precise enough to tell where a line
   wraps, and one word off at a wrap shifts the whole line. Some reading of each line is needed,
   even a poor one: the PP-OCRv5 Arabic recogniser's 32% CER (ADR 0030) is enough for 97–99%.
3. **Snapping to ink gaps is what makes boxes precise.** With the right words on a line,
   proportional splitting gets 0.70 IoU-x; snapping boundaries to gaps gets 0.90–1.00. On Arabic
   prose in four faces, low resolution and fax, M5's boxes are exact (IoU-x 0.99–1.00).
4. **Where it still fails:**
   - **Vocalized Amiri** (IoU-x 0.21–0.70). The guide recogniser cannot read it (its text matches
     19–27% of the VLM's characters), so lines fall back to the width fit. Tashkeel also opens
     gaps inside words.
   - **Amiri lam-alef** (one page, 0.73). Ligatures and wide letter spacing in the classical face
     make gaps inside words as wide as the gaps between them.
   - **A word the VLM wrote differently from the page.** "bit-64" for "64-bit" (ADR 0030) is
     unpaired, and the capacity rule pushes it onto the next line, shifting that line's words (the
     mixed pages under M5). M4's width fit handles it better (0.96).
   - **Table cells** with a dash or "A4": short tokens in wide cells, where proportional widths
     mislead.

   On real scans with skew, warping and stamps, the detector's polygons will be tilted or curved.
   Splitting along a tilted line was not tested.
5. **Cost.** The detector on PaddlePaddle's CPU backend costs 4–10 s per page. That is less than
   Nuqta itself (8.4 s on Metal, 148 s on the CPU) but more than it should be. ONNX Runtime, the
   spec's plan, is the next thing to try.

## Decision

- **Detector: PP-OCRv5 server detector** for line polygons. Its Arabic recogniser reads each line
  only as an alignment guide; its text is never shown.
- **Alignment: M5** (guide anchors where the guide reads well, a width fit with anchors where it
  does not), then **snapping to ink gaps** and **UAX #9 word order** per line. Words go into the
  text layer in the VLM's logical order, each with its own box.
- **Boxes carry the alignment's confidence.** A word whose line came from a fallback (no anchor)
  or whose boundaries were not snapped is marked low-confidence for the review list, the same way
  as uncertain text.

## Known limits

- **Synthetic, geometry-preserving pages only** (26 of 47). There is no truth yet for skewed,
  warped or photocopied pages, or real scans.
- **Ground truth is Chrome's layout,** tightened to ink. A real scan's truth would have to be
  drawn by hand.
- **One detector configuration** (server, 2048 px, PaddlePaddle CPU). The mobile detector, ONNX
  Runtime, and Apple Vision's text rectangles as a free guide on macOS were not tried.

## Consequences / what M4 does next

1. Port the detector to ONNX Runtime (spec §8) and re-time it, including at 1280 px.
2. Add box truth for skewed and warped pages: transform the DOM boxes by the same filter
   parameters, or hand-mark a few. Then test splitting along tilted and curved lines.
3. Try a better guide where the PP-OCRv5 recogniser fails (vocalized text): the platform OCR, or
   Nuqta 0.8B on the line crops of only those lines.
4. When the pipeline lands, the assembly step reuses `safha_core`'s bidi code rather than this
   spike's simplified UAX #9, and stores logical order with per-word boxes (spec §6.3).
