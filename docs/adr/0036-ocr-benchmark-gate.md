# 0036 — The OCR benchmark gate in CI: recorded model answers, replayed

Status: Accepted (M4). Date: 2026-11-23.
Builds on ADR 0006 (the scorer), ADR 0030 (the evaluation set), ADR 0033 (the pipeline),
ADR 0035 (the text layer).

## Context

Spec §8: "CI runs the benchmark on every change to the OCR pipeline and fails on any
regression in CER/WER." §11 repeats it. The pipeline's engines are Nuqta 4B and 0.8B
through `llama-server` (ADR 0032). On GitHub's hosted runners that is not practical:

- **Size.** The models are 6.6 GB with their vision encoders.
- **Speed.** Hosted runners have no usable GPU. On an M3 Max's CPU the 4B takes 141 s
  for one page read (ADR 0032); the pipeline makes about ten requests a page (ADR 0033). The
  47-page set would take many hours on a 4-core runner, on every pull request.
- **Determinism.** A model run is deterministic on one machine (greedy decoding), but
  CPU and GPU arithmetic differ in the last bits, so even a full run on a runner would
  not reproduce the Metal run the baselines came from.

The models change rarely (their versions are pinned, ADR 0034). What changes often is
our code around them: assessment, line strips, which words are re-read, how readings
are reconciled, the order fix, word boxes, the text layer. That is what the gate must
watch.

## Decision

**Gate on recorded model answers replayed through the current pipeline, and run the real
models in a small smoke job.** (`.github/workflows/ocr-benchmark.yml`)

### Recording (`safha_core::ocr::replay`, `examples/ocr_bench.rs`)

- `ocr_bench --record ocr-eval/recordings/set-v2 --model-dir <dir>` runs the full
  quality pipeline over the evaluation set with the real engines on a Mac.
- `Recorder` wraps each engine and keeps every answer: the text, each word's
  probability, whether it was cut off, and the image it answered (its size and an 8×8
  grid of mean brightness). The draft reader's lines (Apple Vision, the word boxes'
  guide, ADR 0035) are kept too.
- One JSON file per sample, committed (about 1 MB for the set).

### Replaying

- `ocr_bench --replay ocr-eval/recordings/set-v2` runs the same pipeline with `Replay`
  engines. Everything else runs for real.
- **Matching.** A request is matched to a recorded answer of the same engine by size
  first, then by the brightness grid. A few pixels of slack in each direction lets a
  strip boundary that moved by rounding still find its answer.
- **A request with no close recording is an error that names the image.** The pipeline
  asked the models something new, so the recordings must be made again. It never guesses.

### What is scored

The bench writes two hypothesis sets, and the gate scores both with the existing scorer
(`safha-ocr-eval`, ADR 0006):

1. **The pipeline's text.** Against the spec's targets (`ocr-eval/targets.json`) and the
   baseline `ocr-eval/baselines/pipeline-nuqta-4b+0.8b.set-v2.json`.
2. **The searchable PDF's text.** Each page goes into a one-image PDF, its text layer is
   written by our writer (ADR 0035), and the page is read back by the app's own
   extractor: what search and copy see. Scored against
   `pipeline-layer-nuqta-4b+0.8b.set-v2.json`.

Any regression fails, in CER, WER, fields, reading-order errors, table cells, or cell
order. That is the scorer's existing rule, with no tolerance beyond float noise.

### Where it runs

- **The gate runs on `macos-latest`.** The evaluation set's pages are drawn by Chrome
  (`ocr-eval/generate.mjs`). macOS's Chrome draws them as they were drawn when the
  recordings were made. Linux's would differ in every glyph's antialiasing.
- **The smoke job runs on `ubuntu-latest`.**
  - `examples/ocr_smoke.rs` installs the llama.cpp runtime and Nuqta 0.8B through the
    app's own download manager: the pinned catalog with checksums, cached between runs by
    the catalog's hash.
  - It starts `llama-server` on the CPU, reads one clean Arabic page, and fails above
    5% CER.
  - This covers what replay cannot: the download, the runtime build, the request, and
    parsing the answer.
- **Triggers:** pull requests and pushes that touch `ocr/`, `models/`, `text/`, the bench
  examples, the scorer, or `ocr-eval/`, plus "Run workflow".

## Consequences

- **A pull request that changes the pipeline's requests must re-record**, with the models,
  on a Mac with a GPU: about 25 minutes for the set. The new baselines go in the same
  pull request. This is deliberate. A change to what the models are asked is a change to
  what is measured, and it must be measured with the models.
- **The model weights themselves are not re-tested in CI.** They are pinned by SHA-256
  (ADR 0034). A new version is a deliberate change, re-measured with `ocr_bench --record`.
- **The smoke job downloads 1.1 GB when its cache is cold.** That happens when the catalog
  changes, or after GitHub evicts the cache (7 days unused).
- **Not covered by CI:**
  - the 4B on a CPU (too slow for a hosted runner);
  - the Vulkan and Metal backends (no GPU there);
  - Windows (the runtime's Windows build is checked only by its checksum).

## Alternatives considered

- **Run the 0.8B over the whole set on CI.** It is the partner, not the pipeline, so it
  would not gate the pipeline. On a CPU it would still take about an hour. Rejected.
- **Self-hosted runner with a GPU.** It would gate the real models. But the owner would have
  to keep a machine online for a public repository, which carries security risks. Rejected
  for now; revisit when real scans join the set (owner item 9).
- **Store hypothesis files only, and score them.** That checks the scorer, not the
  pipeline, so it misses the regressions that matter. Rejected; the recordings replay
  through the code instead.
