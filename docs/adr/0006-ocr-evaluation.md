# 0006 — OCR evaluation method and gates (engine choice deferred to M4)

Status: Accepted for the evaluation method. The engine decision is M4's spike. Date: 2026-11-22.

## Context

The spec makes OCR a headline feature judged on accuracy, with ship gates on CER/WER per
category, exact-match on numbers and IDs, and zero reading-order errors on mixed text (§8). It
asks for the evaluation set and scorer to exist from M0 so data accumulates before the engine
benchmark.

## Decision

**Scorer** — `crates/safha-ocr-eval`, a pure-Rust library and CLI with no OCR in it:

- **CER / WER**: Levenshtein distance over reference length, at Unicode scalar and
  whitespace-delimited word level, micro-averaged per category.
- **Strict normalization for the gated numbers**: NFC, bidi control characters removed,
  whitespace collapsed — nothing else. Tashkeel, Arabic-Indic vs Western digits, punctuation, and
  presentation forms all count, because the spec requires OCR to preserve what is on the page.
- **`CER-base`** (informational): also ignores tashkeel/tatweel and unifies alef/yeh variants.
  The gap between CER and CER-base shows how much error is diacritics only.
- **Field accuracy**: each listed number/date/ID must appear exactly and as a whole value
  ("4,500" inside "14,500" does not count).
- **Reading-order errors**, counted separately from recognition errors: runs of ≥ 2 reference
  words missing from their place but present elsewhere (a moved line, column, or table cell, or a
  word-reversed line), plus words whose letters came out reversed (visual-order Arabic).
- **Gates** from `ocr-eval/targets.json` (the spec's initial targets); a gated category with no
  samples fails rather than passing vacuously. **Regression check** against a stored report;
  reports from different set versions are refused.

**Evaluation set** — `ocr-eval/`: a versioned manifest of page images with ground truth.

- *Synthetic* samples are generated (`generate.mjs`) from the same content files as the PDF
  fixtures: known text, set in four Arabic faces (two Naskh, two Kufi-style sans), rasterized at
  100–300 dpi, and degraded (blur, skew + speckle, bilevel fax, photocopy shadow). 30 samples in
  set v1 across nine categories.
- *Real scans* go in `ocr-eval/scans/` with ground truth that names its human verifier. None yet.

## Evidence: first baseline

Apple Vision (the OCR built into macOS; `ocr-eval/engines/apple-vision.swift`), set v1:

| Category | n | CER | WER | Fields | Order errors | Gate |
|---|---|---|---|---|---|---|
| clean-arabic-300dpi | 4 | 0.25% | 1.58% | – | 0 | pass |
| clean-english-300dpi | 1 | 0.00% | 0.00% | 100% | 0 | pass |
| degraded-arabic | 11 | 1.25% | 5.36% | – | 0 | pass |
| diacritized | 3 | 1.40% | 9.20% | – | 0 | (not gated) |
| other-typefaces | 4 | 0.59% | 3.68% | – | 0 | (not gated) |
| **mixed-arabic-english** | 4 | **9.46%** | 12.82% | 85.7% | **7** | **fail** |
| **numbers-dates-ids** | 9 | 17.15% | 23.50% | **85.1%** | 21 | **fail** |
| tables-forms | 2 | 79.87% | 96.43% | 100% | 6 | (not gated) |

Stored as `ocr-eval/baselines/apple-vision.set-v1.json`. 327 pages/min (informational).

What this tells us:

1. The scorer discriminates the way the spec cares about: an engine that reads clean Arabic
   almost perfectly still fails badly on mixed-direction lines, numbers, and table reading order —
   exactly the failures the spec calls out.
2. **Set v1 is too easy in its degraded category** (1.25% CER against a 5% gate). Synthetic
   degradations of crisp renders are not real fax and photocopy damage. The category needs real
   scans and harsher synthetic conditions before its gate means anything. Per the spec, targets
   are then only ever tightened.
3. The 80% CER on tables is mostly ordering (cells read left-to-right in an RTL table), which CER
   punishes heavily. Table samples need cell-level scoring in M4 rather than page-level CER.

## Consequences / what M4 inherits

- The Vision baseline is a reference point, not a candidate default (macOS only, closed). It could
  serve as an optional third voter on macOS. The spec's required comparisons — plain Tesseract and
  the leading commercial editors — are still to be run; Tesseract is not installed on this machine.
- Candidates to benchmark in M4 (QARI-OCR, Baseer, PP-OCRv5 Arabic, Tesseract `ara`) only need to
  emit one text file per sample to be scored.
- CI runs the scorer's own tests now. The benchmark-in-CI gate is wired when a pipeline exists.
