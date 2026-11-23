# Mirsam (مرسام) — PDF viewer & editor

Desktop PDF viewer/editor (Tauri 2: Rust core + React UI) with first-class Arabic/RTL and
quality-first OCR. The product is **Mirsam** (مرسام in Arabic UI text). Its working name was
Safha, and code identifiers (`safha-core`, `SAFHA_*` variables, the `safha:` URI scheme) keep
that name until the identifier rename noted in `PROGRESS.md`; do not rename them piecemeal.

**Read `docs/SPEC.md` before any non-trivial work.** It is the source of truth for scope,
priorities, quality bars, and licensing. `PROGRESS.md` says where we are; `docs/adr/` records
why things are the way they are. Update `PROGRESS.md` at the end of every session.

## Setup (once per machine)

```bash
scripts/dev-setup.sh                    # fetches PDFium; redirects build output if the checkout is cloud-synced
(cd apps/desktop && pnpm install)
node fixtures/generate.mjs              # PDF test corpus -> fixtures/out/ (needs Chrome, qpdf)
```

Needs: Rust (pinned in `rust-toolchain.toml`), Node 24 + pnpm 9, qpdf, Chrome. On this Mac
rustup is keg-only: `export PATH=/opt/homebrew/opt/rustup/bin:$PATH`.

## Commands

