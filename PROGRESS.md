# Progress

Mirrors the milestones in `docs/SPEC.md` §12. Update at the end of every session.
Last updated: 2026-11-22 (session 6).

Legend: `[x]` done and verified · `[~]` done but with a stated gap · `[ ]` not started.

## Needs the project owner

Nothing here blocks Milestone 1, but each is a decision or an action only you can take.

1. **Move the repo out of OneDrive?** The checkout is in a OneDrive folder. `scripts/dev-setup.sh`
   keeps Cargo's `target/` and pnpm's package store out of it, but `.git` is still synced, and
   sync clients are known to corrupt git repositories under concurrent writes. A plain folder
   (e.g. `~/Developer/safha`) with a git remote for backup is safer.
2. **The code is on GitHub** as the public repository `adrianazizx/Mirsam` (public since
   2026-11-22): `main` (Milestone 0) and `m1-viewer` (Milestone 1), open as pull request #1 with
   Auto-fix on. It has no licence file, so the code is all rights reserved until you choose one
   (spec D2).
3. **CI on pull request #1:**
   - **Green:** licences; the macOS tests; and the shell self-test on macOS, Linux and Windows,
     including printing through GTK on Linux.
   - **The Linux tests** failed because Chrome on Linux draws some word spaces differently. The
     extractor now handles both ways (ADR 0008); the fix is pushed and passes locally on the
     Linux-made corpus. A green CI run is still to be seen.
   - **The Windows tests** have not yet finished a run.
4. **Licence allowlist additions.** Beyond the spec's list, `deny.toml` allows `Unicode-3.0`,
   `CC0-1.0`, `0BSD`, and `MIT-0`; and the app now compiles in **SQLite** (public domain, SPDX
   `blessing`) for the local state store. All are permissive and all arrive through Tauri's own
   dependency tree (Unicode data, ICU4X), so they cannot be avoided on this stack — but the spec
   says to ask.
5. **Digits in the Arabic UI**: Western (current default) or Arabic-Indic for page counts and
   zoom? Probably a setting; which default? (ADR 0003.)
6. **Bundle identifier.** The product is now **Mirsam** (مرسام) and the identifier
   `app.mirsam.desktop`. Before the first signed release it should be the reverse of a domain you
   own; changing it later moves where the app keeps its state (recents, session, positions).
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
12. **Does the 500 MB memory budget include WebKit?** On the 1,000-page benchmark (release build)
    the shell stays under the budget at 339 MB, but the shell and WebKit together reach 726 MB.
    WebKit alone needs about 155 MB with just one page on screen. A budget for the shell only, or
    a separate one for the webview (say 400 MB), is achievable today; 500 MB in total is not
    without deeper work. ADR 0011 has the numbers. Which do you want?
13. **Try Print once (Cmd/Ctrl+P).** On macOS and Windows, printing goes through the platform's
    own PDF viewer (ADR 0012): a window shows the PDF and the system print panel opens over it.
    No session so far could see the screen, so it is unverified. If you see a blank window, or
    the panel prints only part of the document, say so. The fallback is the Linux path below,
    which prints pages as images.

    Linux uses GTK's print dialog, with each page drawn at the printer's resolution (ADR 0013).
    CI checks it by printing to a file; it has not been tried on a desktop with a real printer.

Spec §3 open decisions D1–D7: all proceeding on the stated assumptions; none has been questioned
by anything found so far.

## M0 — Foundations

- [x] Repo scaffold: Cargo workspace, `safha-core`, `safha-ocr-eval`, Tauri shell, React UI
- [x] `CLAUDE.md`, `docs/SPEC.md`, `docs/adr/` (7 records), this file, `docs/demos/m0.md`
- [~] CI on Windows/macOS/Linux — runs on GitHub. A fully green run on PR #1 is still to be seen
      (owner item 3)
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
- ~~Preferences live in `localStorage` until the SQLite store exists (M1).~~ Done in session 3:
  the store holds them; `localStorage` keeps a copy for the first frame only.
