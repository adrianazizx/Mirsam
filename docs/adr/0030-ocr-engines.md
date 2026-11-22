# 0030 — OCR engines for the quality pipeline

Status: Proposed (M4 spike, measured on a synthetic set only). Date: 2026-11-22.
Extends ADR 0006: evaluation set v2, table-cell scoring, alternative reading orders.

## Context

Spec §8 makes OCR accuracy the first priority: accuracy → reading order and layout → robustness
on bad scans → speed → model size. The default pipeline must beat plain Tesseract and the OCR
built into commercial editors in every Arabic category. ADR 0006 built the evaluation set and
scorer, measured Apple Vision, and left the engine choice to this spike. PROGRESS named the
candidates (QARI-OCR, Baseer, PP-OCRv5 Arabic, Tesseract). The spec also asks for "other current
document-OCR models found at build time".

Nothing here is integrated into the app. The harness is `spikes/ocr-engines/`.

## Candidates (checked 2026-11-22)

| Engine | Version | Licence (code / weights) | Download (GiB) | Maintenance | Run |
|---|---|---|---|---|---|
| Tesseract | 5.5.4 (2026-10-28), tessdata_best `ara` + `eng` | Apache-2.0 / Apache-2.0 | 0.03 | Active (commits in Nov 2026) | Yes, `ara+eng` and `ara` |
| PP-OCRv5 Arabic | PaddleOCR 3.7.0, PaddlePaddle 3.3.1; `PP-OCRv5_server_det` + `arabic_PP-OCRv5_mobile_rec` (2025-10) | Apache-2.0 / Apache-2.0 | 0.09 | Active (Baidu) | Yes, CPU |
| QARI-OCR v0.3 | `NAMAA-Space/Qari-OCR-v0.3-VL-2B-Instruct` (Qwen2-VL-2B, 2025-06) | Apache-2.0 | 4.1 | Occasional | Yes |
| QARI-OCR 0.4.0 | LoRA on `Qwen/Qwen3-VL-4B-Instruct` (2026-02); domain: Islamic books | Apache-2.0 (adapter and base) | 0.3 + 8.3 | Occasional | Yes |
| PaddleOCR-VL-1.6 | `PaddlePaddle/PaddleOCR-VL-1.6`, 0.9B (2026-05, updated 2026-11-06); 109 languages | Apache-2.0 | 1.8 | Active (Baidu) | Yes, as a whole-page "OCR:" task |
| Nuqta-Arabic-OCR | `SalahALHaismawi/Nuqta-Arabic-OCR-4B` and `-0.8B` (Qwen3.5 base; released 2026-10, updated 2026-11-19) | Apache-2.0 | 9.1 / 1.7 | Very new; three contributors, 22 GitHub stars | Yes |
| Apple Vision | macOS 26.5.2 | Closed, macOS only | built in | Apple | Yes, reference (ADR 0006) |
| Baseer (Misraj) | — | — | — | — | **No**: the document model was never released as weights (hosted service only). The only open weights, `Misraj/Baseer__Nakba`, are a fine-tune for handwritten manuscripts under CC-BY-NC-SA-4.0 (non-commercial). Skipped with the owner's agreement. |

Not run, and why:

- **PP-OCRv6** (2026-06): no Arabic. Its recogniser's dictionary has no Arabic letters (checked),
  so the spec's note still holds and PP-OCRv5 Arabic is the only Paddle classical option.
- **chandra-ocr-2, surya-ocr-2.5**: OpenRAIL weights, a custom licence that would need the owner.
- **Unlimited-OCR-2, dots.ocr, DeepSeek-OCR-2, MiniCPM-OCR**: no Arabic claim.
- **Arabic handwriting models** (Raqim, Baseer-Nakba): non-commercial, and out of scope.
- **Commercial editors** (Acrobat, ABBYY FineReader): not installed. The owner would have to run
  them (see Consequences).

The owner approved the installs and downloads above (2026-11-22). All of it lives under
`~/Library/Caches/safha-dev/ocr-bench/`.

