# Progress

Mirrors the milestones in `docs/SPEC.md` §12. Update at the end of every session.
Last updated: 2026-11-23 (M4 model download manager, branch `pomidor/m4-model-downloads-ef1050`, merged with `main` after the viewer and shell side track).

Legend: `[x]` done and verified · `[~]` done but with a stated gap · `[ ]` not started.

## Needs the project owner

Nothing here blocks Milestone 1, but each is a decision or an action only you can take.

1. **Move the repo out of OneDrive?** The checkout is in a OneDrive folder. `scripts/dev-setup.sh`
   keeps Cargo's `target/` and pnpm's package store out of it, but `.git` is still synced, and
   sync clients are known to corrupt git repositories under concurrent writes. A plain folder
   (e.g. `~/Developer/safha`) with a git remote for backup is safer.
2. **The code is on GitHub** as the public repository `adrianazizx/Mirsam` (public since
   2026-11-22). It has no licence file, so the code is all rights reserved until you choose one
   (spec D2).
   - `main` holds Milestones 0 and 1 (PR #1) and the Milestone 3 groundwork (PR #4);
   - `m2-organize` holds Milestone 2 so far, open as draft PR #2 with Auto-fix on.
3. **CI on PR #2** was green on every check (tests and shell self-test on macOS, Linux and
   Windows, and licences) before the markup work and the merge of `main` were pushed.
4. **Licence allowlist additions.** Beyond the spec's list, `deny.toml` allows `Unicode-3.0`,
   `CC0-1.0`, `0BSD`, and `MIT-0`; and the app now compiles in **SQLite** (public domain, SPDX
   `blessing`) for the local state store. All are permissive and all arrive through Tauri's own
   dependency tree (Unicode data, ICU4X), so they cannot be avoided on this stack — but the spec
   says to ask. **`MIT-0`: approved by the owner (2026-11-22)**, with the llama.cpp runtime
   (item 17).
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

    **Measured at M3's exit (session 16, `docs/demos/m3.md`).** For text this app writes:
    - **PDFKit (Preview):** copies Arabic, including ligatures and mixed sentences, exactly.
    - **PDFium (Chrome) and Poppler:** reverse ligatures' letters ("الموظفني"), and PDFium
      reverses Latin runs inside Arabic. Both do worse on Chrome's own files.
    - **`/ActualText` on ligatures:** fixed some of PDFium's words and reversed others; Poppler
      ignored it.

    The decision is still yours: which viewer's convention to follow, or whether to accept
    this.
11. **Arabic UI strings** were written by the assistant, not a native-speaker reviewer (the spec
    schedules that review for M7; earlier is better).
12. ~~Does the 500 MB memory budget include WebKit?~~ **Decided (session 6): the app's own
    process only.** It measures 339 MB in the release benchmark. The webview's processes
    (387 MB more) are measured and printed but not budgeted. The spec's §11 and the benchmark
    checker are updated.
13. **Try Print once (Cmd/Ctrl+P).** On macOS and Windows, printing goes through the platform's
    own PDF viewer (ADR 0012): a window shows the PDF and the system print panel opens over it.
    No session so far could see the screen, so it is unverified. If you see a blank window, or
    the panel prints only part of the document, say so. The fallback is the Linux path below,
    which prints pages as images.

    Linux uses GTK's print dialog, with each page drawn at the printer's resolution (ADR 0013).
    CI checks it by printing to a file; it has not been tried on a desktop with a real printer.
14. **IBM Plex in users' PDFs.** Text boxes embed subsets of IBM Plex Sans and IBM Plex Sans
    Arabic (ADR 0019). "Plex" is a Reserved Font Name under the OFL. Embedding a subset in a
    document under the font's own name is common practice and is not redistributing a modified
    font, but `assets/fonts/README.md` asked for this to be confirmed. If in doubt, Noto Sans
    and Noto Sans Arabic (OFL, no reserved name) can replace them.

