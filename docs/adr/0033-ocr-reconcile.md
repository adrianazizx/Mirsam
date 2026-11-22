# 0033 — Re-reading and reconciling OCR: the mixed-text and numbers gates

Status: Accepted for the M4 pipeline (measured on the synthetic set only). Date: 2026-11-22.
Builds on ADR 0030 (engines), ADR 0032 (running them). Spec §8 steps 2–7.

## Context

Through llama.cpp, Nuqta 4B reads `ocr-eval` set v2 at 0.89% CER (ADR 0030, ADR 0032). Two ship
gates still failed:
- **mixed Arabic/English:** 2.07% CER against 2%;
- **numbers, dates and IDs:** 93.6% of fields exact against 99%.

ADR 0030 planned a second reader (Nuqta 0.8B) for unsure lines and for every number, and the
platform's OCR as a third reader for fields. It also required that the pipeline never picks a
reading silently. This ADR records how that was built and what it measures.

## What the errors were

The 4B misses 11 of 171 fields:

- **"64-bit" as "bit-64" (6 fields).** In an Arabic line the number takes the line's direction, so
  "64-bit" is drawn "bit-64". The models write what they see. The image cannot settle it: logical
  "bit-64" is drawn the same way, and so are "COVID-19" and "19-COVID" (as ADR 0008 found from
  glyph order).
- **Five wrong digits on degraded pages:**
  - ١٣٣٤ for ١٢٣٤ (fax, photocopy, curved photo);
  - ٣٠٤٨٧ for ٢٠٤٨٧ (photocopied form);
  - an extra ٩ in an ID.

The mixed category also loses "Lease Agreement", the English heading of the two-column contract,
which the 4B leaves out on both fonts.

## What was tried (spike, recorded readings, `spikes/ocr-engines/reconcile_*.py`)

**Voting doesn't work.** Letting Nuqta 0.8B and Apple Vision outvote the 4B on digit words lifts
fields only to 94.2%. On the fax page both wrote ١٢٢٤, so the vote replaced one wrong number with
another.

**Reading lines at full resolution helps.** Each page was cut into text-line strips. A strip is
read at the page's own resolution, about twice the glyph size of the 2.4 MP page read. Per
misread number, these readers answered:

| Page | Page read (4B) | 4B strip | 0.8B strip | 0.8B page | Apple Vision | Truth |
|---|---|---|---|---|---|---|
| fax, 100 dpi | ١٣٣٤ | ١٣٣٤ | 1334 | ١٢٢٤ | ١٢٢٤ | ١٢٣٤ |
| photocopy | ١٣٣٤ | ١٢٣٤ | 1234 | ١٣٣٤ | ١٢٢٤ | ١٢٣٤ |
| curved photo | ١٣٣٤ | ١٢٣٤ | ١٢٣٤ | ١٢٣٤ | ١٢٣٤ | ١٢٣٤ |
| photocopied form | ٣٠٤٨٧ | ٣٠٤٨٧ | ٢٠٤٨٧ | ٢٠٤٨٧ | — | ٢٠٤٨٧ |
| photocopied form (ID) | ١٠٢٣٤٥٦٧٨٩٩ | ١٠٢٣٤٥٦٧٨٩ | ١٠٣٣٤٥٦٧٨٩ | ١٠٢٣٤٥٦٧٨٩ | — | ١٠٢٣٤٥٦٧٨٩ |

**Other preprocessing doesn't settle the hard two.** The fax strip, enlarged 2x or 3x or
binarised, gave ١٢٢٤, ١٣٣٤ or 1324. The form strip, enlarged or binarised, gave ٣٠٤٨٧, ٢٤٨٧ or
٢١٤٨٧. At 100 dpi the fax's digits are about 10 px tall, and its strokes are broken. These two
are not reliably readable from the image.

Rule variants on the recorded readings (whole set, 2,538 words):