- No fuzzing of the open/parse path yet (spec §11); the truncated fixture is the only malformed input.

## M1 — Viewer (P0 viewing) — in progress on branch `m1-viewer`

- [x] Render scheduler: one render thread, priority queue, cancellation by viewport generation,
      64 MiB byte-capped LRU keyed by (document, revision, page, scale, rect), previews then tiles
      (ADR 0009)
- [~] Continuous scroll, zoom (pinch, Ctrl+scroll), HiDPI — measured with a visible window
      (ADR 0011). Release build, 1,000 pages:
      - scrolling at 4,000 and 12,000 px/s and flinging drops no frames;
      - 0.2% of all frames are dropped;
      - pinch-in is clean;
      - no page is blank longer than 12 ms; random jumps show a preview within 16 ms.
      **Still over budget:** an extreme pinch-out (×1/20 in 1.2 s) drops 3 frames, p95 25 ms.
- [x] View modes: single / continuous / two-page / book; RTL page order. A document's own
      `/ViewerPreferences /Direction` and `/PageLayout` set its defaults, read by our own safe
      catalog reader (ADR 0012): an Arabic book that asks for right-to-left spreads opens that way
- [x] Thumbnails, outline, page labels, go-to-page (ADR 0010) — thumbnail strip on its own render
      lane; outline tree; the page field shows and accepts labels (`i`, `1`, `2`) and Arabic-Indic
      digits. Verified in the real app in both directions
- [x] Tabs and windows (ADR 0012):
      - tabs: close button, middle-click, Ctrl+Tab / Ctrl+Shift+Tab, reordering by drag and by
        Ctrl+Shift+PageUp/PageDown in reading order;
      - windows: New Window (Cmd/Ctrl+N) and Move Tab to New Window; a window's documents close
        with it; the session restores every window; settings follow across windows.
      Checked in the real app: the self-test opens a second window
- [x] Bidi-aware text extraction (spec §6.1) — core (ADR 0008): 86/118 corpus blocks verbatim
      (was 46), 69/70 on pages with a trustworthy mapping, all broken-mapping pages flagged for OCR;
      identical text on pages rotated 0/90/180/270°
- [x] Selection and copy (ADR 0010) — selects visually, copies logically; word/line/page
      selection, floating Copy button, Edit-menu copy. In the real app, select-all on page 1 copies
      exactly the page's text in English and Arabic runs
- [x] Search with Arabic normalization (spec §6.2) — side panel with five options, hits marked on
      pages, ⌘G/⇧⌘G; 1,000 pages: index 1.09 s once, then 18 ms per search (release)
- [x] Recents, session restore, per-file position (SQLite) — wired: start screen lists recents
      (missing files marked), a second launch restores tabs, each file reopens where it was left
- [x] Reading themes (normal, sepia, night) and command palette (⌘K, matches English and Arabic
      titles in either UI language)
- [~] Printing — Cmd/Ctrl+P:
      - macOS and Windows print through the platform's PDF viewer (ADR 0012). Built, but not yet
        seen working on screen (owner item 13).
      - Linux uses GTK's print dialog, with PDFium drawing each page at the printer's resolution
        (ADR 0013). CI prints the four rotated pages to a file and checks them.
- [x] Open password-protected PDFs — inline prompt; wrong password asks again; never stored
- [x] Tiles on Windows and Linux. The shell self-test passes on WebView2 and on WebKitGTK in
      CI: first page 99–155 ms on virtual machines (ADR 0011, session 6 amendment). The memory
      experiment is WebKit-on-macOS only.
- [~] Exit test, part 1 (the 1,000-page bilingual PDF at 60 fps): passes except extreme
      pinch-out (above), and memory if WebKit counts (owner question 12). Session 6 tested three
      causes of the pinch-out drops; none was it (ADR 0011).
