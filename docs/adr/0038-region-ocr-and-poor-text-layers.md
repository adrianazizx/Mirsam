# 0038 — Reading an area again, and replacing another tool's poor text layer

Status: Accepted (M6). Date: 2026-11-23.
Builds on ADR 0033 (the quality pipeline), ADR 0035 (jobs, the text layer), ADR 0037 (review),
ADR 0051 (page objects traced to their bytes).

## Context

Spec §5.8 asks for two P1 features beyond M4:

- **Re-run OCR on a page or a selected region** with different settings.
- **Detect and replace a poor existing text layer**, for example one from another tool.

Arabic text layers from other tools are often wrong in ways a reader cannot see on screen,
because the layer is invisible:

- **Code-page mojibake:** Windows-1256 read as Windows-1252 shows as "ÇáÚÞÏ"; UTF-8 read as
  Latin-1 shows as "Ø§Ù„".
- **No Unicode mapping:** CID fonts without `/ToUnicode`.
- **Words in visual order:** letters last to first.
- **Words in the wrong order.**
- **Text that is simply not what the image shows.**

Search and copy return the garbage. Spec §8 also holds here: nothing is invented, and anything
uncertain stays flagged.

## Decision

### 1. Jobs have a kind (state store schema 6)

`ocr_jobs` gains an `options` column (JSON; NULL is the M4 behaviour):

- **`recognize`** (M4's job). With `replace: true`, the layers it writes take the place of
  another tool's text layer.
- **`region`**: an area of one page (`rect`: displayed points), with a `language` (automatic,
  Arabic, English) and `numbers` only.
- **`check`**: compare the pages' existing text layers with a quick draft. Writes nothing. Its
  verdicts are kept in `ocr_pages` with quality `check`, apart from page results.

They run in the same background runner as M4's jobs, with progress, pause, resume, cancel and
restart (ADR 0035): a region takes the models' servers, and they may need to start (up to 45 s).

### 2. Reading an area again

- **The read itself.**
  - **Larger than the page pass:** the area is rendered alone, with its long side at 1,600
    pixels, within 300–600 dpi and 12 megapixels.
  - **Every word re-read:** the quality pipeline's threshold is set above 1, so the 0.8B
    re-reads every strip and the 4B re-reads every strip too. Accuracy beats speed, and an area
    is small.
  - **Words placed:** they get boxes from the draft reader's lines (ADR 0035 §3), then move to
    where the area is on the page.
- **Language and "numbers only" constrain which reading a word takes, never the prompt.**
  - Nuqta's prompt is the one measured on the evaluation set (ADR 0030); a "digits only" prompt
    is unmeasured and could make a VLM write digits it does not see.
  - **A word that does not fit** takes the single fitting reading another engine gave, and is
    marked `corrected` (still listed).
  - **With no fitting reading, or several,** the word keeps its reading and becomes
    `uncertain`.
  - **What fits:**
    - **Numbers:** digits (any script), and `. , / - – : + ( ) % #` and Arabic separators.
    - **Arabic:** no Latin letters.
    - **English:** no Arabic letters.
  - **A forced language** also sets the lines' direction.
- **The result takes the area's place.** The page as the document shows it (ADR 0037's
  resolution) loses the words whose centres are in the area. The area's lines go:
  - where the first line cut was;
  - before or after what is left of a partly cut line, by which side the area is on in reading
    order;
  - or, with no line cut, by height.

  Words keep their decisions. The merged result is the region job's result for the page, so it
  is the newest state, and undoing it brings the old one back (ADR 0037).
- **Refused:**
  - a page converted to page text (`ocr_region_converted`);
  - a page with a layer of ours that the review has no record of (`ocr_region_unknown_layer`):
    merging would drop its words;
  - a page with only a draft layer (`ocr_region_draft`): it is still being read, and a draft
    keeps no words to merge with.

### 3. Another tool's text is removed by byte range (`objects::invisible_text_update`)

- **What counts.** Text drawn in render mode 3 or 7 (invisible or clip-only), outside our own
  layer and converted streams, and only text with glyphs.
- **How it goes.**
  - **Traced:** by the page-object tracer (ADR 0051), which already decrypts encrypted files.
  - **Removed:** run by run, by the same splice the object editor uses to delete text. A run
    leaves a `Tm` and a no-glyph `[n] TJ` behind, so text after it in the same text object
    stays put.
  - **Scope:** the whole page (`Clear::Page`), or the glyphs whose centres are in an area
    (`Clear::Region`).
- **Where it happens.** `PageLayer::clear` says what a layer replaces. `Document::with_text_layers`
  removes that text first, in the same edit, then writes our layer. Undo restores both.
