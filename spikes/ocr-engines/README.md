# Spike: which OCR engine(s) the quality pipeline is built on (M4)

Throwaway prototype behind [ADR 0030](../../docs/adr/0030-ocr-engines.md). Question: which
engine reads Arabic best on `ocr-eval/` set v2, and which pair or fallback should the M4 pipeline
use? Nothing here is integrated into the app. The measured tables and the recommendation are in
the ADR. The raw outputs land in `spikes/out/ocr-engines/` (git-ignored).

**Short answer.** Nuqta-Arabic-OCR 4B (Apache-2.0) reads the set best: 0.89% CER overall, no
reading-order errors, and right-to-left tables in the right cell order. Its remaining errors are
confident wrong digits on degraded pages, which its own token probabilities do not flag, and a few
substituted words under severe damage, which they do flag. Nuqta 0.8B, as a second reader, catches
most of the rest with few false alarms. Tesseract (34% CER), PP-OCRv5 Arabic (32%), PaddleOCR-VL-1.6
(29%) and both QARI-OCR versions trail; QARI 0.4 also invents words.

## Setup (once; asks nothing of the repo)

Everything heavy lives in `~/Library/Caches/safha-dev/ocr-bench/` (`SAFHA_OCR_BENCH_CACHE`),
outside the repo and outside cloud-synced folders.

```bash
brew install tesseract                                   # 5.5.4, Apache-2.0
B=~/Library/Caches/safha-dev/ocr-bench; mkdir -p $B/tessdata
for f in ara eng; do curl -L -o $B/tessdata/$f.traineddata \
  https://github.com/tesseract-ocr/tessdata_best/raw/main/$f.traineddata; done
uv venv --python 3.12 $B/venv
uv pip install --python $B/venv/bin/python -r spikes/ocr-engines/requirements.txt
export HF_HOME=$B/hf
for r in PaddlePaddle/PaddleOCR-VL-1.6 SalahALHaismawi/Nuqta-Arabic-OCR-0.8B \
         SalahALHaismawi/Nuqta-Arabic-OCR-4B NAMAA-Space/Qari-OCR-v0.3-VL-2B-Instruct \
         NAMAA-Space/Qari-OCR-0.4.0-VL-4B-Instruct Qwen/Qwen3-VL-4B-Instruct; do
  $B/venv/bin/hf download $r; done
node ocr-eval/generate.mjs
```

PaddleOCR downloads its PP-OCRv5 models into `$B/paddlex` on first use.

For the llama.cpp re-measure and the word-box spike (ADR 0030 follow-up, ADR 0031):

```bash
brew install llama.cpp                                    # 1.0.0, MIT; llama-server
for r in 4B 0.8B; do for f in Q8_0 Q4_K_M mmproj-f16; do
  $B/venv/bin/hf download SalahALHaismawi/Nuqta-Arabic-OCR-$r-GGUF \
    nuqta-ocr-$(echo $r | tr A-Z a-z)-$f.gguf; done; done # Apache-2.0, ~9.5 GiB in all
```

## Run

```bash
spikes/ocr-engines/run_all.sh                 # every engine, then the comparison tables
$B/venv/bin/python spikes/ocr-engines/run_tesseract.py --langs ara   # the Arabic-only Tesseract row
$B/venv/bin/python spikes/ocr-engines/run_vlm.py nuqta-4b --confidence
python3 spikes/ocr-engines/agreement.py nuqta-4b nuqta-0.8b apple-vision
python3 spikes/ocr-engines/confidence.py nuqta-4b+conf nuqta-0.8b apple-vision
spikes/ocr-engines/run_all.sh nuqta-4b        # one engine
$B/venv/bin/python spikes/ocr-engines/run_vlm.py nuqta-4b --only form   # a few pages

# Nuqta through llama.cpp (engine name nuqta-<size>-<quant>-<metal|cpu>)
$B/venv/bin/python spikes/ocr-engines/run_llamacpp.py nuqta-4b --quant Q8_0
$B/venv/bin/python spikes/ocr-engines/run_llamacpp.py nuqta-0.8b --quant Q8_0 --cpu --stride 6

# Word boxes (ADR 0031): truth from Chrome, detector lines, then alignment + scores
node spikes/ocr-engines/word_boxes_truth.mjs
$B/venv/bin/python spikes/ocr-engines/wb_detect.py
$B/venv/bin/python spikes/ocr-engines/wb_align.py nuqta-4b-q8_0-metal --verbose
```