## Method

**Evaluation set v2** (47 samples; v1's 30 are unchanged and re-score identically). Without these
changes the comparison would not have meant much:

- **Severe scans** (`degraded-arabic-severe`, 12 samples, tracked): a 100 dpi fax with thinned,
  broken strokes and specks; a second-generation photocopy (ink bleed, mottled grey paper, 2° skew);
  faded toner; a phone photo of a curved page (perspective, wavy lines, uneven light). All are made
  with Chrome SVG filters, so they are deterministic. v1's `degraded-arabic` was too easy: Apple
  Vision scored 1.25% against a 5% gate (ADR 0006). The new conditions spread engines out (Apple
  Vision 6.7%, from 0.5% to 20% per page). The severe conditions are also applied to mixed lines
  and a table.
- **An Arabic-only form** (`ocr-eval/content/form-leave-request.json`): a right-to-left label/value
  table with Arabic-Indic IDs and dates. It stays out of the PDF fixture corpus.
- **Table cells.** Table samples list their cells, and the scorer reports cells read exactly and
  cells read out of order. ADR 0006 asked for this: page CER cannot tell a misread table from a
  misordered one. On the clean form, Apple Vision read 100% of the cells but went column by column,
  which scored as 28% page CER.
- **Either column first** on the side-by-side bilingual contract. The truth read English first, so
  an engine that read the Arabic column first lost 84% CER on a page it read perfectly. That
  penalised exactly the behaviour an Arabic reader wants. The swapped order is now an accepted
  alternative truth, and interleaving the columns still fails both truths.

**Harness.** One runner per engine writes one text file per page; `safha-ocr-eval` scores them.
Every model runs the way its card documents it: its own prompt, its own input size, greedy
decoding. The other settings:

- Vision-language models run through 🤗 Transformers 5.24 on Metal (MPS) in bf16.
- Their Markdown or HTML answers have the markup removed (`to_text.py`), the same way for every
  model; no content character is changed.
- Tesseract runs with automatic page segmentation and the page's true dpi, in two configurations:
  `ara+eng` and `ara`.
- PP-OCRv5 has no layout analysis, so its lines are put in order by a simple right-to-left-aware
  rule with no column detection.

**Machine.** Apple M3 Max, 36 GB, macOS 26.5.2. Other sessions were building on the same Mac,
so the timings are noisy. They are recorded, not gated.

## Results (set v2, 47 pages)

**Strict CER per category.** Lower is better, and bold marks the best engine in each category.
Every report is stored as `ocr-eval/baselines/<engine>.set-v2.json`.

| Category | n | Apple Vision | Tesseract ara+eng | Tesseract ara | PP-OCRv5 Arabic | QARI 0.3 | QARI 0.4 | PaddleOCR-VL 1.6 | Nuqta 0.8B | Nuqta 4B |
|---|---|---|---|---|---|---|---|---|---|---|
| Clean Arabic, 300 dpi (gated) | 4 | 0.25% | 21.78% | 18.34% | 20.18% | 0.17% | 1.42% | 7.29% | **0.00%** | **0.00%** |
| Other typefaces | 4 | 0.59% | 10.72% | 9.05% | 8.46% | 0.34% | 1.59% | 6.37% | **0.00%** | **0.00%** |
| Diacritized | 3 | 1.40% | 68.77% | 59.88% | 76.02% | 1.52% | 4.80% | 36.84% | 0.12% | **0.00%** |
| Degraded Arabic (gated) | 11 | 1.25% | 31.90% | 30.09% | 16.24% | 1.10% | 3.21% | 10.46% | 0.03% | **0.00%** |
| Severe scans (new) | 12 | 6.68% | 44.04% | 36.95% | 38.05% | 3.38% | 6.55% | 21.29% | 1.84% | **1.59%** |
| Mixed Arabic/English (gated) | 4 | 9.46% | 11.80% | 44.40% | 51.33% | 12.44% | **1.61%** | 63.45% | 5.56% | 2.07% |
| Degraded mixed | 4 | 20.71% | 43.31% | 57.37% | 49.48% | 28.45% | 11.40% | 51.46% | 5.86% | **1.52%** |
| Numbers, dates, IDs | 14 | 20.95% | 30.76% | 49.55% | 41.49% | 23.78% | 10.13% | 52.98% | 4.36% | **1.38%** |
| Tables and forms | 5 | 44.45% | 49.89% | 40.93% | 21.35% | 40.40% | 24.41% | 47.90% | 0.92% | **0.31%** |
| Clean English (gated) | 1 | **0.00%** | **0.00%** | 79.60% | **0.00%** | **0.00%** | **0.00%** | **0.00%** | 0.40% | **0.00%** |
| All pages | 47 | 9.52% | 33.58% | 37.37% | 32.43% | 9.66% | 6.30% | 28.67% | 2.02% | **0.89%** |

**Fields, reading order and tables:**

| | Apple Vision | Tesseract ara+eng | Tesseract ara | PP-OCRv5 Arabic | QARI 0.3 | QARI 0.4 | PaddleOCR-VL 1.6 | Nuqta 0.8B | Nuqta 4B |
|---|---|---|---|---|---|---|---|---|---|
| WER, all pages | 19.82% | 47.86% | 56.74% | 50.88% | 16.16% | 14.75% | 43.92% | 4.29% | 1.93% |
| Fields exact (171) | 78.9% | 56.7% | 31.6% | 36.3% | 66.7% | 83.0% | 66.1% | 95.3% | 93.6% |
| … without "64-bit" (165) | 81.8% | 58.8% | 32.7% | 37.6% | 68.5% | 86.1% | 68.5% | 98.8% | 97.0% |
| Reading-order errors, all pages | 39 | 27 | 5 | 16 | 20 | 17 | 43 | 6 | 0 |
| Table cells exact / misordered (80) | 77.5% / 36 | 48.8% / 14 | 33.8% / 2 | 70.0% / 2 | 65.0% / 29 | 85.0% / 34 | 63.7% / 28 | 98.8% / 0 | 97.5% / 0 |

"64-bit" sits inside an Arabic sentence ("(64-bit)!"). A right-to-left line draws it as
"bit-64", and every engine wrote it that way. ADR 0008 found the same case undecidable from glyph
order alone. Putting a left-to-right run back into logical order belongs to the pipeline's
assembly step, so the second row leaves this field out.

**Speed, memory and size.** These are recorded, not gated, and noisy (see Machine above). Peak
Metal is the GPU memory allocated through MPS, which RSS does not show.

| Engine | Seconds per page | Peak RSS | Peak Metal | Weights |
|---|---|---|---|---|
| Apple Vision | 0.27 | 578 MiB | – | – |
| Tesseract ara+eng | 0.7 | 257 MiB | – | 27 MiB |
| Tesseract ara | 0.44 | 179 MiB | – | 12 MiB |
| PP-OCRv5 Arabic | 10.98 | 23.4 GiB | – | 92 MiB |
| QARI 0.3 | 10.3 | 6.0 GiB | 6.1 GiB | 4.1 GiB |
| QARI 0.4 | 11.21 | 8.6 GiB | 9.6 GiB | 8.6 GiB |
| PaddleOCR-VL 1.6 | 3.02 | 902 MiB | 3.0 GiB | 1.8 GiB |
| Nuqta 0.8B | 2.88 | 1000 MiB | 3.3 GiB | 1.7 GiB |
| Nuqta 4B | 10.15 | 786 MiB | 10.8 GiB | 9.1 GiB |

### The ship gates for the best engine (Nuqta 4B)

| Gate (`ocr-eval/targets.json`) | Target | Nuqta 4B | |
|---|---|---|---|
| Clean Arabic, 300 dpi | CER ≤ 1%, WER ≤ 3% | 0.00%, 0.00% | pass |
| Clean English | CER ≤ 0.5% | 0.00% | pass |
| Degraded Arabic | CER ≤ 5% | 0.00% | pass |
| Mixed Arabic/English | CER ≤ 2%, no reading-order errors | 2.07%, 0 errors | **fail** (by 0.07 points) |
| Numbers, dates, IDs | ≥ 99% exact | 93.6% (97.0% without "64-bit") | **fail** |
| Beat plain Tesseract in every Arabic category | | yes, by 9–60 points of CER against Tesseract's better configuration | pass |
| Beat the OCR built into commercial editors | | Apple Vision (the OCR in macOS Preview) is beaten in every category; Acrobat and ABBYY were not measured | open |

The mixed-text miss comes from two things: "bit-64", and the English heading "Lease Agreement",
which it left out of the contract's left column on both fonts. The field misses are "bit-64" and
five wrong digits on degraded pages (below).

## Findings

1. **Nuqta-Arabic-OCR 4B is the most accurate engine in every category but one.** On mixed
   Arabic/English QARI 0.4 is ahead (1.61% against 2.07%). Its overall CER is 0.89%, against Apple Vision's 9.52% and plain Tesseract's 33.6%. It is the only engine with no
   reading-order errors anywhere. It reads right-to-left tables row by row, in the right cell
   order (97.5% of cells exact, none misordered). It keeps tashkeel where printed (0.00% CER on the
   diacritized pages) and adds none to plain text. On the severe scans it scores 1.59%, against
   Apple Vision's 6.68%.
2. **Nuqta 0.8B comes close** (2.02% overall) at a fifth of the size and 3.5 times the speed. It
   misreads fewer digits than the 4B (98.8% of fields without "64-bit", against 97.0%).
3. **The remaining errors are the dangerous kind:**
   - **Confident wrong digits on degraded pages.** Nuqta 4B wrote ١٣٣٤ for ١٢٣٤ on three
     pages, ٣٠٤٨٧ for the employee number ٢٠٤٨٧, and ١٠٢٣٤٥٦٧٨٩٩ (one ٩ too many) for an ID.
   - **Word substitutions on the severe scans:** الطلب for الملعب, المراسمة for المراجعة.

   The spec's rule against silent invention needs a check that catches these. The model's own
   confidence was measured first: the lowest token probability in each word, from
   `run_vlm.py --confidence`, whose text is identical to the plain run on all 47 pages.
   - **It flags the invented words:** الطلب at 0.42, المراسمة at 0.61, the ID with an extra digit
     at 0.68.
   - **It does not flag the wrong digits.** It was certain of them: ١٣٣٤ three times at 0.9995 or
     higher, and ٣٠٤٨٧ at 0.9999. Correct digits on clean pages sit at the same 0.9998–1.0.

   So confidence cannot guard numbers; a second reader has to (finding 4). Flagging rules
   combined:

   | Flag a word when | Of 4B's 44 wrong words | Of its right words (review work) |
   |---|---|---|
   | its probability < 0.95 | 48% | 4.0% |
   | its probability < 0.99 | 70% | 9.7% |
   | < 0.95, or Nuqta 0.8B disagrees | 68% | 5.8% |
   | < 0.99, or Nuqta 0.8B disagrees | 86% | 11.3% |
   | < 0.99, or Apple Vision disagrees | 93% | 25.0% |

4. **A second engine catches what the first gets wrong** (`agreement.py`). With Nuqta 4B as the
   primary engine, measured over every page:

   | Second engine | Of 4B's 44 wrong words, it disagrees on | Of 4B's right words, it disagrees on | Better page of the two (oracle CER) |
   |---|---|---|---|
   | Nuqta 0.8B | 48% | **2.6%** | 0.65% |
   | QARI 0.3 | 80% | 17.2% | 0.75% |
   | Apple Vision | 80% | 19.3% | 0.70% |
   | QARI 0.4 | 64% | 12.5% | 0.72% |
   | PaddleOCR-VL 1.6 | 91% | 36.2% | 0.89% |
   | Tesseract | 95% | 42.0% | 0.89% |

   Nuqta 0.8B rarely raises a false alarm, and it read three of 4B's five wrong digit fields
   correctly. It shares one mistake with 4B: ١٣٣٤ on the second-generation photocopy. Engines from
   another family (Apple Vision, QARI 0.3) disagreed on that one, so the five digit errors would
   all have been flagged with one more voter. Keeping the better whole page barely helps (0.89% →
   0.65% at best). A second engine is useful to flag words for re-reading or review, not to replace
   the first engine's reading.
5. **QARI-OCR invents text.**
   - **0.4** writes تقريب for تقرير, and "لمعالجة العقبات" where the page has "لمعالجتها". That
     is a language-model prior from its Islamic-books training data, the kind of silent invention
     the spec forbids. It scores 1.42% CER on clean Arabic.
   - **0.3** is close to Apple Vision (9.66% overall), but reads tables column by column and
     sometimes writes Western digits for Arabic-Indic ones (1224 for ١٢٣٤).
6. **PaddleOCR-VL-1.6, run as a whole-page "OCR:" task, is weak on Arabic.** It scores 7.3% CER on
   clean single-column pages, misreading letters (يبدف for يهدف, اللبنة for اللجنة). On mixed pages
   it invents words (الإجتماعة, الانفائية) and repeats lines. Its page-level pipeline (a layout
   model, then recognition per region) was not run, because it needs a layout model outside the
   approved downloads. The letter errors on clean single-column text suggest the gap is in
   recognition, not layout. Its config ships `use_cache: false`, which cuts answers short in
   Transformers; the runner turns the cache on.
7. **PP-OCRv5 Arabic:** 32.4% CER overall. It reads unvocalized prose in modern faces well (1.3–2.9% on
   the Noto Naskh, Noto Sans and IBM Plex pages), and it keeps table rows together: 70% of cells
   exact and only 2 misordered. Its failures:
   - **Faces and marks.** It falls apart on the classical Amiri face (32%) and on vocalized text
     (66–80%).
   - **Mixed lines.** It drops Latin words and Western digits inside Arabic lines, so it gets 36%
     of fields exact. "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال (شاملة الضريبة)." comes out
     as "اشتريتأجهزة بسعرريالشاملة الضريبة". The Arabic
     recogniser reads a whole line right to left, and its character set has Latin letters and
     digits, but they do not survive.
   - **Small slips.** It loses sentence-final full stops and sometimes reads a stray mark as text
     ("dc").

   Its speed and memory are PaddlePaddle's CPU backend on macOS:
   - **At PaddleOCR's default detector resolution:** 135 s per 300 dpi page and a 46 GiB memory
     footprint.
   - **With the detector's input capped at 2048 px** (`text_det_limit_type=max`; recognition still
     reads full-resolution crops): 11 s per page and a 23.4 GiB peak RSS.

   The spec planned ONNX Runtime for this tier. Converting PP-OCRv5 to ONNX might fix the cost, but
   not the faces, the marks or the mixed lines.