- **When.**
  - A region job always clears its area.
  - A replacing job clears the page with its draft layer already, since a draft beats garbage.
  - Corrections clear nothing.

### 4. Checking a layer (`ocr::check`)

- **Which pages.** Only a page whose text is all invisible and not ours is judged
  (`LayerKind::Invisible`): a scan with another tool's layer.
  - Pages with no text, our layer, or visible text are listed as not checked.
  - Visible text is the page's own content: OCR can add a layer to it, never replace it.
- **From the text alone:**
  - **No Unicode mapping:** over 10% of characters are U+FFFD, private-use or control
    characters, or there is almost no text.
  - **Mojibake:** the characters map back to Windows-1252 bytes that decode, as UTF-8 or
    Windows-1256, into at least 60% Arabic letters, from text with almost no Arabic and over a
    third Latin-1 letters.
  - **Reversed Arabic:** the article and word-final letters on the wrong ends. There are at
    least 3 words with "ال" at the end, or ة or ى at the start, and over twice those the right
    way round.
  - **Broken mapping:** this app's extractor already finds the glyph mapping suspect (ADR 0008).
- **Against a quick draft of the image** (Apple Vision; Tesseract elsewhere), at 200 dpi.
  - **The measure.** Words are folded:
    - NFKC;
    - no tashkeel or tatweel;
    - alef and yeh forms unified;
    - digits as ASCII;
    - lower case;
    - punctuation gone.

    The Dice coefficient of their character trigrams is the similarity.
  - **Mismatch:** under 0.3, with nothing else found.
  - **Misordered:** 0.5 or more, with the longest common subsequence of the shared words under
    60% of them.
  - **Confirmed by the image:** mojibake and reversal when the redecoded or flipped text is at
    least 0.2 more similar.
  - The draft's own errors (Vision's Arabic is ~10% CER, ADR 0030) are why the thresholds are
    wide.
- **The panel** lists the poor pages, with:
  - what is wrong with them;
  - how much they are like the page;
  - the start of what they say (in `<bdi>`).

  "Replace" queues a replacing job for them (`ocr.checkLayers`, `ocr.replaceLayers`).

### 5. The UI

- **Drawing an area.** `ocr.drawRegion` gives the viewer a `region` pointer mode: the redaction
  tool's drag, with its own mark.
- **Its options.** The panel offers the language as three toggles and "Numbers only" as a check
  box.
- **Reading it.** `ocr.readRegion` queues the job.
- **The jobs list** names what each job does: "Reading an area of page 3…", "Checking the text
  layer of page 3…".

## Evidence

- **`ocr::region` unit tests:**
  - numbers only chooses among readings and never invents;
  - a forced language flags words of the other script;
  - small areas are read larger;
  - an area takes its words' place and the rest keep their decisions.
- **`ocr::check` unit tests**, Arabic, English and mixed:
  - good layers pass against a draft with OCR errors;
  - mojibake is found from the text alone, both Windows-1256 and UTF-8, and French is not
    mistaken for it;
  - reversed Arabic is found and confirmed by the image, and reversed English is never "reversed
    Arabic";
  - missing mappings, misordered words and mismatches;
  - only other tools' invisible layers are judged.
- **`ocr::jobs` tests,** through the real runner and state store:
  - an area read again replaces only its word, with a letter-holding reading flagged under
    "numbers only";
  - undoing it brings the old state back;
  - a check job keeps verdicts, writes nothing, and is not review data;
  - a replacing job's draft and final layers both clear the page.
- **`tests/ocr_review.rs`** (`another_tools_poor_layer_is_found_and_replaced`), on a real file:
  a scan with a Windows-1256 mojibake layer in Helvetica.
  - The check finds it.
  - Our layer replaces it, and the old text is gone from extraction.
  - The page renders the same, pixel for pixel.
  - An area clears only the old line inside it.
  - `qpdf --check` passes.
- **UI tests:**
  - an area drawn, read as Arabic numbers, its word replaced and flagged;
  - layers checked, a poor one described and replaced.

## Known limits

- **Text inside form XObjects** is not removed; nor is text in a text object that spans content
  streams.
- **Text hidden under an image** (drawn visibly, then covered) is not recognised as a layer; nor
  is visible text with a broken mapping, which OCR can only add a layer over.
- **Without a draft reader** (Windows and Linux without Tesseract), the check uses the text
  alone: mismatch and misorder are not found.
- **The thresholds** come from synthetic tests, not from real scans of other tools' layers
  (owner item 9: real scans).