Each runner writes `spikes/out/ocr-engines/hyp/<engine>/<sample>.txt` and a `run.json` with
the settings, seconds per page, peak memory and model size. VLM runners also keep the model's raw
answer (`<sample>.raw.txt`). `summarize.py` scores every engine with `safha-ocr-eval` and prints
the tables pasted into the ADR.

## What was built

| File | Engine | How it runs |
|---|---|---|
| `run_apple_vision.py` | Apple Vision | `ocr-eval/engines/apple-vision.swift`, the M0 reference |
| `run_tesseract.py` | Tesseract 5.5.4, tessdata_best | CLI, LSTM, automatic page segmentation, the page's true dpi; `--langs ara+eng` (default) or `ara` |
| `run_ppocr.py` | PP-OCRv5 server detector + Arabic mobile recogniser | PaddleOCR 3.x on CPU; lines put in reading order by a simple RTL-aware rule (no columns) |
| `run_vlm.py` | QARI 0.3, QARI 0.4, PaddleOCR-VL-1.6, Nuqta 0.8B / 4B | 🤗 Transformers on Metal (MPS), bf16, greedy; each model's own prompt and input size from its card |
| `to_text.py` | — | Removes Markdown/HTML markup from VLM answers, identically for all; never changes content |
| `summarize.py` | — | Scores every engine and prints the per-category tables |
| `adr_tables.py` | — | The compact tables in ADR 0030, from `ocr-eval/baselines/*.set-v2.json` |
| `agreement.py` | — | Where a second engine disagrees with the primary: errors caught, false alarms, oracle CER |
| `confidence.py` | — | Whether the model's own token probabilities (`run_vlm.py --confidence`) flag its errors, alone and with a partner |
| `run_llamacpp.py` | Nuqta 0.8B / 4B GGUF (Q8_0, Q4_K_M) | `llama-server` loaded once, one request per page; Metal, or CPU only (`--cpu`: `-ngl 0 --no-mmproj-offload`); records the server's token timings and peak RSS |
| `word_boxes_truth.mjs` | — | Ground-truth word boxes for the 26 geometry-preserving pages: Chrome's DOM word rectangles, scaled to the image |
| `wb_detect.py` | PP-OCRv5 server detector (+ Arabic recogniser as a guide) | Line polygons and guide text per page, with detector and pipeline timings |
| `wb_align.py` | — | VLM words → detected lines → word boxes (methods M0–M5, ADR 0031); scores line assignment and IoU against the truth |

Notes from running them:

- **PaddleOCR-VL-1.6 ships `use_cache: false`.** In Transformers that re-runs the image prefix for
  every token and ends answers early, so `run_vlm.py` passes `use_cache=True`.
- **PP-OCRv5 on PaddlePaddle's CPU backend (macOS)** needs a 46 GiB footprint for a 300 dpi page
  at the detector's default resolution. `run_ppocr.py` caps the detector's input at 2048 px
  (`--det-side`); recognition still reads full-resolution crops. `--isolate` runs a process per
  page, which is safe but slow (model load each time).
- **Greedy decoding is deterministic here.** The `nuqta-4b+conf` run wrote the same text as
  `nuqta-4b` on all 47 pages.

Timings are noisy: other sessions were building on the same Mac. They are recorded, not gated
(spec §8: no speed target for quality mode).