8. **Tesseract is a low floor:** 9–69% CER across the Arabic categories. It leaves out whole lines on short pages,
   invents Latin fragments ("call)", "GA") when `eng` is loaded, and writes Western commas. Its
   Arabic-only configuration wrecks English (79.6% CER on the English page). As a quick draft layer
   for search it would put a third of the Arabic words wrong.

## Follow-up: Nuqta through a shippable runtime (llama.cpp, GGUF)

Measured later the same day (branch `pomidor/m4-runtime-8c1840`). The app will not ship Python, so
both Nuqta models were re-run through llama.cpp, the runtime their authors publish builds for.
Harness: `spikes/ocr-engines/run_llamacpp.py`.

**Runtime and files (checked 2026-11-22):**

| | Version | Licence | Size |
|---|---|---|---|
| llama.cpp (`llama-server`, Metal and CPU backends) | 1.0.0, build 12359 (commit `b1b770d`), Homebrew bottle | MIT (ggml authors; its bundled cpp-httplib and nlohmann/json are MIT too) | 21 MiB installed |
| `SalahALHaismawi/Nuqta-Arabic-OCR-4B-GGUF` (revision `f4fdd78`, 2026-10-27) | `Q8_0`, `Q4_K_M`, vision encoder `mmproj-f16` | Apache-2.0 (LICENSE file in the repo) | 4.5 / 2.7 GiB + 0.63 GiB |
| `SalahALHaismawi/Nuqta-Arabic-OCR-0.8B-GGUF` (revision `4f72054`, 2026-10-27) | `Q8_0`, `Q4_K_M`, `mmproj-f16` | Apache-2.0 | 0.81 / 0.52 GiB + 0.19 GiB |