| Rule | Fields | Mixed CER | Words fixed / broken | Wrong words flagged | Right words flagged |
|---|---|---|---|---|---|
| Order fix only | 97.1% | 1.52% | – | 0 | 0 |
| + correct when 2+ re-reads agree and none backs the page (strips, 0.8B page) | 98.2% | 1.52% | 4 / 0 | 7 | 28 |
| **+ strip reads weigh more (chosen)** | **98.8%** | **1.52%** | **5 / 0** | 6 | 28 |
| same, Apple Vision also reads every unsure word | 98.8% | 1.52% | 5 / 0 | 10 | 49 |
| same, Apple Vision reads numbers only | 98.8% | 1.52% | 5 / 0 | 6 | 35 |

## Decision

`safha_core::ocr` gains the quality pipeline for a page, `QualityReader::read_page`:

1. **Assess and lay out** (`layout::analyze`).
   - **Ink:** Otsu's threshold, then specks removed. A pixel stays if its 5x5 window holds at
     least 9 ink pixels.
   - **Skew:** the steepest row profile over ±4°.
   - **Line strips:** from the deskewed page's row profile. The threshold follows the page's speck
     level, rows over 70% ink (background, shadows) are skipped, and thin bands are dropped.
   - **Assessment:** skew and the share of specks, as measures for choosing preprocessing.
   - No detector is needed. Curved pages can put several lines in one strip; the model reads
     multi-line crops.
2. **Read** the page with the primary (Nuqta 4B), from the original image, as measured.
3. **Order** (`order::resolve_unit_order`). In lines holding Arabic, `<unit>-<number>` becomes
   `<number>-<unit>`, for a closed list of units English writes number-first ("64-bit", "3-day",
   "5-star"). "COVID-19", "A-4" or "Windows-11" are untouched. Changed words are marked
   `Reordered` and listed for review. The same rule is applied to every re-read, so the re-reads
   cannot undo it.