- [~] Exit test, part 2 (Arabic copy and search correct on all §6.6 fixtures):
      - Pages with a trustworthy text layer: 69 of 70 blocks verbatim, and every word of their
        ground truth is found by search as typed (542 words, `tests/search.rs`).
      - Copy in the real app equals the extracted text (self-test).
      - The one miss is `(64-bit)` in an Arabic line, which is drawn exactly like `(bit-64)`.
      - Pages with a broken mapping are flagged for OCR (M4).
      - The same holds for the fixtures Chrome makes on Linux (62/63; ADR 0008).

Known extraction gap (floors in `crates/safha-core/tests/text.rs`): `(64-bit)` inside an Arabic
line comes out `(bit-64)`. The two are drawn identically, and the same file's `INV-2026-00417`
needs the other reading (ADR 0008, session 6 amendment). Everything else missed is on pages
flagged for OCR.

To re-measure (screen unlocked; the release bundle sometimes opens hidden, and such a run is
rejected as invalid, so run it again):

- debug: `cargo build -p safha-desktop --features devtools && scripts/scroll-benchmark.sh <out> debug`
- release: `(cd apps/desktop && pnpm tauri build --features devtools --bundles app) && scripts/scroll-benchmark.sh <out> release`

Test status at the end of session 6:

- **Rust:** 123 tests pass on this Mac. Clippy is clean with and without `devtools`, `cargo deny`
  is ok, and the licence report is current. The Linux-only printing code is compiled and linted
  only by CI.
- **UI:** 122 tests pass. Biome, `tsc` and the production build are clean.
- **End to end:** self-tests pass on macOS, Linux and Windows in English/light and Arabic/dark.
  The scroll benchmark ran visibly in a debug build (pinch-out 7 dropped frames).

Remaining M1 work:

- confirm printing on screen on macOS and Windows (owner item 13);
- pinch-out frames (needs WebKit's own profiler);
- the memory budget decision (owner item 12).

## M2 — Annotate and organize

- [ ] Markup, notes, text boxes, ink
- [ ] Comments panel
- [ ] Page organizer (reorder/rotate/delete/merge/split/extract)
- [ ] Undo journal
- [ ] Safe save pipeline (temp → validate with reopen + `qpdf --check` → atomic rename)
- [ ] Autosave and recovery

## M3 — Text engine — groundwork merged to `main` ahead of M2

The parts that do not depend on M2, built on branch `pomidor/m3-text-engine-3c22b6` in
parallel with M2 and merged by pull request at the owner's request.

- [x] **ADR 0005's open check: PDFium's content regeneration is lossy** (ADR 0005 amendment,
      `spikes/pdfium-regenerate/`).
      - Deleting, replacing, or merely dirtying one object and regenerating damages untouched
        content on 27 of 34 pages, and `qpdf --check` passes on every damaged file.
      - Type 3 text vanishes (19 pages). Simple fonts sharing a `/BaseFont` are merged, which
        turns Arabic into tofu (7 CoreText pages). Inline images, `sh`, text clipping and
        `Tc`/`Tw`/`Tz` are dropped.
      - Clips, z-order and marked content survive where fonts are not hit. Appending is safe.
      - **Decision: PDFium never regenerates existing content. Text is written by our own
        content-stream writer as incremental updates**, fitting M2's ADR 0014. PDFium renders,
        verifies, and does the full rewrite on save.
- [x] Layout/shaping engine, LTR and RTL together (ADR 0020, `safha_core::typeset`):
      - itemize: bidi, script runs, font fallback per grapheme cluster;
      - shape with `harfrust` 0.16.1;
      - break per UAX #14 (ICU4X);
      - position: per-line reordering, alignment, justification by spaces.
      Output: lines of glyph runs with positions and glyph↔text clusters, backend-neutral.
      0.24–0.34 ms per 306-character mixed paragraph, release build (noisy machine).
- [x] Text pipeline → positioned glyph runs. Proven end to end by a test writer: PDFium draws
      the result, and `safha-core`'s extractor reads back exactly the input, in English, Arabic,
      mixed, and justified mixed text with fallback.
- [x] Font matching and subsetting (one gap, below):
      - Subsetting done (`typeset::subset_fonts`, `subsetter` 0.2.6). Variable fonts are
        instanced. The subsets draw pixel-identical pages from files 33–44× smaller.
      - Matching done (`typeset::match_font`): family, then classification and style, then
        `/Widths`, with text coverage first. It reports exact, same family, or substitute.
      - Installed fonts done (`typeset::FontLibrary`): indexes the platform's font folders from
        a few small tables each (694 faces in 25–45 ms on this Mac, debug build). `find` reads
        the best few in full and instances variable fonts at the requested weight.
      - **Gap:** reusing the PDF's own embedded font when it has the glyphs (in-place-editing
        work).