These are the model authors' own conversions of the same weights, so no custom model licence is
involved. MLX was not measured: llama.cpp also runs on Windows and Linux, and at Q8_0 it already
reproduces the reference exactly (below), so MLX has nothing left to win on accuracy.

**Settings:** the model cards' own. Same prompt and the same 2.4 MP input as the Transformers
runs; `-c 8192 --image-max-tokens 4096`, temperature 0, one request per page to a server loaded
once. The CPU runs use `-ngl 0 --no-mmproj-offload` (no GPU at all, vision encoder included) and
llama.cpp's default 10 threads.

**Accuracy (set v2, 47 pages, Metal).** Reports: `ocr-eval/baselines/nuqta-*-llamacpp.set-v2.json`.

| Category | n | 0.8B bf16 (Transformers) | 0.8B Q8_0 | 0.8B Q4_K_M | 4B bf16 (Transformers) | 4B Q8_0 | 4B Q4_K_M |
|---|---|---|---|---|---|---|---|
| Clean Arabic, 300 dpi (gated) | 4 | 0.00% | 0.00% | 0.00% | 0.00% | 0.00% | 0.08% |
| Other typefaces | 4 | 0.00% | 0.00% | 0.00% | 0.00% | 0.00% | 0.00% |
| Diacritized | 3 | 0.12% | 0.12% | 0.12% | 0.00% | 0.00% | 0.00% |
| Degraded Arabic (gated) | 11 | 0.03% | 0.03% | 0.03% | 0.00% | 0.00% | 0.00% |
| Severe scans | 12 | 1.84% | 1.84% | 2.38% | 1.59% | 1.59% | 1.56% |
| Mixed Arabic/English (gated) | 4 | 5.56% | 5.56% | 5.69% | 2.07% | 2.07% | 2.07% |
| Degraded mixed | 4 | 5.86% | 5.75% | 10.09% | 1.52% | 1.52% | 3.19% |
| Numbers, dates, IDs | 14 | 4.36% | 4.32% | 5.83% | 1.38% | 1.38% | 1.95% |
| Tables and forms | 5 | 0.92% | 0.92% | 0.84% | 0.31% | 0.31% | 0.31% |
| Clean English (gated) | 1 | 0.40% | 0.40% | 0.40% | 0.00% | 0.00% | 0.00% |
| **All pages** | 47 | 2.02% | **2.01%** | 2.68% | 0.89% | **0.89%** | 1.09% |
| Fields exact (171) | | 95.3% | 95.3% | 94.7% | 93.6% | 93.6% | 93.6% |
| Reading-order errors | | 6 | 6 | 8 | 0 | 0 | 0 |
| Pages with the same text as bf16 | | – | 46 / 47 | 36 / 47 | – | **47 / 47** | 40 / 47 |

