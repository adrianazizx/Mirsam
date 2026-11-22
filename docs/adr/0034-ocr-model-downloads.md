# 0034 — Downloading the OCR models

Status: Accepted for M4. The runtime's entries wait for the owner (licences, below). Date: 2026-11-22.
Builds on ADR 0030 (which models), ADR 0032 (`llama-server` as a child process).

## Context

Spec §8 says the models are downloaded on demand, not bundled. The download must be resumable,
and its size explained before it starts. ADR 0030 chose Nuqta-Arabic-OCR 4B (5.5 GB with its
vision encoder) and the 0.8B (1.1 GB). ADR 0032 runs them through llama.cpp's `llama-server` and
expects the download manager to fetch that program too. The UI never touches the filesystem, so
all of this lives in Rust.

## Decision

**`safha_core::models`** owns it, so tests and a future CLI reach it without a webview.

- **Catalog.** Every file is pinned by URL, size and SHA-256:
  - **The models:** the authors' GGUF repositories at the revisions ADR 0030 measured
    (`f4fdd78`, `4f72054`), Q8_0 plus the f16 vision encoder. The file names are the ones
    `ocr::NuqtaModel` loads.
  - **The runtime:** llama.cpp release `b12359`, one build per platform. macOS uses the Metal build
    on Apple silicon and the CPU build on Intel. Windows and Linux use the Vulkan builds: they carry
    the CPU backends as well and load Vulkan only where a driver exists. The CUDA builds are left
    out: they need NVIDIA's CUDA runtime, whose licence is outside spec §10. NVIDIA GPUs still work
    through Vulkan.
  - **Packages.** A package is what the user installs or deletes as one item (a model with its
    encoder; the runtime). Each has a role: primary, ensemble, draft or runtime. OCR needs the
    primary and the runtime.
- **Store layout** (`<app data>/ocr-models`, or `SAFHA_MODELS_DIR`):
  - **Installed packages:** `<id>/` holds the files and a receipt, `.mirsam-package.json`. The
    receipt is written last, inside the staging folder. A package counts as installed only when
    its receipt is there and its files are.
  - **Downloads:** `.downloads/<sha256>.part`, named by checksum, so a re-pinned file never
    resumes onto old bytes.
  - **Staging:** `.staging/<id>-<nonce>/`, renamed into place. An older version is moved to
    `.trash/` first and deleted after. A crash leaves the old install or the new one, never half
    of one; leftovers are removed at the next start.
- **Download.** ureq 3 (already chosen by ADR 0032).
  - **TLS:** rustls on ring, checked against the operating system's trust store
    (rustls-native-certs), so a company's own root certificate works where the OS trusts it.
    `rustls-platform-verifier` was tried first. It pulls `webpki-root-certs` (CDLA-Permissive-2.0)
    for wasm32, which `cargo deny` rejects across all targets.
  - **Resume:** `Range: bytes=N-`. A 200 answer restarts from zero; a 416, or a range that starts
    elsewhere, starts over.
  - **Checks:** the bytes already on disk are hashed before resuming, so the final SHA-256 covers
    the whole file. A mismatch, or a server sending more bytes than pinned, deletes the file:
    `model_checksum_mismatch`.
  - **Failures:** dropped connections, timeouts, 408/425/429 and 5xx are retried up to four times
    in a row, with a doubling wait. A try that received bytes resets the count.
  - **Timeouts:** 30 s to connect, and 30 s of silence on a connection. The silence limit also
    bounds how long a cancel waits on a stalled connection.
  - **Redirects:** followed (Hugging Face serves through a CDN).
- **Disk space.** Checked before downloading through `fs4`. The space needed is what is left to
  download, plus four times each archive's size for unpacking, plus 256 MiB. Too little:
  `model_disk_full` with both numbers, and nothing is fetched. A full disk while writing reports
  the same.
- **Cancel** stops at the next 256 KiB piece and keeps what arrived. The UI then shows the
  package as paused, with Resume and Delete.
- **Delete** moves the folder out of sight first, then removes it, and discards partial downloads.
- **Archives.** `.tar.gz` (macOS, Linux) through `flate2` + `tar`; `.zip` (Windows) through our own
  ~150-line reader on `flate2` (stored and deflated entries, CRC-checked). These archives are
  checksummed before unpacking, but entries are still refused if they could land outside the
  folder: absolute paths, `..`, drive prefixes, and symlinks pointing out. The runtime's program
  is found where the catalog says, or as the one file of that name two levels down.
- **Hardware.** `Hardware::detect` reports, without `unsafe`:
  - **Accelerators:** Metal on Apple silicon; CUDA and Vulkan on Windows (`nvcuda.dll`,
    `vulkan-1.dll`); on Linux the NVIDIA driver, and a Vulkan loader with a non-llvmpipe driver.
  - **Memory:** `sysctl`, `/proc/meminfo`, or PowerShell's CIM query.

  This is reported in Settings and the prompt; recognition does not use it yet.
- **Shell** (`models.rs`):
  - **Commands:** `ocr_models`, `download_ocr_model` (starts a thread and returns at once),
    `cancel_ocr_model_download`, `delete_ocr_model`.
  - **Progress:** `ocr-model-progress` events to every window, at most every 150 ms.
  - **One download per package.** A second request gets `model_busy`. This is tracked inside
    the process; the single-copy work (ADR 0046, on the viewer-shell branch) keeps a second copy
    of the app from downloading the same file. Without it, the worst case is a shared part file
    that fails its checksum and is deleted.
