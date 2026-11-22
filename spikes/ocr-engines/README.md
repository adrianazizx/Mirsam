# Spike: which OCR engine(s) the quality pipeline is built on (M4)

Throwaway prototype behind [ADR 0030](../../docs/adr/0030-ocr-engines.md). Question: which
engine reads Arabic best on `ocr-eval/` set v2, and which pair or fallback should the M4 pipeline
use? Nothing here is integrated into the app. The measured tables and the recommendation are in
the ADR. The raw outputs land in `spikes/out/ocr-engines/` (git-ignored).

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

PaddleOCR downloads its PP-OCRv5 models (and, for `run_paddleocr_vl.py`, its layout model) into
`$B/paddlex` on first use.

## Run

```bash
spikes/ocr-engines/run_all.sh                 # every engine, then the comparison tables
spikes/ocr-engines/run_all.sh nuqta-4b        # one engine
$B/venv/bin/python spikes/ocr-engines/run_vlm.py nuqta-4b --only form   # a few pages
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

Timings are noisy: other sessions were building on the same Mac. They are recorded, not gated
(spec §8: no speed target for quality mode).
