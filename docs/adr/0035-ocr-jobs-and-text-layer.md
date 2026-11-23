# 0035 — Background OCR jobs, word boxes, and the searchable-PDF text layer

Status: Accepted (M4). Date: 2026-11-23.
Builds on ADR 0005 (amendment: our own writer), ADR 0014 (edits as incremental updates),
ADR 0030–0034 (engines, word-box spike, runtime, reconcile, downloads).

## Context

M4's pipeline reads a page well: all five gates pass on `ocr-eval` set v2 (ADR 0033). What
M4 still lacked is everything that makes that reading useful in the app (spec §8, §5.8):

- **OCR must never block the app.** It runs as background jobs with per-page progress,
  pause, resume and cancel, survives a restart, and leaves the document usable meanwhile.
- **Progressive results.** A quick pass makes the document searchable within moments.
  Each page's text is then replaced by the quality result, marked "draft" until then.
- **The output** is an invisible text layer, aligned to the image (word boxes at least),
  in logical order, so search and copy work everywhere. The known pitfall: layers that
  other viewers read as reversed Arabic. The original image is never touched, and the
  layer can be removed or redone.
- **Confidence per word and per page**, kept for review; a pages-needing-review list.

Nuqta writes text without positions. ADR 0031 measured how to give its words boxes: detector
lines, a rough reading of each line as a guide, cuts at ink gaps in UAX #9 order. That spike
used PaddleOCR's detector and recogniser, which have no runtime in our build.

## Decision

### 1. Jobs (`safha_core::ocr::jobs`)

- **The runner.** `JobRunner` keeps a queue of jobs (a file and its pages) and one worker
  thread. Jobs run one after another, oldest first, while their document is open.
  - The worker sees documents only through `JobHost` (the shell): which files are open,
    rendering a page, and writing layers as an edit.
  - It sees engines only through `JobEngines`, so tests drive it with scripted ones.
- **Draft pass, over every page first.** The platform's OCR reads each page at 200 dpi:
  Apple Vision on macOS, Tesseract `ara+eng` elsewhere when it is installed. Its line and
  word boxes become a `Draft` layer at once. Pages are written in batches of 8, or every
  1.5 s, as one edit each.
- **Quality pass, page by page.**
  - The page is read at 300 dpi (capped at 7,000 px on the long side) by `QualityReader`:
    Nuqta 4B, with Nuqta 0.8B re-reading (ADR 0033). There is no third reader, as measured.
  - The words get boxes (section 3), and a `Final` layer takes the draft's place.
  - One edit per page.
- **Kept, then written.** Each page result is stored before it goes into the document. The
  store holds the layer, and the quality pass's words with their probability, status and
  box. Schema 5 adds the tables `ocr_jobs` and `ocr_pages`.
- **Restart.** A job that was running when the app quit is queued again. When its document
  opens:
  1. the results the document lacks (it was not saved) are written back, without reading
     those pages again;
  2. the job carries on with the pages left.

  A document whose page count changed is not resumed: `ocr_document_changed`.
- **Controls.**
  - *Pause* stops after the page being read.
  - *Cancel* stops at once. The models' servers are stopped mid-request; a quality page
    takes 25 s on Metal and minutes on a CPU. What was written stays, and Undo removes it.
  - *Resume* continues where the job stopped.
  - *Remove* forgets a finished job and its review data.
- **Servers are shared across jobs.** `LocalEngines` starts the 4B and 0.8B `llama-server`s
  on first use and keeps them for later jobs. They stop after 5 minutes with nothing to do,
  and always when the app exits (they hold 14 GiB together).
  - **Device:** the GPU when the installed runtime has a backend for this machine
    (`Hardware::runtime_gpu`), else the CPU. The acceleration the download manager
    detects is now used.
- **In the document.**
  - **An edit like any other.** Each batch of layers is one content edit (ADR 0014): on
    the undo stack, autosaved (ADR 0040), the search index rebuilt.
  - **Signed documents are refused** (`ocr_signed_document`). A text layer added after
    signing shows as a change to the signed document.
  - **Files are named by canonical path,** as the state store names them. Save As moves the
    document's jobs to the new file.
- **The Vision helper** is a Swift program (`ocr/vision.swift`). It stays running for a
  job's pages and answers one JSON line per page with its lines, words and boxes.
  - It is built from the embedded source on first use with Xcode's `swiftc`, into the
    cache, which takes 2 s.
  - `SAFHA_VISION_HELPER` names a prebuilt one instead. Shipping it prebuilt and signed
    belongs with signing `llama-server` (M7).