- **Q8_0 loses nothing.** The 4B at Q8_0 writes byte-for-byte the text the bf16 model wrote on
  all 47 pages. The 0.8B at Q8_0 differs on one page and scores the same.
- **Q4_K_M costs accuracy where it matters most:** degraded mixed lines (1.52% → 3.19% for the
  4B, 5.9% → 10.1% for the 0.8B) and digits (1.38% → 1.95%). Ship Q8_0.
- **CPU and Metal agree** on 20 of the 24 pages run on both, and on all 4 for the 4B at Q8_0.
  The four that differ are mixed or photocopied pages. The 4B Q4_K_M's one difference is an en
  dash for an em dash.
- **Q8_0 is no slower than Q4_K_M on the CPU** (143 s against 148 s for the 4B), so Q8_0 is the
  build everywhere.

**The two failing gates do not move.** The runtime reproduces the reference, so the 4B still
misses mixed Arabic/English (2.07% against 2%) and numbers (93.6% of fields against 99%; 97.0%
without "64-bit"). Both need the pipeline steps in Consequences 3, not a different runtime.

**Speed and memory.** This Mac (M3 Max, 14 cores) had other sessions' builds running, so these
are noisy. Peak RSS is `llama-server`'s; on Apple silicon it includes the Metal buffers.

