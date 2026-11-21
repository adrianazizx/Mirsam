# Progress

Mirrors the milestones in `docs/SPEC.md` §12. Update at the end of every session.
Last updated: 2026-11-22 (session 1).

Legend: `[x]` done and verified · `[~]` done but with a stated gap · `[ ]` not started.

## Needs the project owner

Nothing here blocks Milestone 1, but each is a decision or an action only you can take.

1. **Move the repo out of OneDrive?** The checkout is in a OneDrive folder. `scripts/dev-setup.sh`
   keeps Cargo's `target/` and pnpm's package store out of it, but `.git` is still synced, and
   sync clients are known to corrupt git repositories under concurrent writes. A plain folder
   (e.g. `~/Developer/safha`) with a git remote for backup is safer.
2. **Nothing is committed yet.** `git init` was run; committing was left to you.
3. **CI has never run.** `.github/workflows/ci.yml` is written and its YAML parses, but there is
   no remote. Expect first-run fixes, especially on Windows and Linux.
4. **Licence allowlist additions.** Beyond the spec's list, `deny.toml` allows `Unicode-3.0`,
   `CC0-1.0`, `0BSD`, and `MIT-0`. All are permissive and all arrive through Tauri's own
   dependency tree (Unicode data, ICU4X), so they cannot be avoided on this stack — but the spec
   says to ask.
5. **Digits in the Arabic UI**: Western (current default) or Arabic-Indic for page counts and
   zoom? Probably a setting; which default? (ADR 0003.)
6. **Product name and bundle identifier.** `Safha` / `app.safha.desktop` are placeholders.
7. **Shell PATH.** Homebrew's rustup is keg-only. To use `cargo` in your own terminal:
   `export PATH=/opt/homebrew/opt/rustup/bin:$PATH` (not added to `~/.zshrc` for you).
8. **More fixture producers and OCR baselines need installs** (none done without asking):
   LibreOffice, a TeX distribution (XeLaTeX), Tesseract with `ara`/`eng` best models. Word and
   InDesign fixtures need those apps on a machine.
9. **Real scans for `ocr-eval/scans/`.** The set is synthetic only. It needs real Arabic scans
   (fax, photocopies, phone photos, stamps, letterheads) that we may redistribute, each proofread
   by a person. This is the most valuable thing to start collecting now.
10. **Which PDF readers matter most for copy/paste of text we write?** The spike found that no
    writing recipe makes vocalized Arabic extract correctly in both PDFium (Chrome, Edge) and
    PDFKit (macOS Preview), that `/ActualText` does not help, and that the two want opposite
    ToUnicode conventions. Acrobat Reader and pdf.js are untested — testing Acrobat needs it
    installed. The spec's §7 acceptance test on "another viewer" needs rewording once that is
    measured (ADR 0005).
11. **Arabic UI strings** were written by the assistant, not a native-speaker reviewer (the spec
    schedules that review for M7; earlier is better).

Spec §3 open decisions D1–D7: all proceeding on the stated assumptions; none has been questioned
by anything found so far.

## M0 — Foundations

- [x] Repo scaffold: Cargo workspace, `safha-core`, `safha-ocr-eval`, Tauri shell, React UI
- [x] `CLAUDE.md`, `docs/SPEC.md`, `docs/adr/` (7 records), this file, `docs/demos/m0.md`
- [~] CI on Windows/macOS/Linux — workflow written; **never executed** (no remote yet)
- [x] Tauri shell with window; capabilities locked down (no fs/shell/dialog for the webview)
- [x] PDFium bound (pinned build 8125, checksum-verified) and rendering a page to a canvas —
      verified in the real WKWebView in English/LTR and Arabic/RTL via `scripts/selftest.sh`
- [~] Test corpus and fixture generators — 20 PDFs: 8 cases × Chrome and CoreText producers, plus
      linearized, encrypted, truncated, and 1,000-page variants. **Missing producers:** Word,
      LibreOffice, XeLaTeX, InDesign, Tesseract-OCR'd scan. **Missing kinds:** form-heavy, scanned
- [x] Design tokens (light/dark, WCAG AA enforced by test) and base components (Button,
      IconButton, Tooltip, CommandButton; tool rail, tab bar, inspector, status bar, error banner,
      empty state, gallery) in LTR + RTL