4. **Re-read targets** (`reconcile`). A target is any word with a digit, or any word under 0.95
   probability (ADR 0030, finding 3).
   - **Partner (Nuqta 0.8B):** reads the page and every strip.
   - **Primary:** reads the strips that hold a target, placed through the partner's strip text;
     all strips when a target cannot be placed.
   - **Third reader** (the platform's OCR, optional): reads the page, for numbers only.
5. **Reconcile.** Readings of the same word are found by context (three words either side), and
   must be similar to it, so a word is never matched to an unrelated one. They are compared by what
   they say: digits by value, letters without tashkeel.
   - **Confirmed:** every reading agrees.
   - **Corrected:** both strip reads agree on another value; or one strip read and another reader
     do, and no strip read backs the page read. The text is replaced, keeping the page's digit
     script, and the old reading is kept beside it.
   - **Uncertain:** anything else. The page read is kept and the word is flagged.

   Tashkeel the page read kept is never lost to a re-read that differs only in tashkeel.
   Corrections are always another engine's reading of this image, so nothing is invented.
6. **Review** (`PageResult`, `review_list`).
   - Every word carries its probability and status.
   - A page goes on the pages-needing-review list when it has an uncertain word or a cut-off
     answer. Corrected and reordered words are listed too, but don't hold a page back.
   - The list puts pages needing a decision first.

**Third reader.** It is `CommandEngine`, any program that prints an image's text. On macOS that
is `ocr-eval/engines/apple-vision.swift --image` (which needs the Swift toolchain); a signed helper
ships with the model download manager. The spike showed it catches nothing more on this set, so
it is optional and reads numbers only, as the owner asked.

## Results through Rust

Measured with `crates/safha-core/examples/ocr_eval.rs --pipeline`: Nuqta 4B Q8_0 primary and
Nuqta 0.8B Q8_0 partner through `llama-server`, Metal, M3 Max, no third reader. Report:
`ocr-eval/baselines/pipeline-nuqta-4b+0.8b.set-v2.json`.

| | 4B alone (ADR 0032) | Pipeline |
|---|---|---|
| CER, all pages | 0.89% | **0.60%** |
| Mixed Arabic/English (gate: CER ≤ 2%, no order errors) | 2.07%, fail | **1.52%, 0 order errors: pass** |
| Numbers, dates, IDs (gate: ≥ 99% of fields) | 93.6%, fail | **99.4% (170 of 171): pass** |
| Table cells exact | 97.5% | 100% |
| Severe scans (tracked) | 1.59% | 1.46% |
| Regressions in any category | – | none |
| **All five gates** | 3 of 5 | **5 of 5** |

- **Corrections:** 7, all fixes; no right word was changed. Four were digits (١٢٣٤ twice, ٢٠٤٨٧,
  the ID) and three were words on severe scans (تُسجَّل, الأولاد, المراجعة).
- **The one field still wrong** is the 100 dpi fax's ١٢٣٤. It is read ١٣٣٤ and flagged
  `uncertain`, with ١٢٢٤ as the alternative. No field is wrong without a flag.
- **The gate passes by one field.** With 169 of 171 it would fail. The employee number on the
  photocopied form (٢٠٤٨٧) is the borderline case:
  - in the spike's strips, cut by the Python prototype, the 4B read ٣٠٤٨٧ and the field was only
    flagged;
  - in Rust's strips (bilinear deskew, a slightly different line threshold) it read ٢٠٤٨٧, and
    the field was corrected.

  The gate is met on this set, but not with room to spare.
- **Flags:**
  - 36 words uncertain, 5 of them really wrong. The other 31 are false alarms (1.2% of words).
    These come mostly from short words matched to a similar word nearby, and from the 0.8B
    differing on tashkeel-heavy text.
  - 6 words reordered (every "64-bit").
  - 12 of 47 pages need a decision; 17 are on the review list.
- **Wrong words left unflagged:** 28 of 2,538, by a rough word alignment. Most are on the severe
  fax and photocopy pages, where the 4B was confident. They include three en dashes written for
  em dashes, a dropped kasra (قبل for قِبل), and "." for "!". Confidence and a second reader don't
  catch these; a third reader or real-scan calibration might.

**Cost (Metal).**
- 24.9 s per page on average, against 8.3 s for the page read alone.
- 10.2 engine requests per page: 1 for a page with nothing to check, up to 27 for a form with
  many numbers. A page with any target costs the partner's page read, one 0.8B read per strip,
  and the 4B reads of the strips holding targets.
- **CPU-only speed was not measured.** With about ten requests per page, mostly small strips,
  expect several minutes per page on top of the 141 s page read.

## Known limits

- **Synthetic set only.** Real faxes and photocopies may need more re-reading, or a different
  rule.
- **Locating by context is a heuristic.** Short words can be matched to a similar short word
  nearby. That only adds flags, never corrections, since a correction needs two agreeing readers.
- **Left-out text is not recovered.** "Lease Agreement" is still missing. Finding it needs
  detector lines with no words aligned to them (ADR 0031, slice 2).
- **The unit list is a lexicon choice.** It is narrow on purpose, and every use is flagged.
- **Cost.** Strips multiply the requests per page (see results). On a CPU-only machine that is
  minutes per page.

## Consequences / what would close the numbers gap

- **The numbers gate holds by one field on this set.** Ways to widen the margin, best first:
  1. **Re-read the number itself, not the line.** Crop just the number using word boxes (ADR
     0031, slice 2) and read it at several scales. On a 10 px fax digit the strip is too coarse.
  2. **A third reader that is good at digits.** Apple Vision misread the same fax number. A
     classical digit recogniser on the number's crop (PP-OCRv5's, once the detector is ported)
     would be an independent voice.
  3. **Real scans** (owner item 9) to tell whether ١٢٣٤ on a 100 dpi broken-stroke fax is
     representative, or harsher than real faxes.
  - The fields still wrong are flagged. If the owner wants it, a separate tracked measure, "wrong
    fields not flagged", is 0 here; the 99% gate itself stays as it is.
- **Left-out lines** ("Lease Agreement") need slice 2's detector: a line with no words aligned
  to it is read and inserted.
- **Cost on weak hardware.** Re-read only the strips that hold targets with the partner too, once
  word boxes place them without the partner's strip text. Measure on a CPU-only laptop.
- **The pipeline reads the original image for the page read.** Reading the deskewed page instead
  was not measured; it may help skewed scans.
- **The review list and statuses** are ready for the UI (spec §5.8) and for the text layer (slice 4).