| Model | Backend | Pages timed | Seconds per page | Peak RSS | Weights + encoder |
|---|---|---|---|---|---|
| 0.8B Q8_0 | Metal | 47 | 2.7 | 3.8 GiB | 1.0 GiB |
| 0.8B Q4_K_M | Metal | 47 | 2.2 | 4.9 GiB | 0.7 GiB |
| 4B Q8_0 | Metal | 47 | 8.4 | 14.2 GiB | 5.1 GiB |
| 4B Q4_K_M | Metal | 47 | 7.6 | 9.9 GiB | 3.3 GiB |
| 0.8B Q8_0 | CPU only | 8 | **46** | 3.1 GiB | 1.0 GiB |
| 0.8B Q4_K_M | CPU only | 8 | 50 | 2.6 GiB | 0.7 GiB |
| 4B Q8_0 | CPU only | 4 | **143** | 11.1 GiB | 5.1 GiB |
| 4B Q4_K_M | CPU only | 4 | 148 | 7.6 GiB | 3.3 GiB |

(Through Transformers the 0.8B took 677 s for one page on the CPU and 2.9 s on Metal; the 4B took
10.2 s on Metal.) The CPU pages are every 6th (0.8B) or 12th (4B) page of the set, so they
include the severe, vocalized and mixed conditions.