- [x] Command registry with platform shortcuts; every button goes through it
- [x] Licence check: `cargo deny` + `scripts/third-party-licenses.mjs` (511 components, all
      allowed); both wired into CI
- [x] Spike: tile transport → ADR 0004 (native PDFium + raw binary IPC)
- [x] Spike: PDFium edit API vs own content-stream writer → ADR 0005. Result reverses the
      expectation: PDFium's CID-font API *can* place pre-shaped glyphs with our subset font, and
      is the safer serializer; the own writer stays as a second backend
- [x] `ocr-eval/`: scorer (CER/WER, fields, reading order, gates, regression check), 30 synthetic
      samples in 9 categories, first baseline (Apple Vision) stored
- [x] Performance budgets that apply to M0 are met (table below); the rest need M1+ features to measure

### M0 measurements (Apple-silicon MacBook, macOS 26)

| Budget (spec §11) | Target | Measured | Notes |
|---|---|---|---|
| Cold start to usable window | ≤ 1.5 s | **0.50–0.62 s** | Release `.app`, 5 launches: process start → UI up, document opened, first page on canvas, self-test report written (so it overstates the real figure). Debug build + dev server: 0.8–1.3 s |
| First page visible after open | ≤ 300 ms | 19–25 ms request → pixels on canvas | A4 at 1824×2582 px; engine render 7–12 ms |
| 1,000-page document opens without loading all pages | bounded | open 9.6 ms; page 1,000 renders in 19.7 ms | Debug build. Memory over a long session not measured yet |
| Installer size without OCR models | ≤ 60 MB | **6.7 MiB** `.dmg` (14.6 MiB `.app`) | macOS arm64, unsigned; 7.0 MiB of the app is PDFium |
| Scrolling 60 fps, typing latency, search ≤ 1 s | — | not applicable yet | Features arrive in M1/M3 |

### Test status at end of session

- Rust: `safha-core` 11 tests (7 engine + 4 corpus), `safha-ocr-eval` 20 tests, `safha-desktop`
  1 test (bitmap wire format) — all passing;
  `clippy -D warnings` clean with and without `devtools`; `cargo fmt --check` clean.
- UI: 58 tests in 7 files passing; `tsc` and Biome clean.
- End to end (real app): English/light, Arabic/dark, and the encrypted-file error path pass in the
  debug shell; English and Arabic pass in the bundled release app (PDFium loaded from the bundle,
  production CSP in force).
- Not run: anything on Windows or Linux.

### Known gaps carried forward

- Bundling ships PDFium for macOS arm64 only (`tauri.macos.conf.json`). Intel/universal macOS,
  Windows (`pdfium.dll` beside the exe), and Linux bundle paths are still to configure and test.
- The self-test reads back the canvas, not the composited window (ADR 0007).
- `ocr-eval` degraded category is too easy; tables need cell-level scoring (ADR 0006).
- Preferences live in `localStorage` until the SQLite store exists (M1).
- No fuzzing of the open/parse path yet (spec §11); the truncated fixture is the only malformed input.

## M1 — Viewer (P0 viewing)

Starting points already in place: `Document::render_tile`, the command registry, the 1,000-page
fixture, the extraction baseline (46/118 blocks verbatim), ADR 0004.

- [ ] Render scheduler: one prioritized queue with cancellation, tile cache keyed by
      (page, zoom, tile, revision), low-res preview then sharpen (PDFium is single-threaded per
      process — ADR 0002)
- [ ] Continuous scroll, zoom (pinch, Ctrl+scroll), HiDPI
- [ ] View modes: single / continuous / two-page / book; RTL page order; read `/Direction`
- [ ] Thumbnails, outline, page labels, go-to-page
- [ ] Tabs (real: close, reorder), multiple windows
- [ ] Bidi-aware text extraction (spec §6.1) and selection/copy in logical order
- [ ] Search with Arabic normalization (spec §6.2)
- [ ] Recents, session restore, per-file position (SQLite)
- [ ] Reading themes (sepia, night); command palette; printing
- [ ] Open password-protected PDFs (the engine already accepts a password; the UI has no prompt)
- [ ] Re-run the tile-transport spike on Windows and Linux
- [ ] Exit test: 1,000-page bilingual PDF at 60 fps; Arabic copy and search correct on all fixtures

