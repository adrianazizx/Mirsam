# 0037 — OCR review mode: decisions, corrections in the text layer, and Undo

Status: Accepted (M6). Date: 2026-11-23.
Builds on ADR 0033 (reconcile: what is flagged), ADR 0035 (jobs, the text layer, the review
list), ADR 0014 (edits as incremental updates).

## Context

M4 left a pages-needing-review list (ADR 0035 §4): pages with words the readers disagreed on,
words corrected by agreeing re-reads, number–unit compounds put in logical order. A person could
open a word on its page, but not do anything about it. Spec §5.8 asks for a review mode: step
through the flagged words beside their image, and correct them. Spec §8: nothing uncertain is
invented, and uncertain words are flagged, not guessed.

Three things make this harder than a list with a text field:

- **The correction must reach search and copy.** The word's text lives in the page's invisible
  layer, written by our writer (ADR 0035 §2), not only in the app's state store.
- **A correction is an edit of the document,** so Undo takes it away. If the review data said
  "corrected" while the document had been undone, the review would claim a fix the file does
  not have: a silent wrong state.
- **Words are flagged for different reasons,** and not all of them need a decision.

## Decision

### 1. What is flagged (`ocr::review::reasons`)

| Reason | When | Blocks conversion (ADR 0039) |
|---|---|---|
| `uncertain` | the readers disagreed; the page read was kept | yes, until decided |
| `corrected` | re-reads agreed on something else; replaced | no |
| `reordered` | a number–unit compound turned into logical order | no |
| `lowConfidence` | the primary's probability under 0.80, even when re-reads agreed | no |
| `number` | any word with a digit (numbers, dates, IDs) — **only on request** | no |

A cut-off reading (the model hit its token limit) flags the page itself.

### 2. Decisions (`ocr::review::Action`)

- **Accept:** the text as recognised is right.
- **Correct:** a person types the text (or picks another engine's reading). It goes into the
  layer. The word keeps what the pipeline read (`Edited { read }`), so a correction is never
  mistaken for recognition. An empty correction takes the word out of the layer (a speck read as
  a letter).
- **Mark checked:** looked at and left as it is, without vouching for it. This counts as a
  decision: a person saw the image.
- **Check page:** every word still open on the page is checked, and so is a cut-off page.

### 3. Corrections are layer rewrites, as incremental updates

A correction rewrites the page's whole layer from the kept words (one stream replacing the
previous one in `/Contents`, ADR 0035 §2), as one undoable edit. Two changes keep it small:

- **The layer's font is reused.** When every page in an update already has our layer font, the
  new stream uses that font object, instead of writing another copy. A correction is then about
  1–2 KB: a page dictionary, a compressed content stream, an xref section.
- **No other text is touched.** `Clear::None` (ADR 0038) on correction layers.

### 4. Decisions follow the document: a tree keyed by layer digest

- **Each layer records a digest.** Its stream dictionary carries `/MirsamDigest`: a 32-bit hash
  of the layer's quality, words and boxes (`PageLayer::digest`). `Document::text_layer_states`
  reads it back for every page, without decompressing anything.
- **Each decision records the digest it leaves,** and its parent: the state it was taken in. A
  page's kept result is a tree:
  - its root is the result as recognised;
  - each node is a decision;
  - accepting or checking keeps the parent's digest;
  - a correction makes a new one.
- **Review shows the newest state whose digest is the document's** (`review::resolve`):
  - **undo a correction:** the word is open and flagged again;
  - **redo it:** it is corrected again;
  - **a decision taken after an undo** is a sibling branch, and redo leaves it behind;
  - **an unsaved correction lost on quit** (the file has the recognised layer) is open again.
- **No layer of ours:** the page has no review.
- **A draft layer** (the page is being read again): no review either.
- **A layer the review has no record of** (its job removed, or written elsewhere) is shown, but
  `stale`. Corrections are refused there (`ocr_review_stale`), since rewriting the layer from the
  kept words would overwrite text the review cannot see.
- **Layers written before digests** resolve to the newest state, as M4 did.

The decisions live in the state store with the page's result (`PageOcr::decisions`, JSON), so no
schema change was needed for them.

### 5. Review mode in the UI

- **Where it lives.** A card at the top of the OCR panel, opened from the review list
  (`ocr.startReview`).
- **What it shows:**
  - the scan around the word, with the word's box marked;
  - why it is flagged, what the other engines read, and what a person decided;
  - a field holding the text.
- **The order:** the words still open, page by page in reading order, then pages to check.
  Each decision moves to the next.
- **Keys:**
  - **Enter** accepts, or corrects when the field was changed;
  - **⌘/Ctrl+Enter** marks checked;
  - **⌘/Ctrl+Shift+Enter** checks the page;
  - **F8 / Shift+F8** skip forward and back;
  - **Escape** in the field leaves review mode;
  - other readings are buttons.
- **The image** comes from `safha://localhost/ocr-crop?doc=&page=&x0=&y0=&x1=&y1=&h=`: an area of
  the page rendered by the shell as a BMP that the webview decodes (ADR 0011), up to 12 pixels a
  point.
- **The page geometry** is drawn left to right in both UI directions (`dir="ltr"` on the image
  box). The word itself sits in a `dir="auto"` field.
- **Commands** (all in the registry):
  - `ocr.startReview`, `ocr.stopReview`;
  - `ocr.reviewConfirm`, `ocr.reviewChoose`, `ocr.reviewCheck`, `ocr.reviewCheckPage`;
  - `ocr.reviewNumbers`;
  - `ocr.nextFlag` and `ocr.previousFlag`, now with F8 / Shift+F8.
- **Undo and Redo** reload the review list when the document's text changed.

## Evidence

- `ocr::review` unit tests:
  - flags and their reasons, numbers only on request;
  - an edit goes into the layer and keeps the reading;
  - Undo and Redo through the tree;
  - checking a page;
  - results kept before word positions were recorded find their places by text and box.
- `ocr::jobs` test `review_decisions_write_the_layer_and_follow_undo`: through the real runner
  and state store:
  - a correction comes back as a layer;
  - the document's undo brings the word back open;
  - accept and check write nothing;
  - a stale layer refuses corrections.
- `tests/ocr_review.rs`, `a_correction_rewrites_the_layer_and_reuses_its_font`, on a real
  file:
  - an Arabic-Indic price and an English word are corrected;
  - the update is under 4 KB and holds no font program;
  - this app reads the corrections back;
  - the digest in the file is the corrected layer's;
  - `qpdf --check` passes.
- UI tests (`commands/ocrReview.test.tsx`):
  - a correction typed and entered is an undoable edit;
  - the card moves on, accepts, checks and checks the page by keyboard;
  - every number on request;
  - Arabic RTL with Arabic-Indic digits, the image box left to right.

## Known limits

- **A decision taken after an undo** is lost when the undone correction is redone (a branch of
  the tree, by design).
- **The review data is still kept by file** (ADR 0035): a file renamed outside the app loses it.
- **The crop of a page converted with its scan removed** shows the page text, not the scan
  (ADR 0039 requires decisions first, so only already-decided or non-blocking words remain).