- **On the CPU, reading the image is the cost.** A page is 2,420 image tokens. Encoding and
  prefilling them takes 95–97% of the time: 44–49 s for the 0.8B, 128–147 s for the 4B. Writing
  the ~130 output tokens takes 2–5 s. A smaller input would be faster, but its accuracy was not
  measured, and accuracy comes first.
- **Usable as a background job.** At 46 s per page the 0.8B reads a 10-page document in about 8
  minutes on this CPU, and the 4B in about 25. Slower laptop CPUs will take two to four times as
  long. Spec §8 sets no speed target for quality mode and asks for the most accurate engine that
  runs "even slowly".

## Decision

- **Primary recogniser: Nuqta-Arabic-OCR 4B.** Use the "as written" plain-text prompt and the
  card's 2.4 MP input size. It is the default wherever the hardware can run it, which is the
  spec's best-quality tier.
- **Runtime: llama.cpp with the authors' Q8_0 GGUF builds** (follow-up above). Q8_0 reproduces
  the bf16 text exactly; Q4_K_M is not shipped. How the app drives llama.cpp (a bundled
  `llama-server` beside the app, or a Rust binding crate) is an integration choice for the
  pipeline task. Either way its licence is checked then, and the workspace's `unsafe` ban
  stays in our crates.
- **Ensemble partner: Nuqta-Arabic-OCR 0.8B,** for two kinds of re-reading:
  - **Lines.** It re-reads every line holding a word under the primary's confidence threshold
    (start at 0.95; finding 3).
  - **Numbers, dates and IDs.** It reads every one of them, whatever the confidence, because the
    4B is certain of its wrong digits. A third reader from another family reads them too: the
    platform's own OCR where there is one (Apple Vision on macOS).

  Any disagreement leads to a re-read with other preprocessing; if it persists, the word or field
  is flagged for review. The pipeline never picks a reading silently. On this set the rule would
  have flagged all five wrong digit fields; without the third reader, four, since both Nuqta models
  wrote ١٣٣٤ on the photocopy. Where the platform has no OCR, the third reader is still open. The ensemble is there to catch errors, not to vote the
  text into shape.
