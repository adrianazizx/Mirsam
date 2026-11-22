# 0022 — The in-place editor's surface: our own, over a hidden text field

Status: Accepted (Milestone 3 spike). Date: 2026-11-22.

## Context

Spec §7 asks the in-place editor to draw "the editing caret, selection, and IME composition …
by our own editor surface over the canvas using the same layout engine as the writer, so what
the user sees while typing is exactly what gets saved". It marks the choice as a **[SPIKE]**:

- **A, a custom editor surface:** the engine lays the paragraph out, and the webview draws it
  and its caret.
- **B, a transparent native `contenteditable` overlay** kept in sync with the layout engine.

The spike was to evaluate caret fidelity, Arabic IME behaviour, and effort.

## Method

Throwaway prototypes run inside the real app's webview (`spikes/editor-surface/`), in WKWebView
on macOS, at a device pixel ratio of 2. The case is one paragraph 300 points wide, in IBM Plex
Sans and IBM Plex Sans Arabic, which the engine bundles and the UI also loads. Five texts were
used, each at 12 and 24 points, and justified at 12:

- English;
- Arabic;
- mixed (Arabic with "iPhone 15 Pro", numbers and a date);
- fully vocalized Arabic;
- lam-alef ligatures.

- **The engine** (`typeset`) returns the layout plus two new things the real editor will also
  use:
  - **caret stops** (`caret_stops`): for every grapheme boundary, the caret's x position on its
    line;
    - placed on the side of each cluster the run's direction puts it;
    - inside a ligature, its width is shared evenly between its letters;
    - never between a letter and its marks;
    - where two directions meet, both places are given;
  - **glyph outlines** (`glyph_outline`), read from the very font subsets written to the PDF.
- **A:** a canvas draws those outlines at the engine's positions, with the caret at its stop.
  A hidden text field takes input. Each keystroke sends the paragraph to the engine and draws
  the result; it is timed from the keystroke to the next frame. Outlines are cached by glyph,
  so each keystroke sends only new ones.
- **B:** a `contenteditable` paragraph, laid out and drawn by WebKit.
  - It has the same width, size and fonts.
  - It is split into one span per run of the engine's font choice, so the browser uses the
    same face for each letter.
  - Its caret position at every stop (`Range.getClientRects`) is compared with the engine's,
    as are its line breaks.
  - Its typing is timed the same way (`insertText`).

## Results (WKWebView, macOS arm64; raw numbers in the spike's results file)

| | A: our surface | B: `contenteditable` |
|---|---|---|
| Caret vs. the text as saved | exact: the same layout draws both | within 1 CSS px at 12 and 24 pt (mean 0.4–0.6 px, partly pixel rounding); worst 1.4 px (mixed, 12 pt) and 2.8 px (mixed, 24 pt) |
| Line breaks vs. the text as saved | exact | the same lines in all 15 cases (plain and justified) |
| Typing latency, keystroke to next frame | median 17 ms, p95 17–18 ms; also at 416 Arabic characters | median 17 ms, p95 19–42 ms |
| Engine time per keystroke | 0.4–1.9 ms (layout, subsetting, outlines; debug build) | — |
| Sent per keystroke | 8–71 KB (median 8–44 KB; outlines cached by glyph) | — |
| Typed input reaching the editor | `beforeinput` gives `insertText` for an Arabic letter and a fatha, and `deleteContentBackward` | native |

Both meet the typing budget (spec §11: 32 ms per keystroke), with a frame to spare. With the
UI's own fonts, B is close: WebKit shapes these fonts as harfrust does.

## Decision

**A: our own editor surface.** While a block is being edited:

- **The webview draws the engine's layout of it:** the glyph outlines of the fonts that will be
  written, at the positions that will be written. The caret, selection and composition
  underline are drawn from the caret stops.
- **A hidden text field holds the paragraph's text and caret** (the pattern of code editors
  that draw their own text):
  - it receives keyboard, IME and dictation input as `beforeinput` events;
  - it is placed at the caret, so input-method windows open there;
  - it gives clipboard events;
  - it gives screen readers the text and caret position.
- **The engine stays the only source of layout:** each change re-lays out the block (under 2 ms
  here), and outlines are cached by font and glyph.

Why not B, although its caret is within a pixel here:

- **In-place editing must reuse the PDF's own fonts** when they have the glyphs (spec §7.4), and
  a webview cannot draw those:
  - embedded fonts are addressed by glyph number (Identity-H);
  - subsets need not map the text's characters at all;
  - CFF without an OpenType wrapper and Type 3 fonts cannot be loaded by the browser.

  A draws any font the engine can read, by glyph id.
- **B's agreement holds only while the browser's choices happen to match the engine's:** its own
  shaping, line breaking, justification and fallback. Fonts other than the UI's, kashida (P2),
  and our reflow rules would each need B forced into line, and every mismatch shows as a caret
  away from the text. In A, what is drawn is what is saved, by construction.
- **B's apparent saving is small:** it still has to map the DOM selection to model offsets,
  keep the paragraph's DOM in step with the engine's runs, and stop the browser from editing
  the DOM its own way.

## Consequences

- **The editor surface is ours to build:**
  - caret movement: logical, with visual arrows in bidi text, word and line moves, Home and End;
  - hit-testing: from caret stops;
  - selection drawing across directions;
  - IME composition shown inline;
  - undo grouping;
  - the hidden field's text kept in step with the model, for accessibility.

  `caret_stops` and `glyph_outline` are its first pieces and stay in `typeset`.
- **Outlines need fonts the engine reads.** TrueType `glyf` today. CFF outlines, needed for
  OpenType-CFF and many embedded fonts, come with in-place editing; Type 3 glyphs would be drawn
  from their content streams.
- **Measured on WebKit/macOS only.**
  - WebView2 (Windows) and WebKitGTK (Linux) should be re-run (`spikes/editor-surface/README.md`).
  - Real input methods cannot be automated: the Arabic keyboards of macOS and Windows,
    press-and-hold accents, the emoji picker, and dictation. They go to the owner as a manual
    check once the surface exists.
- **The add-text tool (ADR 0021) still types into the shared note editor.** It moves to this
  surface with in-place editing, so new text, too, is seen exactly as it will be saved.

## Verified

- **Unit tests** (`typeset::editing`):
  - caret stops follow each run's direction in mixed text;
  - a letter and its shadda are one stop;
  - where directions meet, a stop has two places;
  - lam-alef's width is shared by its two letters;
  - outlines of a subset are closed contours for every non-space glyph.
- **The spike's report** (above) was produced by `SAFHA_SELFTEST_SPIKE=editor-surface` in the
  real app.
