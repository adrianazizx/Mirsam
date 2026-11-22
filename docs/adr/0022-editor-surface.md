# 0022 — The in-place editor surface: a native text field, not our own caret

Status: Accepted (Milestone 3, spike). Date: 2026-11-22. Measurements: `spikes/editor-surface/`.

## Context

Spec §7 asks for a spike before in-place editing: draw the caret, selection and IME
composition ourselves over the page (using the writer's layout engine), or put a transparent
native `contenteditable` over the block, synchronized with the layout engine. Evaluate caret
fidelity, Arabic IME behaviour, and effort.

## Measurements

Five paragraphs (English, Arabic, mixed, Arabic with full diacritics, justified Arabic) at
300 pt, laid out by `typeset` and by the webview's own layout of the same text in the same
fonts (`spikes/editor-surface/README.md`):

- **Line breaks:** identical in 5 of 5 paragraphs.
- **Character positions:** within 0.01 pt in pure English and pure Arabic, justified or not.
  In the mixed paragraph some Arabic letters sit 1.1 pt (0.09 em) from the engine's after a
  Latin or digit run. Trailing spaces hang in the engine and have no box in the browser;
  marks have no advance in the engine.
- **Line height:** differs by default (16 px vs 15.6 pt); settable.
- **Not measured:** real IMEs (none to drive here); WKWebView (Chromium was measured);
  the custom surface's effort (estimated).

## Decision

**A native text field (the shared editor's `textarea`, already used for new text) is the
editing surface, anchored at the block.** The browser owns the caret, selection, bidi caret movement,
composition, accessibility, and system services; we do not draw them.

- **The engine's layout stays the truth for what is saved.** The page is repainted from the
  PDF after every commit, so a 1 pt difference in the field while typing is not saved.
- **What is built (step 5):** the shared note editor's `textarea`, anchored at the paragraph,
  with the paragraph's direction (`dir`) and its font indicator. It is a text input, not a
  WYSIWYG overlay. Setting the field in the block's own width, size and line height, so it
  sits over the paragraph as the engine will lay it out, is the next step and is where the
  measurement above applies.
- **Why not our own surface:**
  - effort: hit-testing, bidi-aware caret movement and selection across runs and lines, word
    and line movement, composition underlines, IME candidate window placement, and
    accessibility are each a feature to write and to keep correct in two directions;
  - the measurement shows the native layout already follows the engine within a hundredth of
    a point on pure text;
  - Arabic keyboards on macOS and Windows deliver final characters (no composition), so the
    IME case that justifies a custom surface (CJK-style composition) is not the one we have.
- **What would change this:** a divergence the user can see (mixed text drifting more than
  about 2 pt, line breaks differing, justification), or WKWebView behaving unlike Chromium.
  Comparing the engine's line count with the field's after each edit would detect it; **that
  check is not built yet** (follow-up). The drawn-caret surface is the fallback, not started.

## Consequences

- No caret or selection code is written for M3. The editor is the shared `textarea`.
- Still to check in the real app (WKWebView) with an Arabic keyboard: the field's line breaks
  against the engine's. The real-app self-test edits an Arabic paragraph through the editor's
  commands but does not look at the field's own layout.
- What the field shows while typing is the browser's layout of the text in the substitute
  family's metrics only if the font is loaded in the webview; the UI uses its own UI fonts, so
  the field is a text input, not a preview. The page repaints from the PDF on commit.
- Kashida justification and per-cluster font fallback inside the field are not shown by the
  browser; they appear on commit (the page is repainted from the PDF).
