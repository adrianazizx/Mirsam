# 0032 — Running the OCR models: `llama-server` as a child process

Status: Accepted for the M4 pipeline. Date: 2026-11-22.
Builds on ADR 0030 (Nuqta through llama.cpp at Q8_0).

## Context

ADR 0030 settled what reads the page: Nuqta-Arabic-OCR 4B, and 0.8B as the second reader, through
llama.cpp with the authors' Q8_0 GGUF builds. At that precision the output is identical to the
full model. It left open how the app drives llama.cpp. There were two ways:

1. **Link llama.cpp into our process** through a Rust binding crate.
2. **Run llama.cpp's own `llama-server` as a child process** and talk to it over its
   OpenAI-style HTTP API.

The forces that decide it:
- **Memory.** The 4B holds 11–14 GiB while it reads (ADR 0030). The 0.8B holds 3–4 GiB.
- **Crashes.** llama.cpp aborts on internal assertions and on failed allocations.
- **Our build.** It runs on macOS, Windows and Linux in CI, and must build with `unsafe` denied in
  our crates (CLAUDE.md).
- **Backends.** Metal on macOS; CPU, Vulkan or CUDA elsewhere.

## Decision

**`llama-server` runs as a child process,** started for an OCR job and stopped when the job ends.
The engine side is `safha_core::ocr`:

- **`LlamaServer`.**
  - Starts the program with the model card's settings: `-c 8192 --image-max-tokens 4096 --temp 0`;
    on a CPU-only machine also `-ngl 0 --no-mmproj-offload`.
  - Waits for `/health` and stops the process on drop.
  - Reports a stable error code for each failure: `ocr_engine_missing`, `ocr_model_missing`,
    `ocr_engine_failed`, `ocr_engine_stopped`, `ocr_timeout`, `ocr_bad_response`.
- **Private to this run.**
  - It listens on `127.0.0.1` only, on a free port.
  - Every request must carry a random 128-bit key made for this run (`--api-key`).
  - Its stdin is closed; its output goes to a log file or nowhere.
  - The unauthenticated `/health` endpoint only says whether the model is loaded.
- **`NuqtaEngine`** (an `OcrEngine`) sends one chat request per image:
  - The image is a base64 PNG, resized exactly as the benchmark resized it. `OcrImage::resize`
    reproduces Pillow's Lanczos filter bit for bit, so the model sees the same pixels.
  - The card's prompt, `enable_thinking: false`, greedy decoding, and `logprobs`.
  - From the token probabilities, each word gets a confidence: the lowest probability among its
    tokens, even when a token splits a UTF-8 character.
- **HTTP** is `ureq` 3.5.2 without TLS (MIT OR Apache-2.0, maintained; it adds three crates).

Why not link it:
- **Process isolation** is what matters most. When the job ends, the OS takes back every byte of
  the model's 11 GiB. An abort inside llama.cpp (an out-of-memory failure on a smaller machine,
  say) ends the child, not the editor holding the user's unsaved work. A linked library would need
  to free everything itself and could take the app down with it.
- **No C++ in our build.** A binding crate compiles llama.cpp with CMake for each backend (Metal,
  CUDA, Vulkan), which costs CI time on three OSes and makes Qwen3.5 support depend on the
  binding's release cadence. llama.cpp publishes prebuilt, MIT-licensed `llama-server` binaries for
  every platform and backend.
- **No `unsafe` boundary** in our crates, and no FFI binding to keep current.
- **The cost is small:** one process start per job, plus encoding a 1–3 MB PNG per page. Against
  8–143 s per page (ADR 0030) the request overhead does not show (measured below).

## Evidence

The Rust path was run over `ocr-eval` set v2 with `crates/safha-core/examples/ocr_eval.rs`:
Nuqta 4B Q8_0, Metal, M3 Max, the brew `llama-server` 1.0.0.

| | Python harness (ADR 0030) | Rust, `safha_core::ocr` |
|---|---|---|
| Text on the 47 pages | — | **byte-identical on 47 of 47** |
| CER, all pages | 0.89% | 0.89% (no regression against `nuqta-4b-q8_0-llamacpp.set-v2.json`) |
| Seconds per page, Metal | 8.4 | 8.3 |
| Seconds per page, CPU only (4 pages) | 143 | 141 (same text on all 4) |
| Server start, model already in the file cache | — | 1.3 s (Metal), 4.8 s (CPU) |

The request overhead (encoding the PNG, HTTP, JSON with token probabilities) does not show
against the model's time. The `ocr_page` example read rendered fixture pages (English, Arabic,
and a rotated page) in 7–10 s each.

Unit tests cover:
- the sizing rule;
- the resize, against Pillow's output;
- PNG round trips;
- markup removal (the benchmark's cases);
- word confidence across split UTF-8 characters;
- parsing answers and errors;
- the command line;
- a missing program, a missing model, and a server that stops at once.

`crates/safha-core/tests/ocr.rs` recognises two clean pages exactly with the real model. It is
`#[ignore]`d with instructions, because CI cannot hold the weights.

## Consequences

- **The model download manager** (an M4 item) fetches `llama-server` with the models: the build
  for the platform and backend, checksum-verified. The llama.cpp version is pinned and bumped
  deliberately, like PDFium's (Qwen3.5 support needs a recent build). The models load with
  `-m`/`--mmproj` paths, so they can live in the app's data folder.
- **Code signing.** The bundled binary must be signed and notarized with the app on macOS, and
  signed on Windows. It listens on loopback only, so no firewall prompt is expected; this is to be
  confirmed on Windows.
- **Background jobs** (a later slice) own one `LlamaServer` per job and model. Starting the 4B
  takes the load time measured above, so a job reuses its server for all its pages.
- **`OcrError` is not yet in `safha_core::Error`.** The first shell command that runs OCR maps its
  codes and adds the localized messages.