### 2. The text layer (`ocr::layer`, `ocr::font`)

- **Our own writer, as an incremental update** (ADR 0005's amendment). PDFium regenerates
  nothing.
  - **The layer's stream.** Each page gets one content stream. Its dictionary says
    `/MirsamOCR /Draft` or `/Final`. It goes after the page's own content, which is
    wrapped once in `q … Q`, as stamps and added text are.
  - **Replacing.** A later layer takes the earlier one's place in `/Contents`, so the
    final text replaces the draft and a re-run replaces both.
  - **Encrypted files.** New objects are encrypted with the file's key.
- **Its font has no outlines worth drawing,** in the way of Tesseract's "GlyphLessFont":
  - **Codes are UTF-16 code units** (Identity-H). `/ToUnicode` is the identity, as 256
    ranges.
  - **Every letter draws glyph 1,** one advance wide. A word is stretched over its box by
    horizontal scaling (`Tz`); the box spans the font's ascent and descent, so readers'
    character boxes fall where the image has the word.
  - **Glyph 1's outline is a rectangle,** never painted (render mode 3). PDFium skips a
    text object whose glyphs have no extent, and that silently dropped one-letter words
    ("1", "و") from Chrome's copy.
  - **Tashkeel and other combining marks have no advance,** so a mark is drawn from its
    letter's origin, as a shaping engine draws it, and readers attach it there.
    - Marks draw glyphs 2–4: small rectangles of different shapes for vowels, shadda
      and hamza.
    - Preview (PDFKit) drops a glyph that exactly overlaps another, as it does for fake
      bold. That dropped a vowel stacked on a shadda.
  - **About 500 bytes,** built in code, so there is no font licence to consider. One copy
    goes in each update.
- **Order: what a word processor writes** (`Order::LineVisual`, chosen by measurement,
  below). Each line is one run of text objects, left to right as drawn. The line's
  characters are put in visual order (UAX #9 on the whole line, in its direction) and cut
  at its spaces, one run per word box. A space glyph fills each gap, at least a fifth of
  the line's height wide.
  - Punctuation drawn away from its word moves to where it is drawn, for example "؟"
    after an English word in an Arabic line.
  - Readers that rebuild reading order from positions (this app, PDFKit) and PDFium's
    own bidi pass both get logical order back.
- **Placement is taken when the page is rendered for OCR.** A page rotated since still
  gets its words where the image has them.

### 3. Word boxes (`ocr::boxes`): ADR 0031's method, with the draft reader as detector and guide

ADR 0031 named "the platform OCR" as the next guide to try. The draft reader already gives
each line a box and a rough reading, so it serves as both the line detector and the guide.
The PP-OCRv5 detector and recogniser are not needed. Apple Vision's rough reading (9.5%
CER, ADR 0030) is a better guide than PP-OCRv5's (32%).

1. **Pieces of one line are joined.** Vision cuts a line where its direction changes, and a
   piece alone can look like a line of the other direction. Pieces on one row, less than
   two line heights apart, become one line. Their texts are joined in the row's reading
   order.
2. **Line order.** Rows (either direction) or two columns (either first): the order whose
   guide text best matches the recogniser's text.
3. **Words to lines.** A longest-common-subsequence alignment of characters votes each word
   into a line (ADR 0031's M1).
   - A word anchors only where at least half its characters matched. Fewer is chance:
     Vision wrote "bit-64" for "64-bit" and matched a bracket on the next line.
   - Unanchored words fill the lines between their anchored neighbours by capacity.
   - Below half the characters matched overall, a dynamic program fits words to lines by
     width instead (M4).
4. **Order within a line:** UAX #9 on the line's words, in its direction. The direction
   comes from the column margin the line touches, else its first strong letter. A word
   sits where its letters are.
5. **Cutting:** proportional cuts moved to ink gaps by a dynamic program; each box is
   tightened to its ink (M2).

A word placed by capacity or width alone is marked `placed_by_width`, as ADR 0031 decided.
Without a draft reader, the page's own line strips (ADR 0033) are the lines, and there is
no guide.

### 4. The pages-needing-review list (UI)

- **The OCR tool's panel in the inspector** has three parts:
  1. **Recognize:** the whole document, or this page.
  2. **Jobs:** each with a two-layer progress bar (searchable pages under finished ones),
     what it is doing ("Quick draft of page 3…", "Reading page 3 in quality mode…"), why
     it stopped, and its controls.
  3. **Pages needing review:** decisions first. Each page shows its counts (unsure,
     corrected, order fixed, cut off) and its mean confidence. Its flagged words follow,
     with their status, the other readings, and their confidence. Choosing a word opens
     its page with the word's box marked; Next and Previous step through them.
- **Commands** (registry; en and ar strings):
  - `ocr.recognize`, `ocr.recognizePage`;
  - `ocr.pauseJob`, `ocr.resumeJob`, `ocr.cancelJob`, `ocr.removeJob`;
  - `ocr.review`, `ocr.showWord`, `ocr.nextFlag`, `ocr.previousFlag`.
- **Events from the shell:**
  - `ocr-job`: every window follows the jobs;
  - `ocr-applied`: a document's own window repaints it and reloads its review list.
- **Names and directions.** Words and file names are isolated in labels; Arabic labels keep
  Western digits (owner item 5).

## Evidence

### Which order readers can read back (`examples/ocr_layer_probe.rs`)

The same eight lines were written as a layer on a blank page in each order:

- Arabic with ligatures;
- fully vocalized;
- Arabic with "iPhone 15 Pro" and Arabic-Indic digits;
- "(64-bit)" in Arabic;
- a phone number in Arabic;
- Arabic with Arabic-Indic numbers;
- English with "(العقد)";
- plain English.

Each file was read back by four readers. The count is lines exactly right, after NFC.

| Order | This app | PDFium (Chrome's copy) | PDFKit (Preview) | Poppler |
|---|---|---|---|---|
| **Line, visual (chosen)** | **7/8** | 4/8 | **8/8** | 3/8 |
| Line, visual, with left-to-right runs reversed for PDFium | 6/8 | 5/8 | 7/8 | 3/8 |
| Word, visual | 6/8 | 4/8 | 2/8 | 3/8 |
| Word, logical, mirrored matrix | 3/8 | 5/8 | 0/8 | 3/8 |
| Word, logical | 1/8 | 1/8 | 1/8 | 1/8 |

**Where the chosen order still fails:**
- **This app:** the "64-bit" line. The image cannot tell "64-bit" from "bit-64" in an
  Arabic line (ADR 0033).
- **PDFium** gets every Arabic line and every number line right. It fails in four cases,
  as it does on Chrome's own PDFs:
  - it reverses the words of an English run inside Arabic ("Pro 15 iPhone");
  - it keeps "966+";
  - it mixes up brackets at direction changes, twice.
- **Poppler** drops the space where the direction changes. It is not one of the target
  viewers.

`tests/ocr_layer.rs` keeps this measurement as a test: every line exact in this app and in
PDFKit, the Arabic lines exact in PDFium. It also checks that nothing new is drawn, that a
draft is replaced by the final text (one font, one layer), and an encrypted file. Every
write passes `qpdf --check`.

### The pipeline with its layer, on the evaluation set (`examples/ocr_bench.rs`)

The bench runs the quality pipeline over all 47 pages of set v2. It writes each page's text
layer into a one-image PDF of the page and reads it back with this app's extractor: what
search and copy see. The numbers are from the replay of the recorded run (ADR 0036). Its
text is byte-identical to the real run's on all 47 pages.

| CER (fields, cells) | Pipeline's text | The searchable PDF, read back |
|---|---|---|
| All 47 pages | 0.60% | 5.72% |
| All, without the two-column contract (45 pages) | — | **0.88%** |
| Clean Arabic | 0.00% | 0.00% |
| Clean English | 0.00% | 0.00% |
| Other typefaces | 0.00% | 0.00% |
| Diacritized | 0.00% | 0.00% |
| Degraded Arabic | 0.00% | 0.06% |
| Severe scans (tracked) | 1.46% | 2.00% |
| Degraded mixed | 0.16% | 0.47% |
| Mixed Arabic/English | 1.52% | 35.86% (0.73% without the contract) |
| Numbers, dates, IDs: fields | 99.42% | 98.25% |
| Table cells | 100% | 100% |

**Where the read-back loses:**
- **The two-column bilingual contract.** This app reads it paragraph by paragraph across
  the two columns: the Arabic paragraph, then its English twin. It reads the born-digital
  contract the same way (`examples/page_text` on the fixture). The set's truth wants each
  column whole. This is the M1 reader's order for side-by-side text, not the layer's.
- **"(64-bit)" in an Arabic line.** It is written as drawn ("bit-64"), and the app's own
  reader turns it back into "bit-64", undoing the pipeline's order fix (ADR 0033). The
  image cannot tell the two apart (ADR 0008). This costs two fields.
- **Severe scans.** A word on the wrong line here and there.

**Word boxes**, against Chrome's layout of the 26 geometry-preserving pages (ADR 0031's
truth and measures, `ocr_bench --truth-boxes`; 1,278 paired words):

| | Right line | IoU-x mean | IoU-x ≥ 0.5 | IoU-x ≥ 0.7 |
|---|---|---|---|---|
| ADR 0031's M5 (PP-OCRv5 detector and guide, Python) | 99.0% | 0.92 | 93.0% | 90.8% |
| **This (Apple Vision's lines as detector and guide, Rust)** | **99.0%** | **0.93** | **95.0%** | **92.3%** |

104 of 2,538 words (4.1%) were placed by capacity or width, and are marked so.

**Fixed while measuring.** These were found on the full set:
- **One font size per column.** Word boxes are tight to ink, so a line's tallest letter or
  deepest descender made its band taller than its pitch, and two Amiri lines merged into one
  in the reader. Lines of one column also got different font sizes, so readers would not
  group them into paragraphs. Each line now takes the median of its words' tops and bottoms,
  and the median height of its column (clean Arabic read-back: 7.45% → 0.00%).
- **Wide word gaps closed.** Boxes tight to faint ink on a 100 dpi fax left word gaps over
  an em wide, and readers cut lines into columns there. Gaps between 0.4 and 1.5 line
  heights are now closed (degraded mixed: 9.31% → 0.47%). Wider gaps are real cell gaps
  and stay.

### The exit test's document (`docs/demos/m4.md`)

A three-page scan was made of the bilingual fixtures (contract, mixed lines, table) at
200 dpi, with no text. It went through `examples/ocr_document`: the real runner, the engines
installed by the download manager, the state store.

- **Drafts:** all 3 pages in 0.9 s, one edit.
- **Quality pages:** 45 s for the first (with the servers' start), then 38 s and 19 s.
- **Quit after the first quality page, then run again:** the stored results went back into
  the unsaved document at once ("1:Final 2:Draft 3:Draft"), and only pages 2 and 3 were
  read. Done.
- **The result passes `qpdf --check`.** This app finds "الإيجار", "Lease", "iPhone 15 Pro",
  "طباعة", "Printer ink", and "1234" (as "١٢٣٤").
- **Copy:**
  - **This app** reads every line in logical order, as it reads the born-digital original.
  - **Preview** reads every line but two: "INV-2026-00417" comes out flipped, and the line
    holding the misplaced "(64-bit)!" is scrambled. Preview breaks the born-digital
    original's Arabic apart.
  - **PDFium** (Chrome's engine) reads every Arabic word in order. It turns English runs
    inside Arabic lines around and flips brackets at direction changes. It reverses the
    original's Arabic letters.

### Jobs

`ocr::jobs` tests run the real runner with a scripted host and engines:

- drafts in one edit, then one quality edit per page, each kept with its words;
- cancel stops the engines mid-page;
- a restart writes back what the document lost and reads only the pages left;
- a job waits for its document;
- a changed document is refused;
- the review list's order.

The shell test checks that jobs reach the UI without their path. The UI tests cover a job
from start to review, pause, resume, cancel, a failure's message, and Arabic RTL.
`examples/ocr_document.rs` runs a whole PDF through the same runner, engines and store,
headless: see the M4 exit test (`docs/demos/m4.md`).

## Known limits

- **PDFium's copy of mixed lines.** English words inside an Arabic line come out in reverse
  order in Chrome's copy, and brackets at direction changes may flip. The only order that
  fixed the English words cost PDFKit and this app more.
- **Acrobat Reader is untested** (not installed here). The layer follows the structure word
  processors write, so it is expected to read like their files.
- **Moving pages while a job runs:** results are written to pages by number. A job stops
  only when the page count changes.
- **Review data is kept by file.** It follows Save As, but not a file renamed or moved
  outside the app.
- **The draft reader on Windows and Linux** is Tesseract with the optional `tesseract-best`
  models (owner item 23, decided 2026-11-23; ADR 0034's amendment). The `tesseract` program
  itself is the user's: the distribution's package on Linux, or Tesseract's installer on Windows,
  whose bundled LGPL libraries wait on owner item 26. With no program, the quality pass runs
  alone, and boxes come from the page's own line strips with no guide: wider boxes on two-column
  pages.
- **The Vision helper is built on first use,** and needs Xcode's command-line tools. A
  prebuilt, signed helper is M7.
- **Both models stay in memory during a job:** 14 GiB on a CPU-only machine. The 0.8B-only
  fallback for small machines (ADR 0030) still needs its memory threshold.
