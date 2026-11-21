# 0007 — Generated fixture corpus and in-app self-test harness

Status: Accepted. Date: 2026-11-22.

## Fixture corpus

**Decision.** Fixtures are generated, never committed (spec §0 rule 8). One content file per case
in `fixtures/content/` is both the layout input and the ground truth: each block's `text` is the
logical-order string that extraction, search, and copy must reproduce. `fixtures/generate.mjs`
lays the same content out through several *producers*, because each PDF producer stores Arabic
differently, validates every file with `qpdf --check`, and writes a manifest with the expected
text. Only OFL fonts, loaded from `assets/fonts/`, are embedded.

Set today (20 files):

- 8 cases × 2 producers — **Chrome** (Skia PDF backend) and **CoreText/Quartz** (macOS):
  English baseline, Arabic prose in Naskh and in a Kufi-style sans, fully vocalized text,
  lam-alef in every variant, mixed-direction lines (numbers, Latin names, e-mail, phone, brackets
  and punctuation at boundaries, Arabic-Indic digits, Arabic inside English), an RTL bilingual
  table, and a two-column bilingual contract.
- 4 derived with qpdf: linearized, AES-256 encrypted, truncated at 60%, and a 1,000-page
  bilingual document (the M1 exit-test file).

**Not yet covered** (tools not installed on this machine; see PROGRESS.md): Word, LibreOffice,
XeLaTeX, InDesign, a Tesseract-OCR'd scan, form-heavy and scanned files. The generator is built so
adding a producer is one entry in `PRODUCERS`.

Tests fail with instructions when the corpus is missing; they never skip.

## Self-test harness

**Context.** WKWebView has no WebDriver, so the usual Tauri end-to-end route does not exist on
macOS, our primary development platform. We still need proof that PDFium output reaches the
canvas in the real shell, in both directions.

**Decision.** The app tests itself. A `devtools` build launched with `SAFHA_SELFTEST_OUT=<dir>`
opens the startup document, waits for the first page to reach the canvas, and reports what is
actually on screen — layout rectangles, `dir`, theme, timings, the ink ratio of the canvas, and
the canvas pixels as a PNG — then exits. `scripts/selftest.sh` runs it;
`scripts/check-selftest.mjs` asserts on the report. The harness is not compiled into release
builds.

**Limits.** It reads back the canvas, not the composited window, so it cannot catch CSS that
hides the canvas; UI appearance is reviewed in the browser against the mock backend. A window
that is occluded, or a display that is asleep, receives no animation frames; the harness falls
back to timers and records `visibility` in its report.