15. **A JPEG codec for redaction (M5, ADR 0042).** Redacting an area of a *scan* (a page that is one
    JPEG) currently drops the whole image, because overwriting its pixels needs a JPEG decoder and
    encoder. Candidates: `zune-jpeg` (MIT/Apache/Zlib, but 0.5.16 is a release candidate) and
    `jpeg-encoder` (`(MIT OR Apache-2.0) AND IJG`: the IJG licence is permissive but not on the spec's
    list). Which may we add? Until then the summary says how many images were removed whole.
    **Update (M6-a):** the *encoder* half is solved without a dependency: `safha_core::jpeg` is our
    own baseline encoder (ADR 0050), checked against PDFium's decoder. Only a **decoder** is still
    needed — for redacting scans and for downsampling JPEG images when compressing (scans are
    mostly JPEG, so it is compression's biggest gap). `zune-jpeg` (decoder only) is the candidate.
16. **M3's exit, by hand** (`docs/demos/m3.md`):
    - open an edited file in **Acrobat Reader** (spec §7's first acceptance test names it; it
      cannot be driven here);
    - type into the paragraph editor with real input methods: macOS and Windows Arabic
      keyboards, press-and-hold, the emoji picker, dictation.

    Also, when this Mac is idle, re-run the scroll and zoom benchmark. At M3's exit it ran under
    a load average of 85 (another session's OCR benchmark and builds), and its numbers are not
    evidence.

17. ~~May the llama.cpp runtime's builds ship?~~ **Decided (2026-11-22): yes** (ADR 0034).
    - **What was approved:** llama.cpp's own `llama-server` builds (release `b12359`): Metal,
      CPU, and Vulkan with a CPU fallback. Outside spec §10's list, they compile in sheredom
      `subprocess.h` (Unlicense) and miniaudio (Unlicense OR MIT-0), and carry an unaudited web
      UI.
    - **Conditions:** the server always runs with `--no-webui`, and no CUDA build ships (NVIDIA's
      CUDA runtime licence).
    - **Now:** the runtime is in the download catalog.

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

## M1 — Viewer (P0 viewing) — done; merged from `m1-viewer` with the open items below

- [x] Render scheduler: one render thread, priority queue, cancellation by viewport generation,
      64 MiB byte-capped LRU keyed by (document, revision, page, scale, rect), previews then tiles
      (ADR 0009)
- [x] Continuous scroll, zoom (pinch, Ctrl+scroll), HiDPI — measured with a visible window
      (ADR 0011). Release build, 1,000 pages:
      - scrolling at 4,000 and 12,000 px/s and flinging drops no frames;
      - 0.2% of all frames are dropped;
      - pinch-in is clean;
      - no page is blank longer than 12 ms; random jumps show a preview within 16 ms.
      - **Extreme pinch-out (×1/20 in 1.2 s) is within budget since 2026-11-23:** p95 18–19 ms,
        0–2 dropped frames, against 27–34 ms and 5–8 before, measured under the same heavy
        machine load. Zoom gestures are drawn into one canvas and laid out once, at the end
        (ADR 0011 amendment).
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
- [x] Exit test, part 1 (the 1,000-page bilingual PDF at 60 fps): passes, extreme pinch-out
      included since 2026-11-23 (3 of 3 benchmark runs). Memory is within budget: the app's
      process, by the owner's decision.
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

M1 closed in session 6 at the owner's go-ahead, with these items carried forward:

- confirm printing on screen on macOS and Windows (owner item 13);
- ~~pinch-out frames~~ done 2026-11-23 (ADR 0011 amendment).

## M2 — Annotate and organize — done; merged from `m2-organize` (PR #2) with the open items below

Merged into `main` in session 8 at the owner's go-ahead. Demo script: `docs/demos/m2.md`.

Plan, in order: every later slice edits documents, so the editing model and saving come first.

1. ~~**Editing model**~~ **done (ADR 0014):** each edit is an incremental update written by our
   own writer, and undo pops the last one. PDFium's safe API cannot reorder pages without
   breaking the outline and links, and cannot save incrementally. Validated on the corpus with
   PDFium and `qpdf --check`.
2. ~~**Page organizer**~~ **done:** the Pages tool shows a page grid with multi-select, drag to
   reorder, and move earlier/later (keyboard and palette). Rotate, delete, duplicate and insert
   act on the selection, or on the current page in the page view, with undo and redo.
3. ~~**Safe save**~~ **done (ADR 0015):** Save and Save As (temp file → validate → atomic
   rename). The original is never written in place; signed files are saved incrementally. The
   app asks before a modified tab, window, or the app closes.
4. ~~**Merge, insert from another PDF, extract, split**~~ **done (ADR 0016):** PDFium's page
   import; snapshot steps in the history; refuses signed targets. Session 8 added split by
   top-level bookmarks and by file size, and inserting from password-protected files (the
   page area asks for each file's password).
5. ~~**Markup**~~ **done (ADR 0017):** highlight, underline, strikethrough and squiggly from the
   text selection, as standard annotations with appearance streams. Click a markup to select
   it; Delete removes it. New objects in encrypted files are encrypted (RC4, AES-128, AES-256).
   The view paints again instead of rebuilding. Colour choice comes with the inspector.
6. **Notes, text boxes, ink** (RTL-aware text entry):
   - ~~**Ink and sticky notes**~~ **done (ADR 0018):** the Comment tool's Note, Draw (with
     pressure), and Erase-ink modes; a note editor that follows the text's own direction.
   - ~~**Text boxes**~~ **done (ADR 0019):** shaped by M3's `typeset`, drawn with embedded
     IBM Plex subsets by the new `pdf::text` writer. They stand upright on turned pages, and
     their height fits the text.
7. ~~**Autosave and recovery**~~ **done (ADR 0040):** a modified document's current bytes go
   to a recovery file 2 seconds after its last change. At the next start the app offers to
   restore or discard them.
8. ~~**Comments panel**~~ **done (ADR 0041):** filter, sort, reply, resolve, and go to each
   comment. Replies and review states are standard `/IRT` threads, and new annotations carry
   the name the user chose.

- [~] Markup, notes, text boxes, ink:
      - Markup is done (ADR 0017): highlight, underline, strikethrough, squiggly; select and
        delete; undo; English and Arabic; encrypted files.
      - Ink and sticky notes are done (ADR 0018): pressure, eraser, notes upright on turned
        pages, Arabic note text, edit and delete.
      - Text boxes are done (ADR 0019): Arabic, English and mixed text shaped with embedded
        font subsets; they read back exactly once flattened; encrypted files work.
      - Not yet: callouts, choosing fonts, sizes and colours, and moving annotations.
- [~] Comments panel (ADR 0041):
      - lists markup, notes, text boxes and drawings with author, date, the marked text,
        replies, and whether resolved; threads written by other viewers are followed;
      - filter by status, type and author; sort by page, latest activity, or author;
      - reply and resolve (or reopen), as standard annotations that draw nothing; undo covers
        them; removing a comment removes its thread;
      - "Your name on comments" signs new annotations of every kind; no name is guessed.
      - Not yet: shapes, stamps and other kinds from other tools; editing or deleting a single
        reply; XFDF import/export, summaries, flattening (P1).
- [~] Page organizer (reorder/rotate/delete/merge/split/extract):
      - The Pages tool shows a virtualized grid (React Aria `GridList` + `GridLayout`).
      - Select pages with click, Shift and Cmd/Ctrl; drag to reorder; move earlier or later.
      - Rotate, delete, duplicate and insert act on the selection.
      - Both directions: the grid flows right to left in Arabic.
      - Insert pages from files (also merges), including password-protected ones; extract to a
        new file; split by page count, at top-level bookmarks (files named after them), or by
        file size (ADR 0016).
      - Pointer dragging is untested: browser automation cannot start a native drag. The drop
        arithmetic is unit-tested, and the move commands are the keyboard path.
- [x] Undo journal: undo and redo for page edits and annotations (`EditHistory`, ADRs 0014
      and 0017); each step knows whether it changed pages or only annotations
- [x] Safe save pipeline (ADR 0015):
      - temp file → reopen to validate → atomic rename;
      - a full PDFium rewrite drops deleted pages' content;
      - signed files are saved incrementally;
      - `qpdf --check` runs in tests (qpdf is not bundled);
      - unsaved changes are marked on the tab, and the app asks before closing them.
- [x] Autosave and recovery (ADR 0040):
      - each modified document's current revision is written to `<state dir>/recovery/` 2 s
        after its last change, atomically;
      - removed on save, on undo back to the saved revision, and on closing or quitting
        without saving;
      - at startup a dialog offers each one: restore (undo goes back to the original when it
        is unchanged; Save asks where when it changed) or discard, or decide later;
      - encrypted documents stay encrypted, and restoring asks for the password.
      - One copy runs per state directory (ADR 0046, 2026-11-23). A second launch hands its
        documents to the running copy and exits, so recovery files and the store have one
        writer.
- [x] Budgets not regressed (spec §11; measurements below)

### M2 measurements (Apple-silicon MacBook, macOS 26, session 8)

M2 adds no budgets of its own; the check is that M1's did not regress.

- **Search:** 1,000 pages index in 1.11 s once (M1: 1.09 s), then 17–18 ms per search (M1:
  18 ms). Release build, `search_bench`.
- **Scroll and zoom:** the debug-build benchmark ran with a visible window, on `m2-organize`
  and on `main` (M1, without M2) one after the other:

| Debug build, 1,000 pages | `main` | `m2-organize` |
|---|---|---|
| Scrolling 4,000 / 12,000 px/s, fling: median, p95 frame | 17 ms, 17–18 ms | 17 ms, 18 ms |
| Longest blank page; random jump to preview (p95) | 0 ms; 19 ms | 0 ms; 19 ms |
| Pinch in: p95 frame, frames > 20 ms | 25 ms, 4 | 23 ms, 4 |
| Pinch out: p95 frame, frames > 20 ms | 36 ms, 12 | 37 ms, 12 |
| The app's own memory at the end | 334 MB | 344 MB |

  The same on both. Pinch is over budget in debug builds on both; the release figures
  (pinch-in clean, extreme pinch-out p95 25 ms) are M1's open item.
- **First page on screen:** 31–45 ms in the self-test (debug, dev server).
- **M2's own interactions** (not budgeted): an edit is painted in 20–40 ms; a markup is
  added in 15–20 ms.

M2 closed in session 8 at the owner's go-ahead, with these items carried forward:

- **Annotations:** callouts (P0), choosing fonts, sizes and colours, and moving annotations.
  These come with the inspector.
- **Comments:** shapes, stamps and other kinds from other tools are not listed; a single reply
  cannot be edited or deleted from the panel; XFDF import and export, summaries and
  flattening (P1).
- **Organizer:** pointer dragging is untested by automation.
- ~~**Autosave:** a single running copy is not enforced.~~ Done 2026-11-23 (ADR 0046).
- **From M1:** printing on screen (owner item 13). ~~Extreme pinch-out~~ done 2026-11-23.

## M3 — Text engine — done; closed on `m3-text` (PR #8) with the open items below

Closed in session 16 at the owner's go-ahead, with the exit test's gaps stated rather than
waited on. Demo script: `docs/demos/m3.md`.

The groundwork that did not depend on M2 was built on branch `pomidor/m3-text-engine-3c22b6`
in parallel with M2 and merged by pull request at the owner's request. M3 proper started in
session 9.

Plan, in order:

1. ~~**Writing text into page content**~~ **done (ADR 0021):** its own content stream under a
   placing `cm`, the page's original content wrapped once in `q … Q`, resources made the page's
   own; encrypted files; undo.
2. ~~**Add-new-text tool**~~ **done (ADR 0021):** the Edit tool places text; a strip sets font
   (Sans, Naskh), size, colour, alignment and direction.
3. ~~**[SPIKE] Editor surface**~~ **done (ADR 0022, `spikes/editor-surface/`):** the webview's
   own layout matches the engine's line breaks 5 of 5 and its character positions to 0.01 pt
   (1.1 pt on mixed bidi). **Decision: a native text field, not our own caret.** Not measured:
   real IMEs, WKWebView.
4. ~~**Page analysis → edit model**~~ **done (ADR 0023):** own content-stream interpreter and
   font reader (`pdf::content`, `pdf::fontinfo`), glyphs → lines → paragraphs in the page as
   displayed (`blocks`), with logical order, direction, alignment, line pitch, size, colour.
5. ~~**In-place editing with reflow**~~ **done (ADR 0023, `block_edit`):** removal by byte
   range, new text at the old text's place (z-order, clip), own font reused for Latin text it
   has codes for, otherwise the whole paragraph in a bundled family with a "Font substituted"
   note and a Sans/Naskh choice; unused fonts of our own drop out. The Edit tool opens a
   paragraph when pressed.
6. ~~**Exit test**~~ **done with stated gaps (session 16):** spec §7's four acceptance tests.

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
- [~] **Own content-stream writer** (now the only backend). The text half exists:
      `pdf::text` (ADR 0019) turns a layout into operators and embeds its font subsets, used by
      M2's text boxes and, since session 9, written into page content (ADR 0021). Removing a
      block's existing text objects is done (`block_edit`, by byte range, ADR 0023). Must solve:
      - ToUnicode for multi-glyph clusters (Noto Naskh's shadda + fatha extracts twice with
        `/ActualText`);
      - encrypting new objects in encrypted files (done in M2: `pdf::crypt`, ADR 0017).
- [x] Add-new-text tool (ADR 0021):
      - the Edit tool: a press places the text, the shared editor takes it;
      - Sans (IBM Plex Sans + Plex Sans Arabic) or Naskh (Amiri), 8–72 pt, four colours,
        start/centre/end/justify, direction auto/RTL/LTR;
      - written into the page's content: prints, extracts and searches as page text everywhere;
      - turned pages, unbalanced page content, inherited resources, encrypted files; undo; save.
      - Gap: not editable once added (in-place editing, step 5); each addition embeds its own
        subsets (font reuse comes with step 5); installed fonts come with the font picker.
- [x] In-place editing with reflow (ADR 0022, ADR 0023):
      - the Edit tool: a press on the page's own text opens that paragraph in the shared editor;
        a press elsewhere places new text; text that cannot be read exactly says so;
      - removal by byte range, new text where the old was; the clip and the z-order keep;
      - own font for Latin text it has codes for (nothing embedded), else a bundled family for
        the whole paragraph, with the note and a Sans/Naskh choice;
      - turned pages, `TJ`/`'`/`"` text, pages that leave their state changed, edit again
        after save and reopen (no drift, no extra fonts); English and Arabic.
      - Gaps (ADR 0023 "Honest limits"): bold/italic, indents and tabs, mixed styles flatten,
        no kerning in reused fonts, forms/encrypted/Type 3, installed-font substitutes, the
        editor field does not yet sit over the paragraph in its own width and size.
      - Session 11 (ADR 0023 amendment):
        - a press on text that cannot be edited says why, in English and Arabic: shared, needs
          OCR, an unreadable font (Type 3), rotated, encrypted, or unsupported;
        - such text is listed as refused areas, so no new text lands on top of it;
        - the automatic substitute is the closest of five bundled faces (Noto Naskh Arabic and
          Noto Sans Arabic added), by name, serif flag, and the font's own `/Widths`.
      - Session 12 (ADR 0024): **styled runs.**
        - Bold, italic, sizes and colours inside a paragraph are kept through edits.
        - Typed text takes the style before it; typing over a word keeps its style.
        - Each run is written in its own font when it can be, else in a bundled face at its
          weight (bold faces added), slanted for italic.
        - Still to do: weights other than regular and bold.
      - Session 13 (ADR 0025): **bold and italic while editing.**
        - The paragraph field is the browser's own rich-text editing, held to bold and italic.
        - `text.bold` and `text.italic` (Cmd/Ctrl+B and I, and B and I buttons) run its own
          commands, so its undo covers them.
        - The edit carries marks, served by the page's own bold and italic fonts of the same
          family, or by bundled faces.
        - Not yet settable from the editor: underline, size, colour.
      - Session 14 (ADR 0026): **the PDF's own embedded fonts.**
        - An edited paragraph is set in its fonts' embedded TrueType or OpenType programs
          when they have the glyphs, between reusing codes and bundled faces.
        - Arabic from Chrome's subsets (no `GSUB`) works through presentation forms.
        - `fsType` is respected.
        - Still to do: Type 1 and bare CFF programs.
      - Session 15 (ADR 0023 amendment): **tashkeel.**
        - Chrome's vocalised Arabic and this app's own read in place exactly: no stray spaces,
          letters in order, marks on their letters.
        - A vocalised paragraph's marks go with it when it is edited or removed.
- [~] **Exit test: all acceptance tests in spec §7** (session 16, `tests/m3_exit.rs`,
      `docs/demos/m3.md`).
      - **All four pass:** typo, Arabic insert, mixed sentence, round trip. Checked against
        PDFium's, Poppler's and PDFKit's rendering and text, where installed.
      - **Gaps:**
        - Acrobat Reader is a check by hand;
        - copying Arabic ligatures and mixed lines is exact in PDFKit and in this app, but not
          in PDFium (Chrome) or Poppler (owner item 10, measured);
        - the scroll benchmark is to be re-run on an idle machine (owner item 16).
      - **Typing** in the paragraph field takes 17–18 ms a keystroke (median), within the 32 ms
        budget.

M3 closed in session 16 at the owner's go-ahead, with these items carried forward:

- **The exit test by hand** (owner item 16): Acrobat Reader, real input methods, and the scroll
  and zoom benchmark on an idle machine.
- **Copying our Arabic in Chrome and Poppler** (owner item 10): they reverse ligatures' letters
  and print mixed lines in visual order, in Chrome's own files too.
- **Editing:**
  - underline, sizes and colours are not settable from the editor;
  - only regular and bold weights;
  - indents, lists and tab stops are not kept;
  - reused fonts lose kerning;
  - a paragraph that grows can overlap the text below it;
  - direction is decided by the majority of letters.
- **Fonts:** Type 1 and bare CFF programs are not re-set; installed fonts are not offered as
  substitutes (comes with the font picker).
- **Not edited:** text in forms, encrypted files, Type 3 and CMap-encoded fonts. Each is refused
  with its reason.

## M4 — OCR (quality-first)

- [~] Engine benchmark and ADR 0030 (proposed; side track on branch `pomidor/m4-ocr-benchmark-f94419`):
  - **Primary recogniser: Nuqta-Arabic-OCR 4B.** 0.89% CER over the set, no reading-order errors.
  - **Ensemble partner: Nuqta 0.8B.** It re-reads low-confidence lines and every number, date
    and ID, with the platform's OCR as a third reader for fields.
  - **Runtime: llama.cpp with the authors' Q8_0 GGUF builds** (MIT runtime, Apache-2.0 weights).
    The 4B at Q8_0 writes the same text as bf16 on all 47 pages. Q4_K_M loses accuracy on
    degraded mixed lines and digits, so it is not shipped.
  - **Without a GPU: the 4B on the CPU** (owner's decision, 2026-11-22), at 143 s per page
    (11 GiB). The 0.8B (46 s per page, 3 GiB) is only for machines whose memory cannot hold the
    4B. That memory threshold is still to be set from a timing on a typical Windows laptop.
  - **Quick draft:** Apple Vision on macOS, Tesseract elsewhere.
  - **Not chosen:** QARI-OCR (invents words), PaddleOCR-VL-1.6 (weak Arabic), PP-OCRv5 Arabic
    (32% CER; drops Latin words in Arabic lines), Baseer (no weights released).
  - **Beats:** plain Tesseract and Apple Vision in every Arabic category.
  - **Gaps:**
    - Synthetic set only.
    - Acrobat and ABBYY not measured.
    - CPU speed measured on an M3 Max only, not on a typical Windows laptop.
  - **Gates:** the 4B alone fails two, mixed Arabic/English (2.07% CER against 2%) and numbers
    (93.6% against 99%). Through the reconcile pipeline (ADR 0033) **all five pass**: mixed
    1.52%, numbers 99.4%. Numbers pass by one field.
- [x] `ocr-eval` set v2 (47 samples):
  - Severe scans (a new tracked category) and an Arabic form.
  - Table-cell scoring.
  - Either column first on bilingual side-by-side pages.
  - Baselines for nine engine runs in `ocr-eval/baselines/*.set-v2.json`.
- [x] Re-measure Nuqta through a shippable runtime: llama.cpp 1.0.0, Q8_0 and Q4_K_M, Metal and
      CPU only (ADR 0030 follow-up; baselines `ocr-eval/baselines/nuqta-*-llamacpp.set-v2.json`)
- [~] Word boxes for VLM text, spiked (ADR 0031, proposed):
  - **Method:** PP-OCRv5 server detector lines, its Arabic recogniser as an alignment guide,
    words split by ink gaps, UAX #9 word order.
  - **Results:** 99.0% of words on the right line, 93% with horizontal IoU ≥ 0.5 (mean 0.92)
    over 26 pages.
  - **Gaps:**
    - Vocalized Amiri is weak (IoU-x 0.21–0.70).
    - No truth yet for skewed or warped pages.
    - The detector costs 4–10 s per page on PaddlePaddle's CPU backend (ONNX Runtime not tried).
- [~] Quality pipeline: assess, preprocess, layout analysis, recognition, re-recognition/ensemble,
      verification with no silent invention. In slices:
  - [x] **1. Recognition from Rust** (ADR 0032): `safha_core::ocr`.
    - The `OcrEngine` trait, and `NuqtaEngine` over `llama-server` run as a child process.
    - A Pillow-exact resize, and word confidence from token probabilities.
    - Examples `ocr_eval` and `ocr_page`.
    - **Result:** over the eval set, byte-identical text to the benchmark on all 47 pages (0.89%
      CER), at 8.3 s per page on Metal and 141 s on the CPU.
  - [ ] **2. Word boxes in Rust:** the PP-OCRv5 detector through ONNX Runtime, and ADR 0031's
        alignment.
  - [x] **3. Reconcile and verify** (ADR 0033): `QualityReader`.
    - **Assessment:** skew and specks.
    - **Deskewed line strips,** found without a detector.
    - **Order fix:** number–unit compounds ("bit-64" → "64-bit") in Arabic lines.
    - **Re-reads:** every number and every word under 0.95 is re-read: by the 0.8B (page and
      strips), by the 4B (the strips holding targets), and optionally by the platform OCR for
      numbers.
    - **Reconcile:** a word is corrected only when the re-reads agree; otherwise it is flagged.
    - **Results:** 7 corrections, none wrong. CER 0.60%. All five gates pass.
    - **Cost:** 24.9 s per page on Metal.
    - **Still open:** omitted lines ("Lease Agreement") wait for slice 2's detector. CPU-only
      cost is not measured.
  - [ ] **4. Assembly into the text layer** in logical order (spec §6.3).
- [ ] Background jobs with draft-then-quality results; survives restart
- [~] Word-level confidence, pages-needing-review list. The engine side is done (ADR 0033):
      each word has a probability and a status, and `review_list` gives the pages needing
      review. The UI is not built yet.
- [ ] Searchable-PDF output in logical order
- [~] Model download manager; hardware acceleration (ADR 0034, branch `pomidor/m4-model-downloads-ef1050`):
  - **Engine:** `safha_core::models`. A pinned catalog (Nuqta 4B and 0.8B GGUF by revision, size
    and SHA-256). Resumable HTTPS downloads with retries, checksum, a disk-space check, cancel,
    atomic install into `<app data>/ocr-models`, delete, and a status query. tar.gz and zip
    runtime archives are unpacked safely.
  - **Hardware:** Metal, CUDA, Vulkan and memory are detected and reported.
  - **UI:** Settings (Cmd/Ctrl+,) lists the models with size, licence, state and progress. The
    OCR tool's inspector panel shows the model's state. The first OCR use without the model asks
    to download it and explains its size.
  - **Runtime:** the llama.cpp runtime (`b12359`; Metal, CPU, and Vulkan with CPU fallback; no
    CUDA) is a required package, with the owner's licence approval (item 17).
  - **Gaps:**
    - Acceleration is reported, not yet used by recognition.
    - Windows memory detection and the Windows/Linux archive paths are untested on those OSes
      (CI builds them).
    - Signing a downloaded `llama-server` is open (M7).
- [ ] Benchmark gate in CI
- [ ] Exit test: targets and ship gates in spec §8 met on `ocr-eval/`

## M5 — Forms, sign, protect

- [~] Form filling with shaped appearance streams (session M5-c, ADR 0045):
  - **Done:** text (single-line, multi-line, comb, password), check boxes, radio buttons, drop-downs,
    and lists. Each widget gets our own appearance, with Arabic shaped and starting at the right.
    `/V` is written, and `NeedAppearances` is turned off after every valued field is drawn.
  - **Tested** in PDFium renders, after a save through PDFium, flattened by qpdf, and in an
    encrypted file.
  - **Gaps:**
    - JavaScript format, calculate, and validate actions are not run; a date field is text.
    - Fields are filled in the inspector, not on the page.
    - Fields in encrypted object streams are not listed.
- [~] Fill & Sign on flat forms (ADR 0045): text, today's date, tick, cross, and dot, placed by a
  click, as our own `/Stamp`s, so they are not comments. Upright on turned pages.
  - Moved by dragging (a dashed outline shows where it lands) or with the arrow keys (1 pt, Shift
    10 pt). Resized by dragging a corner handle (in proportion, about the opposite corner) or in
    25% steps; text keeps its size for later changes.
  - Text, dates and marks in black, blue, red or green, chosen in the Sign panel; choosing one also
    recolours the selected text or mark.
  - Moved to another page by dragging it there (the view scrolls on near the edge) or with
    "previous page" and "next page"; upright on pages turned otherwise.
  - **Gaps:** fixed starting sizes.
- [~] Drawn/typed/image signatures and initials (ADR 0045):
  - **Making them:** a pad with pressure; typed in Naskh (Amiri) or Sans; or a PNG (transparency
    kept as a soft mask) or JPEG, picked through the native dialog.
  - **Kept** in the state store (schema 4), on this device only. **Placed** in proportion and
    resizable.
  - **Gap:** the image import and the image preview URL are untested in the real shell.
- [x] Encryption and permissions (session M5-b, ADR 0043): set, change or remove passwords on save
      (AES-256 revision 6 by default, AES-128 for old readers), owner and user passwords, permission
      flags honoured by the command registry; Arabic passwords tested; `qpdf --check` and reopen pass.
- [~] True redaction with pattern search (session M5-a, ADR 0042): text, image pixels, paths,
      annotations and metadata mentions removed from the file and checked by extraction and render;
      e-mail/phone/IBAN/ID/card presets and regex, Arabic-Indic digits normalised; mark by drag,
      selection, or search; review, undo before apply, apply. **Gaps:** JPEG and other compressed
      images are dropped whole (owner item 15); encrypted files are refused; vertical writing and
      custom CMaps are only covered by the extraction check; the signed-file path and the drag in the
      native shell are untested.
- [x] Sanitize: metadata, scripts, attachments, comments, hidden text and layers; the file is
      rewritten in full so unlinked objects are gone (ADR 0042)
- [ ] Form creation
- [~] Certificate signatures (PAdES) — **[SPIKE] done (ADR 0044, `spikes/pades`)**: RustCrypto stack
      recommended; a prototype signs an incremental update that Poppler/NSS and OpenSSL verify. The
      feature itself (`safha_core::sign`, key import, signing UI, revocation) is not built.

## M6 — Edit everything else + convert

- [ ] Images, links, object manipulation
- [~] Headers/footers, page numbers (Western or Arabic-Indic digits, start on the right in RTL),
      watermarks (text as outlines, PNG/JPEG images; opacity, rotation, behind or over; page
      ranges) — ADR 0050. Gap: text that runs vertically in page space (an upright footer on a
      page turned 90°) extracts letter by letter (extractor work); no "remove stamp" yet
- [ ] Find and replace
- [~] Compression (unused objects, object streams, Flate, images reduced by drawn resolution, size
      preview before saving a copy), image export (PNG, own baseline JPEG encoder, 36–600 dpi),
      text/Markdown export (logical order, RTL blocks marked) — ADR 0050. Gaps: JPEG images are not
      downsampled (needs a decoder, owner item 15); no font re-subsetting or duplicate merging;
      no progress bar or cancel for long exports
- [ ] OCR review mode, region re-OCR, replacing poor text layers, scan-to-editable-text

## M7 — v1.0 polish

- [ ] Accessibility pass
- [ ] Native-speaker Arabic localization review
- [ ] Onboarding, auto-update, installers and code signing
- [ ] Opt-in crash reporting
- [ ] Performance hardening

## Session log

### M4 side track — 2026-11-22 (OCR model download manager; branch `pomidor/m4-model-downloads-ef1050`)

- **ADR 0034.** 0033 is taken by the reconcile work on `pomidor/m4-reconcile`.
- **Engine (`safha_core::models`):**
  - **`Catalog`:** the Nuqta 4B (5.5 GB) and 0.8B (1.1 GB) Q8_0 GGUF files and their vision
    encoders, pinned to the revisions ADR 0030 measured. `runtime_package` defines llama.cpp
    `b12359` for six platforms: Metal on Apple silicon, CPU on Intel Macs, Vulkan with CPU
    fallback on Windows and Linux; no CUDA, because of NVIDIA's runtime licence. The owner
    approved its licences (item 17; MIT-0 settles that part of item 4), on condition of
    `--no-webui` and no CUDA. It is in the catalog as a required package.
  - **`ModelStore`:**
    - Status: not installed, partial, installed, outdated.
    - Install: resumable `.part` files named by SHA-256, the whole file hashed, a disk-space
      check, cancel, staging, then a receipt and a rename into place.
    - Delete and clean.
  - **`Downloader`:** ureq with rustls (ring) and the OS trust store (`rustls-native-certs`).
    Ranges, redirects, retries with backoff, and timeouts.
    - `rustls-platform-verifier` was tried first and dropped: its wasm32-only
      `webpki-root-certs` is CDLA-licensed, and `cargo deny` checks every target.
  - **Archives:** tar.gz through `tar`, zip through our own small reader. Escapes are refused.
  - **`Hardware::detect`:** accelerators and memory.
- **Shell (`models.rs`):**
  - **Commands:** `ocr_models`, `download_ocr_model` (on a thread),
    `cancel_ocr_model_download`, `delete_ocr_model`.
  - **Progress:** `ocr-model-progress` events, throttled to 150 ms.
  - **Folder:** `SAFHA_MODELS_DIR` overrides the models' folder.
- **UI:**
  - **Settings dialog:** `app.settings`, Cmd/Ctrl+,.
  - **OCR models section:** progress bar, Download, Resume, Update, Cancel, Delete with confirm.
  - **OCR tool's inspector panel.**
  - **First-use prompt:** `ocr.recognize` asks to download the model, then opens Settings to
    show the download.
  - **Recognition itself** shows "arrives in a coming update" until the pipeline commands land.
  - **Store and strings:** an `ocrModels` store, 8 commands, en/ar strings, and a mock backend
    whose downloads take a few timer steps.
- **Licences checked:**
  - Nuqta GGUF and the Qwen3.5 base models: Apache-2.0.
  - llama.cpp builds: approved by the owner (item 17).
  - New crates: ureq, rustls, ring, rustls-native-certs, tar, flate2, fs4. All MIT, Apache-2.0
    or ISC; `cargo deny` passes.
- **Tests:**
  - **Rust:** 28 unit tests in `models`, against a local HTTP server and archive fixtures. No
    model is downloaded.
  - **Live check:** an `--ignored` test fetched and resumed a 1.1 MB file through Hugging Face's
    CDN.
  - **Shell:** 2 tests.
  - **UI:** 12 tests (prompt, download, cancel and resume, delete, failures, busy, Arabic RTL,
    inspector).
  - **Browser build:** seen in English (light) and Arabic (dark).
- **Not done / not checked:**
  - A real multi-GB download end to end in the app.
  - Windows and Linux runs.
  - A native-speaker review of the new Arabic strings (owner item 11).
- **Merged with `main` after M5-c and the M4 pipeline** (PRs #12, #15, #16), and again after
  Milestone 3 closed (PR #8), and after M6-a and the M5 fill-resize fix (PRs #14, #20; module
  lists, app-store fields, dialogs, locale keys and the session log), and after the viewer and
  shell side track (PR #22): the models' folder now sits in the state directory this copy holds
  (ADR 0046), so two copies of the app never download into the same folder.
  - **Conflicts were additions on both sides:** dependencies (ureq gains `rustls-no-provider`
    for HTTPS), the `lib.rs` modules, registry arguments and error codes, locale keys, the
    inspector's panels, and the ADR index.


### Viewer and shell side track — 2026-11-23 (pinch-out; one running copy; branch `pomidor/viewer-shell-gaps-84e4f5`)

Two gaps carried since M1 and M2, worked in parallel with M4–M7.

- **Pinch-out within budget (ADR 0011 amendment).**
  - **Cause, found with a per-frame trace** now in the benchmark: every relayout during a
    gesture cost one slow frame, partly because clearing `will-change` rebuilt the page area's
    compositing layer. Far into a zoom-out the compositor was slow even without relayouts.
  - **Variants tried:** 8 cheaper CSS variants, none within budget (table in the ADR).
  - **Fix:** a gesture is drawn into one canvas over the viewport from the previews and tiles the
    pages already have, and the layout happens once, at the end.
  - **Release build, full benchmark, 3 runs each, alternating builds:**

    | | Before | After |
    |---|---|---|
    | Pinch-out p95 | 27–34 ms | 18–19 ms |
    | Pinch-out dropped frames | 5–8 | 0–2 |
    | All phases dropped | 0.7–1.2% | 0.2–0.5% |
    | Benchmark verdict | fails 3 of 3 | passes 3 of 3 |

  - **No regressions:** scrolling, flinging, pinch-in, blank pages (≤ 20 ms), jumps (preview
    p95 ≤ 57 ms) and memory (app ≤ 321 MB) are all within budget.
  - **Trade-off:** the gesture's one relayout is its last frame (41–45 ms after a pinch-out; the
    25 frames after a pinch-in have p95 23–25 ms, too short a phase for the p95 rule).
  - **Conditions:** other sessions' builds kept the load average at 40–80 on 14 cores, so all
    figures are higher than session 4's quiet-machine ones. Compare builds, not sessions.
  - **New benchmark mode:** `SAFHA_SELFTEST_SPIKE=scroll:zoom` runs only the zoom phases, three
    times over: a 30-second loop for work on zooming.
- **One running copy (ADR 0046).**
  - **Mechanism:** an OS file lock on `<state dir>/instance.lock`. A later copy sends its
    documents' absolute paths over loopback TCP with a secret token and exits.
  - **macOS:** "open document" Apple Events (Finder, `open -a`) open in the running copy.
  - **Where documents go:** the window used last, as the active tab. A document that arrives
    while the app is still starting joins the startup documents.
  - **Why not Tauri's single-instance plugin:** it is keyed by bundle identifier, which would
    make parallel self-tests hand PDFs to each other or to the owner's copy. On macOS it is not
    a lock.
  - **Dependencies:** no new crates; `getrandom` and `dirs` were already in the tree.
  - **Tests:** 7 new unit tests in `instance.rs` and 2 in `state.rs`.
  - **End to end:** `scripts/instance-selftest.sh` passes on macOS: the second copy exits 0 in
    under 1 s and its PDF becomes the active tab. `open -a` was checked by hand.
  - **Not tested:** Windows and Linux. CI does not run the new script yet; M7 owns the CI
    files.
  - **Known limit:** `open -n` on macOS loses its file; the running copy only comes forward.

Test status at the end of this track (this Mac):

- **Rust:** 347 workspace tests pass. Clippy is clean with and without `devtools`, `cargo fmt`
  is clean, `cargo deny` is ok, and the licence report is current (no new crates).
- **UI:** 171 tests pass. `tsc` and Biome are clean.
- **End to end, release build:**
  - the shell self-test passes in English (light) and Arabic (dark);
  - `scripts/instance-selftest.sh` passes;
  - the zoom gestures in Arabic (dark): pinch-out p95 18 ms, nothing dropped, no blank page.
- **After merging `main` (session 16):**
  - 454 Rust tests and 201 UI tests pass, and clippy is clean both ways.
  - The full benchmark passes at load 25: pinch-out p95 17 ms, nothing dropped; pinch-in p95
    17 ms; 0.4% of all frames dropped; the app's memory 326 MB.
  - `scripts/instance-selftest.sh` passes.
### Session M6-a — 2026-11-22 (Milestone 6: stamps, exports, compression; parallel to M4/M5)

Branch `pomidor/m6-exports-a29095`, from `main` after M4 runtime; `main` merged in as it moved (M5-b encryption, M4 pipeline slices 1 and 3, M5-c forms, session 16's M3 close).
Stayed out of OCR, forms/signatures and encryption code, as asked.

- **ADR 0050** records the approach. In short: headers, footers, page numbers and watermarks are
  *stamps*, one incremental update each (one undo), drawn by our own writer in a new content stream per
  page (ADR 0021's wrapping; nothing regenerated), placed on the page as displayed, marked as
  pagination artifacts; compression is our own full rewrite, previewed before a copy is saved.
- **Engine (`safha_core`):**
  - `stamp`: `Band` (start/centre/end, `{page}`/`{pages}`, digits, first number, `rtl`), `Watermark`
    (text or image, opacity, rotation, behind); `Document::with_stamp`. A band embeds **one** font set
    for all its pages (1,000-page numbering: 1.0 MB, 0.49 s release; 5.3 MB with a subset per page).
    Text watermarks are drawn as glyph outlines (`pdf::outline`), so they stay out of copy, search and
    export (at 45° as text they read back as "L IA T N E ID F N O C").
  - `jpeg`: our own baseline JPEG encoder (no licence question; see owner item 15).
  - `export`: `Document::export_page_image` (PNG/JPEG, 36–600 dpi), `Document::export_text`
    (plain with form feeds between pages; Markdown with escaping, page markers, and
    `<div dir="rtl">` around blocks whose direction differs from the document's).
  - `compress`: `Document::compressed` (refuses signed and encrypted files; checked by reopening).
    1,000-page fixture 571 KB → 170 KB in 0.24 s; single-page Chrome fixtures 12–26 % smaller.
  - `pdf::text::draw_text` split into `draw_with_subsets` + `fonts_to_embed` (same output).
  - `png` is a normal dependency (OCR page images and image signatures use it too). M5-c's
    `pdf::image` and this branch's watermark image writer overlap: a follow-up can merge them.
  - Merged on 2026-11-23 (branch `pomidor/dreamy-tereshkova-743393`): watermarks write images through `pdf::image`, each caller passing its own `ImageLimits`; JPEG sizes come from `jpeg::frame`, so image signatures now also refuse 12-bit, arithmetic-coded and lossless JPEGs.
  - `examples/m6_report.rs`: stamping and compression timings and sizes for any PDFs.
- **Shell (`output.rs`):** `stamp_pages`, `pick_watermark_image` (the image is read here; the UI gets
  its name), `export_page_images` (one file, or a folder for several, never overwriting),
  `export_text`, `compress_document` (kept in memory with the bytes it came from) and
  `save_compressed` (`compress_stale` once the document changed). Error codes `no_image`,
  `image_unreadable`, `compress_stale`.
- **UI:** six commands open one dialog each (`pages.headerFooter`, `pages.pageNumbers`,
  `pages.watermark`, `file.exportImages`, `file.exportText`, `file.compress`), which run `output.*`
  commands. Page ranges are typed as "1-3, 5, 8-" (Arabic-Indic digits, the Arabic comma and dashes
  read too). The Arabic UI defaults to Arabic-Indic digits, right to left and Naskh. Stamps need the
  `modify` permission and exports `copy` (ADR 0043); compression is off for encrypted files. Strings
  in English and Arabic. Checked in the browser preview in both directions; found and fixed number
  fields snapping 150 dpi to 156 (a step of 12 from a minimum of 36).
- **Tests:** `tests/stamp_export.rs` (10, English and Arabic: footer and header placement on plain
  and turned pages, Arabic-Indic digits and right-side start, undo, translucent rotated and behind
  watermarks, PNG alpha and our JPEG decoded by PDFium, page images, text/Markdown in logical order,
  compression of the whole corpus with page text unchanged and `qpdf --check`, image reduction,
  encrypted refused); unit tests in `jpeg`, `stamp`, `export`, `compress`, `pdf::outline`, `output.rs`;
  UI `pageRanges.test.ts` and `output.test.tsx` (dialogs in English and Arabic, permissions).
  Workspace tests, clippy (with and without `devtools`), Biome, `tsc`, `cargo deny` and the licence
  report pass. The end-to-end shell self-test was not re-run (no shell window code changed).
- **Not done here:** images/links/object editing and find & replace (other M6 items); OCR review
  items (depend on M4).

### Session 16 — 2026-11-22 (Milestone 3's exit test)

- **`tests/m3_exit.rs`:** spec §7's four acceptance tests, each clause checked against every
  viewer installed.
  - PDFium's rendering and text (Chrome's engine) everywhere.
  - Poppler (`pdftoppm`/`pdftotext`) where installed.
  - PDFKit (`sips`, and `tests/support/pdfkit_text.swift`) on macOS.
  - What cannot run says so.
- **Results:** all four pass.
  - **Typo:** no other paragraph moves; three engines draw the saved page identically outside
    the paragraph; all read the fix.
  - **Arabic insert:** a re-set word is drawn as Chrome drew it (width and ink within 5%); the
    paragraph re-wraps right to left; PDFKit copies it exactly.
  - **Mixed sentence:** in order in this app and PDFKit.
  - **Round trip:** English, Arabic and an embedded-font heading, with no drift or duplicate
    fonts.
- **Measured:** PDFium and Poppler reverse ligatures' letters and mixed runs, in Chrome's own
  files too. `/ActualText` on ligatures helped some words and broke others, so it was not
  kept. Owner item 10 is updated.
- **Self-test:** typing in the paragraph field is 17–18 ms a keystroke at the median (spec §11:
  32 ms).
- **Benchmark:** the scroll benchmark ran under a load average of 85 and is to be re-run (owner
  item 16).
- **Demo script:** `docs/demos/m3.md`.
- **Milestone 3 closed** at the owner's go-ahead, with the gaps above carried forward rather than
  the benchmark re-run first. `main` (M5-b: encryption and permissions) was merged in first.

### Session 15 — 2026-11-22 (Milestone 3: tashkeel in paragraphs edited in place)

- **Found:**
  - Chrome gives a cluster's text to a zero-width mark glyph off the line and none to the
    letter. The result was stray spaces, swapped letters, and lost letters.
  - This app's own marks below the line formed lines of their own and attached to the wrong
    letters.
  - **No mark's operator was part of its paragraph:** edits left marks on the page.
- **Fixed in `blocks`:**
  - carriers take their letter's box and baseline;
  - marks find their letter by drawing order (direction-aware) and take its baseline;
  - every mark's operator belongs to its paragraph.
- **Measured:** 33 of 126 fixture paragraphs read exactly, up from 26. Chrome's vocalised
  page and this app's tashkeel (Amiri, Plex Sans Arabic) are exact.
- **Tests:** three unit tests, and two integration tests (vocalised ground truth, and no
  leftover ink after removing a vocalised paragraph, in Chrome's text and this app's).

### Session 14 — 2026-11-22 (Milestone 3: the PDF's own embedded fonts, ADR 0026)

- **What producers embed:** the fixtures' subsets were inspected.
  - Chrome: TrueType with a `cmap` of presentation forms and ligatures, but no `GSUB`/`GPOS`.
  - CoreText: a Mac Roman `cmap` only.
  - harfrust has no Arabic fallback, so it gave `.notdef`.
- **`typeset::presentation_forms_in`:**
  - joined forms from ICU's joining types;
  - lam-alef;
  - Unicode's optional ligatures when the font has them;
  - letters kept where a subset maps the isolated form by the letter.
  - `logical_text` folds forms back for `/ToUnicode` and `/ActualText`.
- **A new font source between page codes and bundled faces:**
  - a paragraph is laid out in its fonts' embedded programs and re-embedded under their names;
  - this needs the glyphs, `GPOS` for marks, and an `fsType` that allows it;
  - `EditedBlock::source` reports it, through the shell to the UI.
- **Plumbing:**
  - `FontFace::has_table` and `embedding_permissions`;
  - `FontInfo` keeps the program's place;
  - `NamedFont` names can be a PDF's own.
- **Tests:**
  - Rust: unit tests for forms, ligatures and permissions; integration tests for Chrome's
    Arabic heading re-set and shortened in its own program, a missing form and tashkeel
    falling back, and Latin still embedding nothing.
  - UI tests unchanged.

### Session 13 — 2026-11-22 (Milestone 3: bold and italic commands in the paragraph editor, ADR 0025)

- **The field:**
  - The paragraph editor's field is a `contenteditable` held to text, line breaks, bold and
    italic.
  - Enter is a line break, pasted text is plain, and drops are refused.
  - `richText` fills it from runs and reads it back (inline styles and WebKit's un-bold
    included).
- **Commands:**
  - `text.bold` and `text.italic` (Cmd/Ctrl+B, Cmd/Ctrl+I) run the browser's own `bold` and
    `italic`, so undo covers them.
  - B and I toggle buttons keep the field's focus and show the selection's state.
- **Engine:**
  - `BlockEdit::marks` sets each character's bold and italic.
  - A run that needs another style takes the page's font of the same family that has it
    (Helvetica-Bold), else the paragraph is set in bundled faces.
  - A style-only change is an edit.
- **Tests:**
  - Rust: own-font marks, bundled bold-italic, family key, UTF-16 marks.
  - UI: rich text, the editor's buttons and marks in English and Arabic, registry gating and
    style-only saves.
  - Real-app self-test: an Arabic word made bold in the field with WebKit's command and read
    back as a bold run (English light; Arabic dark on two fixtures).

### Session 12 — 2026-11-22 (Milestone 3: styled runs in edited paragraphs, ADR 0024)

- **Reading:**
  - Every character of a paragraph keeps its font, size and colour through the
    visual-to-logical reorder.
  - The UI gets the runs (UTF-16 offsets, font, bold, italic, size, colour) and a `styled`
    flag.
  - Lines group into a paragraph when they share any font, not only the dominant one: a mostly
    italic line no longer splits a paragraph.
- **Editing:**
  - The field's plain text is mapped onto the old runs by a character LCS.
  - Kept characters keep their style; typing over a word takes its style; other typing takes
    the style before it (after it at the start).
- **Writing:**
  - When every run's own font has codes for its text: each run in its font, with font and
    colour switching inside the line, and justification at spaces.
  - Otherwise, the closest bundled face per original font at the run's weight:
    - bold faces added (Plex Sans and the Noto VFs at 700, Amiri Bold, Plex Sans Arabic
      SemiBold);
    - per-span fallbacks of the same weight in `typeset`;
    - spans no longer carry a font across a style boundary;
    - italic slanted 12° under an oblique name, so it reads back italic.
- **Tests:**
  - Unit tests for runs and restyling.
  - Four integration tests on a Helvetica / Bold / Oblique page with a red word: runs read;
    own-font edit; substituted edit kept over a second edit; Arabic over a bold word in a
    bold Arabic face.
  - A UI test of the styled note in English and Arabic.

### Session 11 — 2026-11-22 (Milestone 3: main's in-place editor kept; reasons and closer substitutes ported)

- **Two implementations met.**
  - PR #8 (`m3-text`) and PR #9 (merged into `main`) each built M3's editor-surface spike,
    page analysis and in-place editing, with opposite surface decisions: our own canvas, or a
    native text field.
  - At the owner's choice, `m3-text` merged `main` keeping main's tree whole (`4b7926c`); the
    branch's own three commits stay in history only.
  - What main lacked was then ported onto its design.
- **Refusal reasons:**
  - `block_edit::Refusal` replaces English strings: shared, unreadable (needs OCR), font,
    rotated, encrypted, unsupported.
  - Text that is not a paragraph (Type 3 fonts, rotated text, all text of encrypted pages) is
    listed as refused areas after the paragraphs.
  - A press there shows the reason, in English and Arabic, instead of placing new text on top.
    On Chrome's Type 3 fixtures every line is covered.
- **Closer substitutes:**
  - Noto Naskh Arabic and Noto Sans Arabic are bundled.
  - With the Plex and Amiri faces, they are ranked by `match_font`: coverage, name, serif
    flag, and the font's `/Widths` for letters that do not join.
  - The notice names the face, isolated from the sentence's direction.
  - Sans and Naskh stay as the user's override.
- **Tests:**
  - Rust: 306 tests (new unit tests for width matching and area joining, and the refusal test
    extended to encrypted and Type 3 pages).
  - UI: 159 tests (a press on refused text; the substitute notice).
  - Real-app self-test: English (light) and Arabic (dark, two fixtures).

### Session M5-c — 2026-11-22 (Milestone 5: form filling, Fill & Sign, signatures; parallel to M5 crypto and M6)

Branch `pomidor/m5-forms-0522c7`, from `main` after PR #11. Left the OCR code and the
encryption/certificate-signature work (ADRs 0043/0044 on `pomidor/m5-crypto-e11ba5`) alone; only used
`pdf::crypt`'s existing API.

- **ADR 0045** records the approach:
  - Our parser reads and writes the form.
  - Every widget's appearance is ours, shaped by `typeset` and written by `pdf::text`.
  - `/Q 0` starts where the text's own direction starts.
  - `NeedAppearances` is turned off once every valued field is drawn.
  - Fill & Sign items are `/Stamp`s named `Mirsam*`, moved and resized through `/Rect` alone.
  - Signatures are kept in the state store.
- **Engine:**
  - **`safha_core::forms`:** `read`, `form_update`, and `forms/appearance.rs`;
    `Document::form` and `Document::with_form_edits`.
  - **`annotate::fill`:** `FillText`, `Mark`, `SignatureStamp`, and `Signature` (drawn, typed,
    image). `AnnotationEdit` gains `AddFillText`, `AddMark`, `AddSignature` and `SetRect`;
    `SetContents` also takes fill text. `AnnotationKind` gains `FillText`, `Mark` and `Signature`.
  - **`pdf::image`:** PNG decoding and JPEG pass-through as image XObjects with soft masks; the
    annotation writer writes images.
  - **`store`:** a `signatures` table (schema version 4).
  - **Dependencies:** `png` (MIT OR Apache-2.0) moves from dev-dependencies to dependencies, and
    `serde_json` joins them. Both were in the tree already, and `THIRD_PARTY_LICENSES.md` is
    unchanged.
- **Shell (`forms.rs`):**
  - Commands: `form_fields`, `set_form_field`, `add_fill_text`, `add_fill_mark`,
    `place_signature`, `set_annotation_rect`, `signatures`, `save_drawn_signature`,
    `save_typed_signature`, `import_signature_image` (native dialog), and `delete_signature`.
  - `safha://localhost/signature?id=N` serves kept images.
  - A new error code, `store_unavailable`.
- **UI:**
  - The Forms tool's panel and the Sign tool's panel, in the inspector.
  - The new-signature dialog (Draw, Type, Image).
  - The viewer's `place` mode.
  - Smaller and Larger in the selection toolbar.
  - Fill text edited in the note editor.
  - 21 commands, English and Arabic strings, and signature tokens (with an ink-on-paper contrast
    pair).
  - The mock backend has a sample form on the sample document's first page, and keeps signatures.
- **Tests:**
  - **Rust:** `tests/forms.rs` (9) runs on a hand-written AcroForm. It covers every kind of field,
    Arabic and English, comb, `NeedAppearances`, encryption made with qpdf, and a save through
    PDFium.
  - `tests/fill_sign.rs` (4) uses the fixtures: render-in-place, `SetRect`, `SetContents`, turned
    pages, encrypted files, flatten read-back, and save.
  - Unit tests cover `fill`, `pdf::image`, `forms`, `forms/appearance`, and the store.
  - **UI:** `commands/forms.test.tsx` (11, in English and Arabic, including the permission gates).
  - `cargo test --workspace` (378 after the merge), clippy (with and without `devtools`), fmt,
    `pnpm test` (190), `tsc`, Biome, `cargo deny`, and the licence report are all clean.
- **Checked by eye:**
  - Renders of a filled form and of Fill & Sign items (PDFium).
  - The Forms and Sign panels and the signature dialog in the browser preview, against the mock,
    in Arabic (RTL) and English (LTR).
  - Not run in the native shell. The end-to-end self-test does not cover forms.
- **Merged with `main` after M5-b** (encryption, PR #13):
  - **Conflicts** were additions on both sides: registry error codes, store imports, locale keys,
    `pdf` modules, and the ADR index.
  - **Permissions:** the new commands follow M5-b's gates. `form.setValue` needs `fillForms`, and
    the `fill.*` commands need `annotate`. The panels show why when they are off.
  - **Schema:** this branch takes state-store schema version 4; M5-b did not change the schema.
- **Merged with `main` again after the M4 pipeline** (PRs #12 and #15): `ocr` and `forms` both
  added to `lib.rs`, both branches made `png` and `serde_json` normal dependencies, and the session
  logs met at the top of this list.
- **Drag to move Fill & Sign items** (added on request after PR #16 merged; its own PR):
  - **In the viewer:** a press on an item, with the Sign or Select tool, drags it. A dashed outline
    shows where it lands, kept on its page. A press that does not travel selects it.
  - **`fill.move`** applies the drag as one undo step through `SetRect`, and the item stays
    selected.
  - **Arrow keys** nudge the selected item 1 pt, and Shift+arrow 10 pt, only while the page view has
    the focus (8 commands).
  - **After an edit** the viewer reloads the visible pages' annotations, so a just-placed item drags
    at once.
  - **Tests:** a viewer test drags in both modes (including the page edge and a press that stays);
    a command test covers the move, the nudges, and the arrow keys left to text fields. 193 UI tests
    pass.
  - **Checked in the browser preview** against the mock: placing a tick, dragging it at once, and
    Shift+→ three times.
- **Corner handles to resize Fill & Sign items** (added on request; its own PR, after the drag one):
  - **Handles:** a selected item shows four. Dragging one resizes the item about the opposite corner,
    in proportion (`resizedRect`), at least 6 pt, and on its page.
  - **Letting go** goes through `fill.move` (`SetRect`), and the item stays selected.
  - **Grabbing:** a handle reaches 7 px outward but only a quarter of the item inward, so a small
    tick can still be dragged by its middle. Resize cursors show over the handles.
  - **Also fixed:** a doc comment the drag change had left above the wrong method.
  - **Tests:** a viewer test (the handles appear on selection, the corner drag, and the middle still
    moving the item) and `resizedRect` unit tests (proportion, the anchor, the minimum, the page
    edge). 196 UI tests pass.
  - **Checked in the browser preview:** a drawn signature selected, then enlarged by its
    bottom-right handle.
- **Colours for Fill & Sign text and marks** (added on request; its own PR, after the resize one):
  - **Choosing:** the Sign panel offers the Edit tool's four document colours (moved to
    `state/colors.ts`, shared). New text, dates and marks take the chosen one (`fill.color`;
    `add_fill_text` and `add_fill_mark` take a colour).
  - **Recolouring:** with a text or mark selected, choosing a colour recolours it:
    `AnnotationEdit::SetColor`, through the shell's `set_annotation_color`, as one undo step. Its
    appearance is drawn again in the same box, `/C` and the text's `/DA` are updated, and the colour
    survives a later change of text. Signatures refuse it.
  - **Tests:** a Rust test recolours text and a cross in a plain and an encrypted file (red pixels
    in their boxes, the same boxes, the text read back flattened, the colour kept after changing the
    text, a signature refused). A UI test covers new items in the chosen colour, recolouring the
    selection, signatures left alone, and the swatches in English and Arabic. 446 Rust and 207 UI
    tests pass.
- **Fill & Sign items move to another page** (added on request; its own PR, after the colour one):
  - **Dragging:** an item follows the pointer onto the page it is over, held where it was grabbed
    and kept inside that page. The view auto-scrolls near the top or bottom edge, as it does for
    text selection.
  - **Commands:** the panel's "previous page" and "next page" (`fill.toPreviousPage`,
    `fill.toNextPage`) move the selected item to the same place on the neighbouring page, and show
    that page.
  - **Engine:** `AnnotationEdit::MoveToPage` (shell: `move_annotation_to_page`) moves the
    annotation object from one page's `/Annots` to the end of the other's, sets `/P` and `/Rect`,
    and, between pages turned differently, rewrites its appearance's `/Matrix` with the stream data
    untouched.
  - **Also fixed:** merging `main` into #23 broke the watermark dialog's colour list (read-only
    tuples), so the shared palette types its colours as plain `[number, number, number]`. That fix
    was pushed to #23 too.
  - **Tests:** a Rust test moves fill text from the unturned page to the 90° page, then to the 180°
    one. It lands in its box only, upright, nothing is left behind, it survives a save, and a note is
    refused. A viewer test drags across a page boundary; a command test covers moving by drag and by
    previous/next page. 471 Rust and 220 UI tests pass.

### M4 pipeline slice 3 — 2026-11-22 (reconcile and verify; branch `pomidor/m4-reconcile`)

- **The failures, broken down:** the 4B misses 11 of 171 fields.
  - **6 are order:** "64-bit" is drawn "bit-64" in an Arabic line, and the image can't tell
    which.
  - **5 are misread digits** on degraded pages.
  - **Voting doesn't fix them:** a plain vote (0.8B + Apple Vision) reaches only 94.2%.
- **Spike** (`spikes/ocr-engines/reconcile_lines.py`, `reconcile_sim.py`):
  - Line strips read at full resolution by both models fix most digit misreads.
  - The fax and form digits don't settle under enlargement or binarisation.
- **`safha_core::ocr`** (ADR 0033):
  - `layout`: assessment, deskew, line strips.
  - `order`: the unit-word fix.
  - `reconcile`: words with statuses; corrections only when re-reads agree.
  - `pipeline`: `QualityReader`, `PageResult`, `review_list`.
  - `command`: an external program as the third reader. `apple-vision.swift` gets an `--image`
    mode.
  - The `ocr_eval` example gets `--pipeline [--vision] [--reread-all-strips]`.
- **Measured through Rust on set v2** (Metal):
  - **All five gates pass:** mixed 1.52% (0 order errors), numbers 99.4% (170 of 171), CER
    0.60% (was 0.89%), table cells 100%, no regressions.
  - **Corrections:** 7, all fixes.
  - **The one field still wrong** (the 100 dpi fax's ١٢٣٤) is flagged.
  - **Flags:** 36 words uncertain, 31 of them right (1.2% false alarms). 12 of 47 pages need a
    decision.
  - **Speed:** 24.9 s per page, 10 requests per page on average. CPU-only speed not measured.
- **The numbers margin is one field.** The photocopied form's employee number was corrected in
  Rust but only flagged in the spike. ADR 0033 lists what would widen the margin: re-reading the
  number's own crop (needs word boxes), a digit-strong third reader, and real scans.
- **Tests:**
  - `cargo test --workspace` passes: 373 tests, plus 1 ignored (the real-model test). That
    includes 17 new OCR unit tests (35 in all) for layout, order, reconcile, pipeline and
    command.
  - `clippy -D warnings` (with and without `devtools`), `fmt --check` and biome are clean.

### M4 pipeline slice 1 — 2026-11-22 (recognition from Rust; branch `pomidor/m4-pipeline`)

- **`safha_core::ocr`** (ADR 0032, accepted):
  - **`OcrEngine` and `NuqtaEngine`.** `llama-server` runs as a child process: loopback only,
    a per-run API key, stopped on drop, typed error codes.
  - **`OcrImage`.** PNG in and out, and a Lanczos resize that matches Pillow bit for bit, so
    the model sees the benchmark's pixels.
  - **Text and confidence.** Markup removal as in the benchmark, and word confidence from
    llama-server's token probabilities, across UTF-8 characters split between tokens.
- **Measured through Rust** (`examples/ocr_eval.rs`):
  - Nuqta 4B Q8_0 wrote byte-identical text to the benchmark's llama.cpp run on all 47 pages:
    0.89% CER, no regression.
  - 8.3 s per page on Metal, and 141 s on the CPU over 4 pages (same text).
  - `examples/ocr_page.rs` reads rendered PDF pages: English, Arabic, and a rotated page.
- **Finding for slice 3:** letting Nuqta 0.8B and Apple Vision outvote the 4B on digit words only
  lifts fields from 93.6% to 94.2%, and on a fax page it swapped one wrong number for another. So
  reconcile must re-read and flag, not vote.
- **Dependencies:** `ureq` 3.5.2 without TLS, plus `ureq-proto` and `utf8-zero` (all MIT OR
  Apache-2.0). `base64` 0.23 and `png` 0.18 were already in the tree. `cargo deny` and the licence
  report pass.
- **Tests:**
  - `cargo test --workspace` passes: 350 tests, plus 1 ignored (the real-model test,
    `tests/ocr.rs`, which passes with `--ignored` locally).
  - 18 new unit tests.
  - `clippy -D warnings` and `fmt --check` are clean.
  - No UI changes; no command reaches the UI yet.
- **Setup note:** this worktree's `target/` had been created as a real directory inside OneDrive.
  `scripts/dev-setup.sh` now redirects it to the cache.

### M4 side track — 2026-11-22 (Nuqta through llama.cpp; word boxes; branch `pomidor/m4-runtime-8c1840`)

- **Nuqta through llama.cpp** (ADR 0030, new follow-up section):
  - **Runtime and files:** llama.cpp 1.0.0 (MIT) and the authors' own GGUF builds (Apache-2.0)
    at Q8_0 and Q4_K_M, with the cards' prompt and input size.
  - **Accuracy:** at Q8_0 the 4B wrote byte-identical text to the bf16 run on all 47 pages, so
    0.89% CER again. The 0.8B scored 2.01% against 2.02%.
  - **Q4_K_M loses accuracy** on degraded mixed lines (4B: 1.52% → 3.19% CER) and digits, so it
    is not shipped.
  - **The two failing gates are unchanged:** mixed Arabic/English 2.07% against 2%, and fields
    93.6% against 99%.
  - **Speed per page:** Metal 8.4 s (4B Q8_0) and 2.7 s (0.8B Q8_0). CPU only: 143 s (4B
    Q8_0, 11 GiB) and 46 s (0.8B Q8_0), against 677 s for the 0.8B through Transformers. About 96% of
    the CPU time goes to reading the 2,420 image tokens.
  - **Decided by the owner:** the 4B is the default even without a GPU; the 0.8B is only for
    machines too small for the 4B.
  - **Spec updated to match:** the §8 engine table (Nuqta 4B, the 0.8B as second reader,
    PP-OCRv5 for word boxes only, platform OCR or Tesseract as the draft) and the §9
    "OCR runtime" row (llama.cpp, plus ONNX Runtime for the detector).
- **Word boxes spike** (ADR 0031):
  - **Ground truth:** Chrome's DOM word rectangles for the 26 geometry-preserving pages.
  - **Detector:** PP-OCRv5's server detector found every line.
  - **Words to lines:**
    - **Text widths alone** put only 70–75% of words on the right line.
    - **Anchoring** to the PP-OCRv5 recogniser's rough text, then splitting each line at ink
      gaps in UAX #9 word order, gives 99.0% on the right line and 93% with IoU-x ≥ 0.5.
  - **Still weak:** vocalized Amiri text, and a word the VLM wrote differently from the page
    ("bit-64").
- **Harness:** `spikes/ocr-engines/`: `run_llamacpp.py`, `word_boxes_truth.mjs`, `wb_detect.py`,
  `wb_align.py`.
- **Installed:**
  - **llama.cpp** through Homebrew (21 MiB).
  - **The four GGUF builds plus their vision encoders** (about 9.5 GiB), in
    `~/Library/Caches/safha-dev/ocr-bench/hf`.
- **Needs the owner (M4):**
  - ~~Confirm the 4B is the default on CPU-only machines.~~ Confirmed. Still open: the memory
    threshold for the 0.8B, which needs a timing on a typical Windows laptop.
  - **Real scans (item 9).** Box truth for skewed and warped pages too.
- **Tests:** no Rust or UI code changed. The scorer runs in this session used `safha-ocr-eval`
  as it was.

### Session M5-b — 2026-11-22 (Milestone 5: encryption and permissions; PAdES spike)

Branch `pomidor/m5-crypto-e11ba5`, from `main`. Stayed out of OCR, forms/Fill & Sign and exports.

- **Encryption and permissions** (ADR 0043). `pdf::protect::rewrite` takes PDFium's full rewrite,
  decrypts every object with the document's password, unpacks object streams, and writes it again
  encrypted for the new settings (or plain): AES-256 revision 6 (random file key wrapped by user and
  owner passwords, `/Perms`) and AES-128 revision 4. `safha_core::security` has `Permissions` (the `/P`
  flags), `Protection`, `SecurityChange` (keep, remove, set) and `SecurityInfo` (cipher, revision,
  flags, owner or user); `Document::security()` and `save::save_secured`.
- **Unicode passwords:** R6 stores SASLprep-style normalised UTF-8 (NFKC, spaces, mapped-to-nothing),
  read as typed then normalised, and `Engine::open_shared` retries PDFium with the normalised form.
  AES-128 takes ASCII only (`password_not_encodable`). Tested with Arabic passwords (presentation forms
  vs letters) and a decomposed accent.
- **Rules:** changing protection needs the owner password when `/P` restricts anything (`not_permitted`);
  a signed file is refused; undo starts over after the save (new bytes).
- **UI:** commands `security.open|apply|cancel` (palette: "Password and permissions…"), a dialog with
  mode, cipher, passwords, seven permission switches, Save and Save as…; the form lives only in the
  dialog. `PERMISSION_GATES` in the registry disables Print, Copy, page edits, text edits, redaction and
  markup for a file opened with its user password. English and Arabic strings.
- **Tests:** seven integration tests (`tests/security.rs`: qpdf `--check` and `--show-encryption` for
  both ciphers, user and owner passwords, owner-only files, Arabic, unlocking an encrypted fixture,
  object streams, identical render), unit tests for the keys and normalisation, 7 UI tests.
  Protection adds 4–5 % to a file; 21 ms for 1,000 pages (release).
- **PAdES spike** (ADR 0044, `spikes/pades`, `make-test-pki.sh`): RustCrypto `cms` 0.3.0-rc.2 +
  `x509-cert` + `x509-tsp` + `rsa` 0.10 + `p256` sign an incremental update (P-256, RSA-2048, an Arabic
  subject, an RFC 3161 time-stamp). **Poppler `pdfsig`: valid and trusted; OpenSSL `cms -verify -cades` and
  `ts -verify`: OK**; a flipped byte is caught; our engine opens it as signed and a later edit keeps it
  valid. CMS 1.3–2.9 KB, signing 0.2–9 ms. Recommendation: build on this stack; B-B, then B-T, then B-LT.
- **Not done / not checked:** Adobe Acrobat (not installed; ask someone to open the spike's PDFs), the
  signing feature and UI, revocation, key import (`.p12`, keychain, smart cards); public-key security
  handlers, RC4 writing, `/Perms` verification; the dialog was exercised with component tests, not
  clicked through in the running app.
- **For the owner:** two used crates are release candidates (`cms`, `x509-tsp`), pinned with `=`;
  OK to build the signing feature on them until their stable releases?
- Setup note: a fresh worktree has no PDFium or fixture corpus; symlink `third_party/pdfium` and
  `fixtures/out` from the main checkout (both are git-ignored).

### Session M5-a — 2026-11-22 (Milestone 5: true redaction, pattern search, sanitize; parallel to M3/M4)

Branch `pomidor/m5-redaction-e527d5`, from `main` (M0–M2, M3 groundwork). Avoided `pdf::text`, `typeset`
and OCR code, as asked; the redactor has its own content-stream tokenizer (`redact/content.rs`).

- **ADR 0042** records the approach. In short: our own content-stream rewrite (PDFium never
  regenerates content, ADR 0005's amendment); glyphs cut out by character code with exact `TJ` gap
  compensation; image pixels overwritten in per-drawing copies; paths cut or dropped; annotations
  removed; redacted text scrubbed out of all metadata strings and XMP; a black box drawn; the result
  checked by extraction (`redaction_failed` if text is still readable under a mark); saving's full
  rewrite drops the old objects.
- **Engine (`safha_core::redact`):** `redaction_update`, `find_matches` + `Preset`,
  `sanitize_update`; `Document::with_redactions` / `with_sanitized`. A new fixture,
  `personal-data` (Arabic-Indic IDs and phones, e-mail, IBAN), was added to the corpus (26 files).
- **Shell:** `find_redactions`, `apply_redactions`, `sanitize_document` (`redaction.rs`); the
  `redaction_failed` error code.
- **UI:** the Protect tool's panel in the inspector, the apply confirmation, a Redact button in the
  selection toolbar, drag-to-mark and the marks' overlay in the viewer (`mode: "redact"`), 17 new
  commands, English and Arabic strings, `--color-redact-*` tokens. Backend mock and types updated.
- **Bugs the tests found:** inline images read by length did not advance the lexer; Chrome's
  zero-advance Type 3 dots were missed until their `d1` boxes were read; the IBAN pattern swallowed
  the next word; an image replaced in a copy stayed reachable through its old resource name; a
  trailer without `/Info` does not hide it (readers merge trailers).
- **Verified:** see ADR 0042. `cargo test -p safha-core` (18 redaction tests, including every text
  block of every corpus fixture and sanitizing every corpus file), UI tests in both languages,
  clippy and biome clean, the panel seen in the browser build in English and Arabic.
- **Not done / not checked:** JPEG pixel editing (owner item 15), encrypted files, a signed file's
  snapshot path, the native shell's drag on a real PDF, a native-speaker review of the new Arabic strings
  (owner item 11), the autosave recovery file keeping pre-redaction bytes on disk until save.

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

### Session 10 — 2026-11-22 (Milestone 3: editor-surface spike, page analysis, in-place editing)

- **Spike (ADR 0022):** the webview's own text layout matched the engine's line breaks in 5 of
  5 paragraphs and its character positions to 0.01 pt (1.1 pt after a Latin run in mixed
  text). Decision: a native text field, no custom caret. Unmeasured: real IMEs, WKWebView.
- **Reading pages (ADR 0023):** `pdf::content` (tokenizer with byte ranges, interpreter),
  `pdf::fontinfo` (ToUnicode, encodings, widths), `blocks` (glyphs → lines → paragraphs, logical
  order, direction, alignment). `cargo run -p safha-core --example page_blocks -- f.pdf` lists
  a page's paragraphs.
- **Editing (ADR 0023):** `block_edit`; shell commands `page_text_blocks`, `edit_text_block`;
  the Edit tool opens a pressed paragraph; the editor shows which font sets it.
- **Findings on the fixtures:** CoreText's Arabic is not read (dots are separate shapes with no
  text), Chrome's pages here use Type 3 fonts, so the editable fixtures are CoreText English,
  our own written text, and the turned-pages file.
- **Tests:** 127 lib + 11 new integration tests (`tests/block_edit.rs`); 158 UI tests; real
  app self-test, English/light and Arabic/dark, edits an Arabic paragraph and reads it back.
- **Next:** spec §7's exit test as a whole (four acceptance tests against Chrome/Acrobat
  output), the field in the paragraph's own width and size, installed-font substitutes, and
  the line-count self-check named in ADR 0022.

### Session 9 — 2026-11-22 (Milestone 3 started: text in page content, the add-text tool)

- **M2 merged into `main`** (PR #2) at the owner's go-ahead; M3 continues on `m3-text`.
- **Text in page content (ADR 0021):**
  - New text gets its own content stream under a placing `cm`, so it stands upright on turned
    pages.
  - The page's original content is wrapped once in `q … Q`, so a page that leaves its
    graphics state changed cannot move the new text.
  - The page gets its own `/Resources`, adding the fonts under unused names.
  - Encrypted files are supported, and undo works (a new change kind: `Content`).
- **Bundled families:** Sans (IBM Plex) and Naskh (Amiri), now one module shared with text
  boxes.
- **The Edit tool:** a press places new text; a strip sets font, size, colour, alignment and
  direction. A content change repaints in place, re-reads page text, and clears stale search
  results.
- **Extraction fix:** a ligature whose letters cannot tell their order (Plex's noon-yeh "ني")
  read back reversed on pages with no other evidence. It is now reversed by default, as PDFium
  reports such letters. The corpus is unchanged at 86/118.
- **Self-test:** Arabic text placed with the Edit tool is painted, reads back as page text,
  and is undone, in English and Arabic.
- **Tests:** 270 Rust and 155 UI tests pass on this Mac.

### Session 8 — 2026-11-22 (Milestone 2: autosave and recovery, comments panel, close-out)

- **Autosave (ADR 0040):**
  - After each edit, undo or redo, a modified document's current bytes (a valid PDF: the file
    as opened plus each edit's incremental update) are written to a recovery file 2 seconds
    later. A burst of edits writes once.
  - The state store's new `recovery` table (schema 3) records each file's original path, name,
    time, and the original's size and modification time.
  - The file goes away when the document is saved, undone back to its saved revision, or closed
    or quit without saving.
- **Recovery:**
  - The first window's startup lists recovery files no open document holds. A dialog offers
    each: Restore, Discard, or Decide later (offered again at the next start).
  - A restored document opens modified. When the original is unchanged, undo goes back to it;
    when it changed or is gone, Save asks for a new file (`save_as_required`).
  - An encrypted recovery file uses the usual password prompt; a wrong password asks again.
  - Save-and-close no longer closes the tab when that Save As is cancelled.
- **Self-test:** after a page is duplicated the document has a recovery file, and undo back to
  the file removes it (a devtools command, `selftest_has_recovery_file`).
- **CI fix:** the macOS and Windows runners draw the page at under 1 pixel per point, where a
  note's icon and a text box have 19–38 matching pixels. The fixed 50-pixel bar failed them.
  The bar now follows the render scale (8 px per square px/pt, at least 8).
- **CI fix (Windows):** the shell's unit-test binary failed to start
  (STATUS_ENTRYPOINT_NOT_FOUND) once autosave made test code reach the Tauri runtime. Tauri
  embeds its Common Controls v6 manifest into the app binary only. `build.rs` now has the
  linker embed the same manifest (`windows-app-manifest.xml`) into every binary, tests included.
- **Comments panel (ADR 0041):**
  - A fourth view of the side panel lists every comment the app knows: author, date, the
    marked text (read in logical order, Arabic too), its replies, and whether it is resolved.
  - Filter by status, type and author; sort by page, latest activity, or author. Choosing a
    comment scrolls to it and selects it.
  - Reply and Resolve/Reopen write standard annotations: `/IRT` replies and `/Review` states,
    with empty appearances so they draw nothing. Threads from other viewers (replies to
    replies, several states, groups) are followed; removing a comment takes its thread along.
  - Our parser reads the thread structure (PDFium's safe API exposes no `/IRT`); PDFium reads
    the decrypted text, names and dates.
  - "Your name on comments" (a setting) signs new annotations of every kind. The system's
    user name is never written unasked.
- **Closing M2:**
  - **Split** (ADR 0016 amendment): at top-level bookmarks (one file per entry, named after
    it; titles made safe for file names) and by file size (each file holds as many pages as
    fit, measured by writing it). The split dialog offers all three ways and lists the
    bookmark titles.
  - **Inserting from password-protected files:** the page area asks for each file's password
    in turn, then inserts them all as one undo step.
  - **Budgets:** search, scrolling, jumps and memory measured against `main`; no regression
    (table in M2).
  - **Demo script:** `docs/demos/m2.md`.
- **Tests:** 259 Rust and 153 UI tests pass on this Mac. The self-test passes in English (LTR)
  and Arabic (RTL, dark), autosave and the comments panel included. CI is green on macOS,
  Linux and Windows.

### Session 7 — 2026-11-22 (Milestone 2: markup, ink, notes, text boxes)

- **CI on PR #2:** a slow macOS runner failed the scroll benchmark's timing. On CI every
  benchmark timing check now warns; only the app's own memory fails the run (ADR 0011). After
  that, every check was green.
- **Markup annotations (ADR 0017):**
  - Highlight, underline, strikethrough and squiggly from the selection toolbar or the command
    palette.
  - Each is a standard annotation with its own appearance stream, written as an incremental
    update, so undo covers it.
  - A selection across pages is one undo step.
  - Click a markup to select it; Delete (⌫ on a Mac) removes it, with its pop-up and replies.
- **Encrypted files:**
  - Objects an edit adds are now encrypted, with RC4 (40–128 bits), AES-128 and AES-256.
  - The key is checked with the user or the owner password.
  - Tested on seven encrypted variants made with qpdf.
- **No flash after markup:**
  - an annotation change paints the pages again instead of rebuilding the view;
  - tiles, previews, thumbnails, and organizer images stay until their new renders arrive.
- **Fix:** a tab's close button kept its English label after switching to Arabic (React Aria
  cached the tab items).
- **Self-test:** highlights the first page's text and checks the yellow on screen in the same
  view, and undo; in English and Arabic.
- **Merged `main`** into `m2-organize` twice: the Milestone 3 groundwork (to clear PR #2's
  conflicts), then the rename to Mirsam.
- **Ink and sticky notes (ADR 0018):**
  - **The Comment tool** has Note, Draw and Erase-ink modes.
  - **Ink** widens with pen pressure. A preview stands in until the page repaints, and the
    eraser removes whole strokes.
  - **Notes** are `/Text` annotations with their text in `/Contents`. Their icon stands upright
    on turned pages.
  - **The note editor's** text field takes its direction from the text (`dir="auto"`).
  - **The annotation module** is now a folder: markup, ink, notes.
- **Text boxes (ADR 0019):**
  - **Shaped text:** M3's `typeset` lays out English, Arabic and mixed text. The new
    `pdf::text` writer draws it with embedded IBM Plex subsets (the text half of the
    content-stream writer).
  - **Layout:** a box stands upright on turned pages; its height fits the text and grows
    downward when the text changes; saving it empty removes it.
  - **Tests:** each box is flattened with qpdf and read back by our extractor, exactly, in
    logical order.
  - **Extraction:** word-space markers are now read inside form XObjects too, which flattened
    Arabic needed.
- **Tests:** 238 Rust and 142 UI tests pass on this Mac. The self-test passes in English (LTR)
  and Arabic (RTL, dark): markup, an Arabic note, an Arabic text box, and a pressure stroke,
  each painted and undone.

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

CI on this PR hit a Windows runner measuring 1,133 ms for the first page (main measures about
200 ms), so it carries `m2-organize`'s CI timing changes unchanged (the CI half of 353d2a6, and 65ba020):
with `--ci`, the self-test's first-page budget and every scroll-benchmark timing check warn on
hosted runners; behaviour checks and the shell's memory still fail the run. Identical hunks, so
`m2-organize` still merges cleanly.

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
- **Milestone 1 closed** and merged to `main` (PR #1, every CI check green). The owner decided
  that the memory budget covers the app's own process.
- **Milestone 2 started** on `m2-organize`:
  - **Editing model (ADR 0014):** each edit is an incremental update written by our own writer,
    and undo is a stack of them.
  - **Page edits** (rotate, delete, duplicate, insert blank) on the current page, with undo and
    redo, in the core, the shell, and the UI.
  - **Self-test:** checks an edit and its undo in the real app in both directions.
  - **Save and Save As** (ADR 0015). The app asks before closing a modified tab, window, or the
    whole app; the self-test checks the window prompt.
  - **Page organizer:** the Pages tool shows a page grid with multi-select, drag to reorder,
    and move earlier/later. The self-test drives it on multi-page files.
  - **`selftest.sh`** now refuses to run when port 1420 already serves a UI. It used to test
    another checkout's UI silently.
  - **Draft PR #2** is open with Auto-fix on.
  - **Pages across files** (ADR 0016): insert pages from other PDFs (also merges), extract
    pages to a new file, split into files of N pages. A notice says what was written.
  - **Tests:** 147 Rust and 129 UI tests pass on this Mac.

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