- **UI.**
  - **Settings** (Cmd/Ctrl+,): a dialog whose first section is the OCR models. Each row shows the
    name, role, size, licence and state; a progress bar; Download, Resume, Update, Cancel, and
    Delete with a confirmation. Every action is a registry command.
  - **The OCR tool's inspector panel:** the model's state, *Recognize text*, and the way to
    Settings.
  - **First use.** `ocr.recognize` without the model opens a prompt first. It says what the
    model is, that it runs locally, its size (less what is already downloaded), and that OCR will
    be slow when there is no usable GPU. *Download* opens Settings, where the download shows.

## Licences (spec §10)

| Package | Licence | Checked |
|---|---|---|
| Nuqta-Arabic-OCR 4B / 0.8B GGUF | Apache-2.0: LICENSE file and card in each repo; base models Qwen3.5-4B / 0.8B are Apache-2.0 | 2026-11-22, Hugging Face API and files |
| llama.cpp `b12359` builds | MIT (ggml, cpp-httplib, nlohmann/json, stb_image), BoringSSL Apache-2.0 (macOS, Windows), LLVM OpenMP Apache-2.0 WITH LLVM-exception (Windows `libomp.dll`, shipped with its licence) | 2026-11-22, release workflow and `vendor/` at the tag |
| — outside the list | sheredom `subprocess.h` (Unlicense) and miniaudio (Unlicense OR MIT-0), compiled into `llama-server`/`mtmd`; the built-in web UI's npm packages (not audited; the app passes `--no-webui`) | asked the owner |

**The runtime is therefore not in the catalog yet.** `runtime_package` defines and tests the six
builds, and `Catalog::for_platform` adds them once the owner agrees. Until then `llama-server`
comes from the machine, as ADR 0032's tests use it.

New crates are all permissive and maintained:

| Crate | Version | Licence |
|---|---|---|
| ureq | 3.5.2 | MIT OR Apache-2.0 |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT |
| ring | 0.17 | Apache-2.0 AND ISC |
| rustls-native-certs | 0.8.5 | Apache-2.0 OR ISC OR MIT |
| tar | 0.4.46, no default features | MIT OR Apache-2.0 |
| flate2 | 1.1.11, on miniz_oxide | MIT OR Apache-2.0 |
| fs4 | 1.1.0 | MIT OR Apache-2.0 |

`cargo deny check licenses bans sources` passes.

## Evidence

- **Rust unit tests (28 in `models`):**
  - **Against a local HTTP server** (`models/test_server.rs`): a fresh install with monotonic
    progress; cancel, then resume through a redirect with exactly one `Range: bytes=N-`
    request; two dropped connections, then completion; 503s retried; a server without range
    support; wrong checksum and wrong size refused and deleted; a 404 not retried; too little
    disk space downloading nothing.
  - **Archives:** tar.gz and zip runtimes unpacked, with their program found; a missing program
    leaving no staging folder; version replacement; delete; a damaged install not counting; two
    packages installed at once.
  - **Archive safety:** escaping paths and links, CRC damage, junk input.
  - **Catalog:** HTTPS pins for every platform; no CUDA builds.
  - **Hardware:** detection.
- **Live test** (`tests/model_download_live.rs`, `--ignored`): fetches a 1.1 MB file from the
  Nuqta repository through Hugging Face's CDN redirect, cancels at 400 KB, then resumes and
  verifies. It passed on 2026-11-22. No model was downloaded.
- **Shell tests:** the event and listing JSON the UI reads, and job tracking.
- **UI tests:** the store and helpers; the first-use prompt; download, progress and
  installation; cancel, resume, and delete with confirmation; failure messages with disk sizes;
  `model_busy`; the inspector panel; Arabic RTL rendering with Western digits.
- **Browser build:** checked in English (light theme) and Arabic (dark theme). The progress bar
  fills from the inline start (right in RTL).

## Consequences / open

1. **The owner** decides on the runtime's licences (PROGRESS.md). With a yes, the runtime joins
   the catalog as a required package. The prompt then names both downloads in its total.
2. **The OCR pipeline** finds its files through `ModelsState::installed` (shell) and
   `ModelStore::installed` (core): `ServerConfig::for_model(program, &installed.dir, model)`.
3. **Code signing (M7).** A downloaded `llama-server` is not signed by us. On macOS, files our
   process writes carry no quarantine flag, and a child process is not subject to the app's
   library validation, so it runs. Notarization rules may still prefer re-hosting signed builds.
   This needs checking with the packaging work (ADR 0032 assumed bundling).
4. **System libraries on Linux.** The Linux builds use the system's `libgomp`, OpenSSL 3 and
   (Vulkan) `libvulkan`. The `.deb`/`.rpm` dependencies should name them.
5. **The memory threshold** for falling back to the 0.8B (ADR 0030) can use
   `Hardware::memory` once it is set.
6. **The draft reader** needs no download on macOS (Apple Vision). Tesseract elsewhere is not
   packaged yet; `Role::Draft` is there for it.