- [ ] **Own content-stream writer** (now the only backend). Waits for M2's
      `safha_core::pdf` writer to merge. Must solve:
      - ToUnicode for multi-glyph clusters (Noto Naskh's shadda + fatha extracts twice with
        `/ActualText`);
      - encrypting new objects in encrypted files.
- [ ] Add-new-text tool
- [ ] In-place editing with reflow — **[SPIKE]** editor surface vs `contenteditable` overlay first
- [ ] Exit test: all acceptance tests in spec §7

## M4 — OCR (quality-first)

- [~] Engine benchmark and ADR 0030 (proposed; side track on branch `pomidor/m4-ocr-benchmark-f94419`):
  - **Primary recogniser: Nuqta-Arabic-OCR 4B.** 0.89% CER over the set, no reading-order errors.
  - **Ensemble partner: Nuqta 0.8B.** It re-reads low-confidence lines and every number, date
    and ID, with the platform's OCR as a third reader for fields.
  - **Weak-hardware default:** Nuqta 0.8B, if a native runtime makes it usable without a GPU.
    Through Transformers on the CPU it takes 677 s per page.
  - **Quick draft:** Apple Vision on macOS, Tesseract elsewhere.
  - **Not chosen:** QARI-OCR (invents words), PaddleOCR-VL-1.6 (weak Arabic), PP-OCRv5 Arabic
    (32% CER; drops Latin words in Arabic lines), Baseer (no weights released).
  - **Beats:** plain Tesseract and Apple Vision in every Arabic category.
  - **Gaps:** synthetic set only; Acrobat and ABBYY not measured; accuracy measured in Python on
    Metal, not yet through the runtime the app would ship.
  - **Gates the best engine still fails:** mixed Arabic/English (2.07% CER against 2%) and
    numbers (93.6% against 99%; 97.0% without "64-bit").
- [x] `ocr-eval` set v2 (47 samples):
  - Severe scans (a new tracked category) and an Arabic form.
  - Table-cell scoring.
  - Either column first on bilingual side-by-side pages.
  - Baselines for nine engine runs in `ocr-eval/baselines/*.set-v2.json`.