| Task | Command (from repo root unless noted) |
|---|---|
| Run the app | `cd apps/desktop && pnpm tauri dev` (add `--features devtools` for self-test/spikes) |
| UI only, mock backend | `cd apps/desktop && pnpm dev` → http://localhost:1420 |
| Rust tests | `cargo test --workspace` |
| Rust lint / format | `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all` |
| UI tests / types / lint | `cd apps/desktop && pnpm test` · `pnpm typecheck` · `pnpm lint` (`pnpm format` fixes) |
| End-to-end shell check | `cargo build -p safha-desktop --features devtools && scripts/selftest.sh <pdf> <out> [en\|ar] [light\|dark]` then `node scripts/check-selftest.mjs <out> <ltr\|rtl>` |
| Installers (PDFium inside, checked) | `node scripts/bundle.mjs [--target <triple>] [--features devtools]` (never bare `tauri build`; ADR 0060) |
| Self-test an installer | `scripts/bundle-selftest.sh <dmg\|deb\|rpm\|AppImage\|setup.exe\|msi> <out>` (bundle built with `--features devtools`) |
| Fuzz the open/parse path | `node fuzz/seed.mjs && cp Cargo.lock fuzz/ && cd fuzz && cargo fuzz run <open\|file\|objects\|content\|cmap>` (fuzz/README.md) |
| Licence check | `cargo deny check licenses bans sources` · `node scripts/third-party-licenses.mjs [--check]` |
| Scroll/zoom benchmark (window must be visible) | `scripts/scroll-benchmark.sh <out> debug\|release` then `node scripts/check-scroll-benchmark.mjs <out>` |
| Page text / extraction report | `cargo run -p safha-core --example page_text -- f.pdf [page]` · `--example fixture_report [-- --verbose]` |
| Release-mode tests | set `SAFHA_PDFIUM_DIR=$PWD/third_party/pdfium/<platform>/lib` (release builds don't search `third_party/`) |
| Render a page to PNG | `cargo run -p safha-core --example render_png -- in.pdf out.png [page] [scale]` |
| Text-extraction baseline | `cargo run -p safha-core --example fixture_report` |
| Search index cost (release) | `SAFHA_PDFIUM_DIR=… cargo run --release -p safha-core --example search_bench -- f.pdf [query…]` |
| OCR eval set + score | `node ocr-eval/generate.mjs` · `cargo run -p safha-ocr-eval -- --manifest ocr-eval/out/manifest.json --hyp <dir> --engine <name> --targets ocr-eval/targets.json` |
| OCR benchmark (CI gate, ADR 0036) | `cargo run --release -p safha-core --example ocr_bench -- --out <dir> --replay ocr-eval/recordings/set-v2` (`--record … --model-dir <dir>` with the models) |
| OCR a whole PDF headless (ADR 0035) | `cargo run --release -p safha-core --example ocr_document -- in.pdf out.pdf --models <store> --state <dir>` · `ocr_smoke --models <store> …` installs models |

A task is done when: code + tests pass, clippy/biome are clean, no performance budget regressed
(spec §11), and it works in **both English (LTR) and Arabic (RTL)**.

## Layout

```
crates/safha-core/       UI-independent engine (PDFium binding, documents, rendering). No Tauri here.
crates/safha-ocr-eval/   OCR scorer: CER/WER, field accuracy, reading-order errors, gates
apps/desktop/src-tauri/  Tauri shell: window + thin typed commands over safha-core
apps/desktop/src/        React UI: backend/ (IPC + mock), state/, commands/, components/, i18n/, styles/
fixtures/                content/*.json (ground truth) → generate.mjs → out/ (PDFs, not committed)
ocr-eval/                evaluation set definition, targets, baselines; out/ is generated
assets/fonts/            OFL fonts (fixtures now, bundled text-engine fonts later)
spikes/                  throwaway prototypes behind ADRs; not in the default build
fuzz/                    cargo-fuzz targets (own workspace, nightly); never ships
scripts/                 setup, PDFium fetch, self-test, licence report
third_party/pdfium/      downloaded binaries (git-ignored)
```

## Conventions

- **Arabic/RTL is never a follow-up.** Any text-handling code is Unicode/bidi-correct in its first
  version. CSS uses logical properties only (`margin-inline-start`, `inset-inline-end`, …) — never
  `left`/`right`. Wrap user-supplied names in `<bdi>`. Directional icons set `mirrorInRtl`.
- **Design tokens only.** No raw colours, sizes, or durations in components; add a token in
  `src/styles/tokens.css` (and its contrast pair in `tokens.test.ts`).
- **Every user action is a command** in `src/commands/registry.ts`; buttons and shortcuts call
  `runCommand`. No ad-hoc handlers that bypass it. Sole exception: continuous gestures (pinch,
  Ctrl/Cmd+wheel zoom), which the viewer handles directly (ADR 0009).
- **Every user-facing string** exists in both `en.json` and `ar.json` (a test enforces parity).
- **The UI never touches the filesystem.** Files are opened by Rust commands; the UI gets ids and
  file names, not paths. Do not add Tauri fs/shell permissions.
- **Bitmaps reach the webview as images the webview decodes itself**: BMPs over the `safha` URI
  scheme, loaded by `<img>` (ADR 0011). Never base64/JSON, and never as JavaScript buffers
  (`ArrayBuffer`, `Blob`): WebKit keeps those hundreds of megabytes too long.
- **Engine logic lives in `safha-core`**, so the CLI and tests reach it without a webview. The shell
  crate only adapts.
- Rust: `unsafe` is denied workspace-wide; no `unwrap()` outside tests; errors are typed
  (`thiserror`) and reach the UI as a stable `code` that maps to a localized message.
- Tests never skip silently: a missing fixture corpus is a failure with instructions.
- **Licences** (spec §10): MIT/BSD/Apache-2.0/ISC/Zlib/MPL-2.0/OFL only. No GPL/AGPL (MuPDF,
  Poppler, Ghostscript, iText). LGPL or custom model licences: ask first. Verify a dependency's
  current version, maintenance, and licence before adding it.
- Never commit copyrighted or confidential PDFs/scans. Fixtures are generated.
- **OCR: accuracy beats speed** in every trade-off (spec §8). OCR jobs have no speed budget.
- Spikes (`[SPIKE]` in the spec) get a throwaway prototype and an ADR with measurements before the
  real implementation.
- The pinned PDFium build (`scripts/fetch-pdfium.sh`) and `pdfium-render`'s `pdfium_NNNN` feature
  must be bumped together.