## M2 — Annotate and organize

- [ ] Markup, notes, text boxes, ink
- [ ] Comments panel
- [ ] Page organizer (reorder/rotate/delete/merge/split/extract)
- [ ] Undo journal
- [ ] Safe save pipeline (temp → validate with reopen + `qpdf --check` → atomic rename)
- [ ] Autosave and recovery

## M3 — Text engine

- [ ] Layout/shaping engine (itemize → shape → break → position), LTR and RTL together
- [ ] Font matching and subsetting
- [ ] **First:** verify PDFium content regeneration when existing objects are deleted/replaced
      (clipping, z-order, marked content) — the one open check from ADR 0005
- [ ] Text pipeline → positioned glyph runs; emit through PDFium's CID Type 2 font API (ADR 0005)
- [ ] Own content-stream writer as second backend where PDFium cannot serve (incremental save, PDF/A)
- [ ] Add-new-text tool
- [ ] In-place editing with reflow — **[SPIKE]** editor surface vs `contenteditable` overlay first
- [ ] Exit test: all acceptance tests in spec §7

## M4 — OCR (quality-first)

- [ ] Engine benchmark and ADR (QARI-OCR, Baseer, PP-OCRv5 Arabic, Tesseract; must beat Tesseract
      and commercial editors in every Arabic category)
- [ ] Quality pipeline: assess, preprocess, layout analysis, recognition, re-recognition/ensemble,
      verification with no silent invention
- [ ] Background jobs with draft-then-quality results; survives restart
- [ ] Word-level confidence, pages-needing-review list
- [ ] Searchable-PDF output in logical order
- [ ] Model download manager; hardware acceleration
- [ ] Benchmark gate in CI
- [ ] Exit test: targets and ship gates in spec §8 met on `ocr-eval/`

## M5 — Forms, sign, protect

- [ ] Form filling with shaped appearance streams
- [ ] Fill & Sign on flat forms
- [ ] Drawn/typed/image signatures
- [ ] Encryption and permissions
- [ ] True redaction with pattern search
- [ ] Sanitize
- [ ] Form creation; certificate signatures (PAdES) — **[SPIKE]** crypto library first

## M6 — Edit everything else + convert

- [ ] Images, links, object manipulation
- [ ] Headers/footers, page numbers, watermarks
- [ ] Find and replace
- [ ] Compression; image export; text/Markdown export
- [ ] OCR review mode, region re-OCR, replacing poor text layers, scan-to-editable-text

## M7 — v1.0 polish

- [ ] Accessibility pass
- [ ] Native-speaker Arabic localization review
- [ ] Onboarding, auto-update, installers and code signing
- [ ] Opt-in crash reporting
- [ ] Performance hardening

## Session log

### Session 1 — 2026-11-22

Built Milestone 0 from an empty folder. Installed on this machine: rustup + Rust 1.100, qpdf,
cargo-deny (all via Homebrew). Decisions and measurements are in ADRs 0001–0007.

Things that were not obvious and are worth knowing next time:

- Spec library check: `rustybuzz` has had no release since 2024-11; `harfrust` (0.16, released
  this month) is the maintained HarfBuzz port. `pdfium-render` 0.9.6 binds PDFium up to build 8125.
- pnpm refuses a symlinked `node_modules`; the supported route for a cloud-synced checkout is
  `virtual-store-dir`, which `scripts/dev-setup.sh` writes into a git-ignored `.npmrc`.
- `pdfium-render` applies a render origin only on its form-data path; the matrix path ignores it.
- A webview whose window is occluded, or whose display is asleep, gets no animation frames. A
  self-test run hung for exactly that reason until the harness got a timer fallback.
- Base64-in-JSON IPC left 600 MB resident in the shell after one benchmark pass (ADR 0004).
- `FPDFText_SetText` does no shaping, but `FPDFText_LoadCidType2Font` + `SetCharcodes` +
  `SetPositions` takes pre-shaped glyphs and our own subset — the second spike overturned the
  assumption that we must write content streams ourselves (ADR 0005).
- `lopdf` silently drops encryption when re-saving an encrypted file. Do not put it in the save
  path for user files without guards.
- The CI clippy step covers the spike crate too (`--workspace`); it is clean today.