- **Machines without a usable GPU: Nuqta 4B on the CPU** (decided by the owner, 2026-11-22).
  Through llama.cpp the 4B runs on the CPU alone, and the spec puts the most accurate engine
  first if it runs even slowly. On this CPU it takes 143 s per page (Q8_0, 11 GiB peak memory).
  Through Transformers the 0.8B took 677 s per page on the CPU, because Transformers falls back
  to an unoptimized implementation of Qwen3.5's linear-attention layers there.
  - **Nuqta 0.8B alone** (Q8_0, 46 s per page on the CPU, 3 GiB) is only for machines whose
    memory cannot hold the 4B.
  - **Still open:** that memory threshold. It is set once the 4B is timed on a typical Windows
    laptop.
- **Quick draft pass (searchable within moments):** the platform's built-in OCR where there is one
  (Apple Vision on macOS: 0.27 s per page, 9.5% CER). Elsewhere it is Tesseract `ara+eng`, marked
  as a draft and replaced page by page. Windows' built-in OCR is not measured yet.
- **Not chosen:** QARI-OCR (invents text), PaddleOCR-VL-1.6 as run (weak Arabic recognition),
  PP-OCRv5 Arabic as a recogniser (finding 7), Baseer (no weights for documents).

This changes the spec's §8 engine table, which expected PP-OCRv5 as the classical "default on weak
hardware and second opinion". The measurements put Nuqta in both roles. PP-OCRv5's detector,
with its Arabic recogniser as a guide, provides the word boxes (ADR 0031). The spec's §8 engine
table and its §9 "OCR runtime" row now say this (updated 2026-11-22, at the owner's request).

## Known limits

- **Synthetic set only.** There are no real scans yet (owner item 9). Every page was rendered by
  us, so the set may favour models trained on synthetic renders of the same common fonts, Nuqta
  included. The text itself is ours, so it cannot have leaked into any model's training data. The
  severe conditions are synthetic too. Real faxes, photocopies, phone photos, stamps and
  letterheads may rank the engines differently.
- **Small set.** 47 pages and 171 fields, so one page moves a category by several points.
- **Commercial editors not measured.** Acrobat and ABBYY FineReader need the owner. Apple Vision is
  the only commercial OCR measured.
- **Runtime measured on macOS only.** llama.cpp at Q8_0 reproduces the bf16 text on Metal. CPU
  runs cover 24 pages on an M3 Max; a typical Windows or Linux laptop CPU, and llama.cpp's other
  GPU backends (Vulkan, CUDA), are not measured.
- **Word boxes** come from a separate detector and an alignment step: ADR 0031.
- **Model risk.** Nuqta is a month old, from a three-person team. The weights are Apache-2.0, so
  they can be pinned and mirrored. Before shipping, check what its training data was and under
  which terms.
- **Timings** come from one Mac with other builds running. PP-OCRv5's numbers are PaddlePaddle's
  CPU backend on macOS, not ONNX Runtime (spec §8's plan).

## Consequences / what M4 does next

1. ~~Re-measure Nuqta through a shippable runtime.~~ Done (follow-up above): llama.cpp Q8_0
   matches bf16. The owner made the 4B the default on CPU-only machines too. Next: time it on a
   typical Windows laptop and set the memory threshold below which the 0.8B is used instead.
2. ~~Word boxes.~~ Spiked and measured: ADR 0031.
3. Fix in the pipeline, not the engine: logical order for left-to-right runs inside right-to-left
   lines ("bit-64"), and the digit check above. Then re-check the mixed and numbers gates.
4. The owner: real scans (item 9), and if possible Acrobat Pro or ABBYY on `ocr-eval/out/images/`
   for the commercial comparison.
5. Gates stay as they are (spec: only ever tightened). `degraded-arabic-severe` gets a gate once
   real scans calibrate it.
6. The benchmark-in-CI gate (spec §8) waits for a pipeline. The stored v2 baselines are what it
   will compare against.