- [ ] Re-measure Nuqta through a shippable runtime (GGUF/llama.cpp or MLX), and its speed on CPU
- [ ] Word boxes for VLM text: detector + alignment (PP-OCRv5's detector is the first candidate)
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

### M4 side track — 2026-11-22 (OCR engine benchmark, branch `pomidor/m4-ocr-benchmark-f94419`)

- **ADR 0030 (proposed): Nuqta-Arabic-OCR 4B as the recogniser, Nuqta 0.8B as its partner.**
  Nine engine runs over the eval set:
  - **Apple Vision**, the reference.
  - **Tesseract 5.5.4** with tessdata_best, in two configurations.
  - **PP-OCRv5 Arabic.**
  - **QARI-OCR 0.3 and 0.4.**
  - **PaddleOCR-VL-1.6.**
  - **Nuqta-Arabic-OCR 0.8B and 4B.**

  Baseer was skipped: the document model was never released as weights, and Baseer-Nakba is
  non-commercial. Every model, tool and Python environment lives under
  `~/Library/Caches/safha-dev/ocr-bench` (about 27 GB). The owner approved each download.
- **Eval set v2** (47 samples; v1's 30 unchanged):
  - **Severe scans:** fax, second-generation photocopy, faded toner, curved phone photo.
  - **An Arabic-only form.**
  - **Table-cell scoring in the scorer.**
  - **Either column first** on bilingual side-by-side pages. Before this, an engine that read the
    Arabic column first lost 84% CER on a page it read perfectly.
- **Findings that shape the pipeline:**
  - **Self-confidence fails on digits.** Nuqta 4B is certain (p ≥ 0.9995) of the wrong digits it
    writes on degraded pages, so numbers need a second reader whatever the confidence. Its own
    confidence does flag the words it invents.
  - **QARI 0.4 rewrites words.**
  - **"(64-bit)" in an RTL sentence** comes out as "bit-64" from every engine.
- **Harness:** `spikes/ocr-engines/`. It has runners, an agreement and confidence analysis, and
  the tables for the ADR.
- **Needs the owner (M4):**
  - Real scans (item 9).
  - If possible, Acrobat Pro or ABBYY run on `ocr-eval/out/images/` for the commercial
    comparison.
  - An OK before downloading Nuqta's GGUF or MLX builds to re-measure through a shippable runtime.
- **Tests:** `cargo test --workspace`: 130 passed, 26 of them in `safha-ocr-eval`, with 6 new.
  `clippy -D warnings` and `cargo fmt --check` are clean. The UI was not touched.

### Rename to Mirsam — 2026-11-22

The owner chose the name **Mirsam** (مرسام). Renamed everything a user or reader sees: window
titles, `productName` (the macOS bundle is now `Mirsam.app`), `app.name` and the
`engine_unavailable` message in `en.json` and `ar.json`, the bundle identifier
(`app.mirsam.desktop`, so the app starts with empty state once), README, CLAUDE.md, the spec
header, two ADRs, and the bundle paths in `scripts/selftest.sh` and
`scripts/scroll-benchmark.sh`. The Arabic word صفحة ("page") stays wherever it means a page.

**Not renamed yet, on purpose:** code identifiers — the crates `safha-core`, `safha-ocr-eval` and
`safha-desktop` (and their directories), `safha_core` imports, `SAFHA_*` environment variables,
the `safha:` URI scheme and CSP entry, the npm package `safha-desktop`. `m2-organize` and the OCR
branch have committed and uncommitted work inside those crates, so renaming them now would make
every one of their files conflict. Do it in one change once those branches have merged, and
update CLAUDE.md, the spec header and the README note when it is done.

### Session M3-a — 2026-11-22 (M3 groundwork, parallel to M2)

Branch `pomidor/m3-text-engine-3c22b6`, cut from `main`, in its own worktree. M2 continued on
`m2-organize` meanwhile; this session touched no UI file. ADR numbers 0020–0029 are reserved
for this track. Merged to `main` by pull request at the owner's request, ahead of M2.

- **ADR 0005 check run:** PDFium's content regeneration is lossy (27 of 34 pages damaged).
  Text moves to our own writer as incremental updates (ADR 0005 amendment).
- **`safha_core::typeset` (ADR 0020):**
  - the layout engine;
  - font subsets with variable-font instancing;
  - font matching;
  - an index of installed fonts (`FontLibrary`).
- **Tests:** 165 Rust tests pass (41 new). Clippy, fmt, `cargo deny` and the licence report
  are clean.
- **New dependencies** (all verified, all allowed):
  - `harfrust` 0.16.1 (MIT);
  - `icu_segmenter` / `icu_properties` 2.4 (Unicode-3.0);
  - `subsetter` 0.2.6 with fontations' `skrifa`/`write-fonts` and linebender's `kurbo`
    (MIT/Apache-2.0);
  - `read-fonts` 0.47 as a direct dependency (it was already in the tree).
- **Timings are noisy:** other sessions were building on the same Mac. The scroll benchmark
  was not run.

Things that were not obvious:

- PDFium's `FPDFPage_GenerateContent` cannot write Type 3 text at all. It emits `BT … Tm` and
  stops, leaving the stream unbalanced. Chrome writes Type 3 for variable fonts, so most of the
  corpus is affected.
- PDFium keys generated font resources by name and type. CoreText's several subsets of one
  face share a `/BaseFont`, so they collapse into one.
- PDFium renders a page from its in-memory object list. Rendering before and after
  save-and-reload is therefore an exact oracle for what serialization lost.
- Noto Naskh Arabic has Latin glyphs of its own, and draws Arabic dots as separate glyphs.
  Fallback tests need a Latin-only first font (IBM Plex Sans).
- An empty ToUnicode destination does not hide a glyph from PDFium: it falls back to the CID
  as a character. `/ActualText` around a one-letter cluster works. Around a two-glyph,
  two-mark cluster, PDFium emits the text once per glyph.
- `subsetter` keeps the hinting tables of a default instance and drops them when instancing,
  so a regular subset can be larger than a bold one.
- On macOS, 112 installed faces have family names starting with a dot. These are hidden UI
  fonts, and they include LastResort, whose `cmap` claims every character. A font index must
  drop them, or LastResort "covers" any text.
- macOS's Courier declares itself neither fixed-pitch nor serif. A curated family table beats
  font metadata.

### CI: docs-only changes skip the build — 2026-11-22

- **The repository went public** on 2026-11-22, so standard GitHub-hosted runners are free and
  CI keeps all three platforms on every pull request.
- **Documentation-only changes** (`docs/`, Markdown files other than `THIRD_PARTY_LICENSES.md`)
  run nothing. "Run workflow" on the Actions tab runs everything on any branch.
- **If the repository goes private again:** CI draws on a $12 monthly Actions allowance (GitHub
  Free), and macOS minutes cost about ten Linux ones. The nine runs on 2026-11-22 used about
  $4.90, 60% of it on macOS. Drop macOS from pull requests first; keep the runs on `main`,
  because they build the caches that pull requests reuse.

### Session 6 — 2026-11-22 (Milestone 1: exit tests, other platforms, Linux printing)

- **CI fixes for PR #1:**
  - The 1,000-page fixture no longer overflows Windows' command line, and no longer depends
    on qpdf reading `@file` arguments, which Ubuntu's qpdf does not.
  - Tests skip CoreText fixtures only where the manifest says so.
  - The macOS self-test gets a warm-up launch.
  - The benchmark's frame budgets warn on CI virtual machines.
- **Text extraction** (ADR 0008 amendment): 85/118 blocks verbatim (was 83), and 69/70 on
  trustworthy pages. The fixes cover Amiri clusters drawn from one origin, a heading alone in its
  column, and bracket pairs mirrored by bidi level. `(64-bit)` is documented as undecidable from
  the page.
- **Search across the corpus:** a new test searches every ground-truth word as typed.
- **Windows and Linux:** the shell self-test runs on both in CI, and passed on its first run.
- **Printing on Linux:** GTK's print dialog, with PDFium drawing each page (ADR 0013). CI
  exports four pages to a file and checks them.
- **Pinch-out:** three hypotheses measured and rejected (ADR 0011).
- **Chrome on Linux** draws some word spaces inside another run. Handled now, so extraction on
  Linux-made fixtures matches macOS (ADR 0008).
- **Printing on Linux** verified in CI: four rotated pages exported in 203 ms, each on paper of
  the right orientation.

### Session 5 — 2026-11-22 (Milestone 1: reading preferences, tabs, windows, printing)

- **Reading preferences:** `safha_core::catalog` reads `/ViewerPreferences /Direction` and
  `/PageLayout` without `unsafe`. It handles classic and stream cross-references, incremental
  updates, and Flate object streams. New fixtures: `derived/rtl-book.pdf` and
  `derived/rtl-book-objstm.pdf` (24 in all).
- **Tabs:** reordering by drag and by keyboard, in reading order.
- **Windows:** multiple windows with per-window sessions (store schema 2) and settings broadcast.
- **Printing:** through the platform's PDF viewer, still unverified.
- **CI:** the first CI run's fixes were pushed as PR #1 (Auto-fix on). PR #1's first run then
  failed three checks, fixed in `40a9331`:
  - Windows: the 1,000-page fixture's qpdf arguments overflowed the command line. An `@file`
    list fixed Windows but Ubuntu's qpdf does not read it, so the fixture is now a two-page file
    whose range `1-2` is repeated 500 times in one argument.
  - Linux and Windows: tests skip CoreText fixtures only where the manifest records the producer
    as unavailable.
  - macOS self-test: an unmeasured warm-up launch, because a cold first launch took 461 ms to the
    first preview.

Status: 118 Rust tests and 121 UI tests pass. Clippy is clean both ways. Self-tests pass, with a
second window opened in the real app and the right-to-left book opening right to left in book
layout.

Things that were not obvious:

- WebKit's print panel cannot be checked from a session that cannot see the screen: the print
  window opened, but `screencapture` was refused and the window reported itself hidden.
- Closing the last window must keep its tabs in the session (they are what the next launch
  restores), and so must quitting, which closes every window. `RunEvent::ExitRequested` marks
  the second case.
- qpdf's JSON round trip (`--json-output=2` → edit → `--json-input`) is a clean way to make
  fixtures with specific catalog entries.

### Session 4 — 2026-11-22 (Milestone 1: the exit-test measurement)

The scroll benchmark ran with a visible window for the first time. In the debug build:

- no blank pages;
- 1–2% slow frames while scrolling, and up to half while pinching;
- 1.37 GB of memory, of which more than 1 GB was WebKit's and was never returned.

Changes:

- **Tile transport replaced (ADR 0011):** tiles are now BMP images over the `safha` scheme,
  loaded by `<img>`. In isolation, WebKit's growth per 2 GB of tiles fell from +170–190 MB to
  +22–39 MB. The core's bitmaps are now BGRA, with `Bitmap::encode_bmp`.
- **Zoom gestures** scale with a CSS transform and `will-change`, and lay out only now and then.
- **Fairer frame rule:** the checker now counts dropped frames.
- **Benchmark detail:** it records memory per phase, and the viewer's own time per phase.

Result, release build:

| | Before | After |
|---|---|---|
| Dropped frames | — | 0.2% |
| Scrolling | 1–2% slow frames | no dropped frames |
| Pinch-out | up to half the frames slow | 3 dropped |
| Shell + WebKit at end | 1.37 GB | 726 MB |

Things that were not obvious:

- Handing tile bytes to JavaScript is what costs memory: an `ArrayBuffer` response or a fetched
  `Blob` alike. WebKit's garbage collector reclaims them late and its heap stays at its
  high-water mark. An `<img>` decodes natively and frees on `src = ""`.
- WebKit rejects a `BITMAPV4` RGBA-bitfields BMP. Plain `BI_RGB` (BGRX) works, and is what PDFium
  renders anyway. One comparison here was confounded: the plain variant also arrived three times
  slower, which gives the garbage collector time. Delivery speed matters as much as format for
  anything that passes through a `Blob`.
- `createImageBitmap` options made no difference to memory.
- Measured with counters, our JavaScript costs under 1 ms a frame. The slow frames were WebKit
  repainting resized pages during zoom.
- A CSS transform without `will-change` re-rasterizes at every new scale (+187 MB per pinch-in).
- An idle WKWebView shows a p95 frame interval of 18–19 ms. A 17.5 ms p95 budget could never
  pass; a missed vsync (over 25 ms) is the meaningful signal.
- The self-test opens the thumbnail panel, and WebKit's `localStorage` copy carried that into the
  next run despite the fresh state store. Settings missing from the store now fall back to their
  defaults, and changes made before the store answers are kept.
- The release bundle opened hidden in 3 of 5 benchmark launches, with the screen unlocked.
- **This machine's typecheck had been too lenient since Milestone 0.** `scripts/dev-setup.sh` puts
  pnpm's packages outside the OneDrive folder; from there, library type files cannot find
  `@types/react`, so every React Aria component was typed `(props: any) => any`. CI's fresh
  install caught a `Tabs` prop error this machine missed. `tsconfig.json` now maps `react` to the
  app's own types, and the two setups agree. This is also why React Aria callbacks seemed to need
  explicit parameter types under TypeScript 7.
- Windows needs `.gitattributes` (`eol=lf`), or the formatters reject a CRLF checkout.

### Session 3 — 2026-11-22 (Milestone 1: navigation, search, selection)

Branch `m1-viewer`. The user-facing half of M1 on top of session 2's core (ADR 0010):

- **Side panel:** thumbnails, outline, search.
- **Selection and copy:** visual select, logical copy.
- **Local state:** recents, session restore and remembered position, wired to the SQLite store.
- **Also:** settings in the store, reading themes, the command palette, and the password prompt.
- **Core:** `view::ViewTransform` (rotation-correct geometry), `search::IndexCache`, page labels in
  one call, `Urgency::Thumbnail`, and position-only duplicate removal (ADR 0008 amendment).
- **Fixtures:** new `derived/rotated.pdf`, 22 in all.
- **Self-test:** now also checks selection, search and thumbnails in the real app.
- **Test isolation:** self-tests use a throwaway state store (`SAFHA_STATE_DIR`).

Things that were not obvious:

- React Aria's `<Tabs>` builds its collection by rendering *everything* inside it. A search
  results `ListBox` placed anywhere inside crashed the app with "Duplicate key 1": hit 1 against
  the tab of document 1. `<Tabs>` now wraps only the tab bar and page area.
- `PointerEvent.detail` is always 0 in Chromium, so double/triple-click must be counted by hand.
- React Aria form validation in `native` mode keeps a server error as the input's custom
  validity, which blocks the next submit: the password prompt could not be retried. `aria` mode
  shows the message without blocking.
- Under TypeScript 7, React Aria's render-prop and callback parameters are not inferred; they need
  explicit types.
- A debounced session save was lost when the app quit within the delay (the self-test does). It is
  now written immediately; tab changes are rare.
- On a rotated page PDFium orders characters by displayed position, so "duplicates are near each
  other in the stream" was false; duplicates are now found by position alone.
- The concurrency stress test holds PDFium's open lock for seconds; sharing a test binary with a
  timing assertion made that flaky. It lives in `tests/concurrency.rs` now.

### Session 2 — 2026-11-22 (Milestone 1 started)

Branch `m1-viewer`. Text extraction and search in the core (ADR 0008); outline and page labels;
the local state store (recents, view state, settings, session);
Chrome fixtures now carry outlines; new `derived/page-labels.pdf`. The tiled viewer is being built in
parallel by a background agent (render service in `safha-core/src/render/`, UI viewer).

Things that were not obvious:

- Chrome writes Arabic in visual order inside `/ReversedChars`, one `/ActualText` span per letter,
  dots as separate glyphs, and a space glyph at every word break. PDFium's character list drops
  most of those spaces; its page-object list still has them, at the right positions.
- `PdfPageTextObject::text()` in pdfium-render rebuilds the page's whole text analysis per call
  (11 s to index 500 pages); `PdfPageText::for_object` on one shared text page takes 0.54 s.
- Release builds do not look in `third_party/pdfium`; set `SAFHA_PDFIUM_DIR` to run release tests.
- PDFium reports a multi-letter Arabic glyph's letters reversed on Chrome pages but not CoreText's;
  Arabic joining rules (a backward-only letter cannot start a ligature) tell the two apart.

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
