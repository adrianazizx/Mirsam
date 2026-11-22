# 0021 — Writing text into page content, and the add-text tool

Status: Accepted (Milestone 3). Date: 2026-11-22.

## Context

Spec §5.3 (P0): "Add new text (any font, size, colour, alignment, direction)". Spec §7's write-back
step: "emit new ones (section 6.3 rules); keep z-order and clipping; update font resources".

The pieces exist:

- the layout engine (`typeset`, ADR 0020);
- the text half of the content-stream writer (`pdf::text`, ADR 0019), so far used only for text
  box appearances;
- incremental updates with undo (ADR 0014), and encryption of new objects (ADR 0017).

ADR 0005's amendment ruled out PDFium regenerating content: new text is written by our own
writer as incremental updates.

Text written into the page itself (not an annotation) prints, extracts and searches as the page's
own text in every viewer. It is also the write half of in-place editing, which comes later in M3.

## Decision

### Writing (`text_edit::add_text_update`, `Document::with_new_text`)

- **Placement:**
  - the UI gives the box's top-left corner and width on the page as displayed;
  - the engine lays the text out to that width (font fallback, shaping, UAX #14 breaking,
    alignment, paragraph direction);
  - the box's height is what the text needs.
- **Its own content stream**, appended to the page's `/Contents`:
  - drawn inside `q … Q`, under a `cm` that maps the box's top-left corner (y up) to page
    space;
  - so it stands upright on a turned page, as displayed.
- **The page's original content is wrapped once.** A `q` stream goes before it and a `Q` stream
  after it, both with a `/MirsamWrap` key in their dictionaries.
  - A page whose content leaves the graphics state changed (an unbalanced `cm`, which real
    producers write) cannot move or scale the new text.
  - Later additions find the wrapper and only append.
- **Resources:** the page gets a `/Resources` dictionary of its own. It holds what the page used
  (its own, or inherited from the page tree) plus the new fonts, under names it does not use yet
  (`MT0`, `MT1`, …). Other pages that shared the old resources keep them unchanged.
- **Fonts:** Type 0 / CIDFontType2 subsets with Identity-H, ToUnicode, and `/ActualText` for
  clusters drawn with several glyphs (ADR 0019's writer).
- **Marking:** the drawing is wrapped in `/MirsamText` marked content, so in-place editing can
  find the blocks this app wrote.
- **Encrypted files:**
  - new streams and fonts are encrypted under their own numbers;
  - copied resources that hold strings would no longer decrypt under the page's number, so such
    a page is refused (`edit_unsupported`).
- **History:** a new change kind, `Change::Content`: the same pages at the same sizes, painted
  again, with their text read again. The shell's edit outcome gains `textChanged`. The UI then
  repaints in place (as for annotations), drops its page-text layers and selection, and clears
  stale search results.

### Bundled fonts (`fonts`)

Two families, compiled into the engine (OFL, `assets/fonts/`):

- **Sans:** IBM Plex Sans (weight 400) with IBM Plex Sans Arabic, ADR 0019's pair. The text's
  own script's face goes first.
- **Naskh:** Amiri, a classical Naskh with matching Latin. Plex fills any gap.

Text boxes now take their faces from the same module. Installed system fonts (`FontLibrary`,
ADR 0020) come with the font picker.

### The add-text tool (UI)

- **The Edit tool** places new page text: a press puts the box's corner there, and the shared
  editor takes the text (Cmd/Ctrl+Enter saves; pressing elsewhere keeps it). The field follows
  the text's own direction. A dashed outline shows where the text goes.
- **A strip above the page** sets the text added next:
  - font (Sans, Naskh);
  - size (8–72 pt);
  - colour: four swatches. These are document colours, kept as data rather than design tokens;
  - alignment (start, centre, end, justify). The start and end icons mirror in Arabic;
  - direction (from the text, right to left, left to right). These icons show physical
    directions and do not mirror.
- **Every choice is a command:** `text.place`, `text.setStyle`.

### Extraction: ligatures whose letters cannot tell their order

IBM Plex Sans Arabic draws noon-yeh ("ني") as one ligature glyph. Its ToUnicode mapping is in
logical order, and PDFium reports the letters reversed ("ين"), as it does for Chrome's pages.
Our extractor puts a multi-letter glyph's letters back by the joining rules. Both of these
letters join both ways, so they carry no evidence, and the page's other glyphs vote (ADR 0008).

With no votes at all, the extractor used to keep PDFium's order, which is wrong on our pages and
Chrome's. **It now reverses** when nothing on the page argues for keeping the order. The corpus
is unchanged (86 of 118 blocks verbatim), and pages that deliver ligature letters in order still
vote for it.

Labelling such glyphs with `/ActualText` instead was tried. PDFium reports `/ActualText` letters
reversed too, and inside form XObjects (flattened text boxes) it misplaced them. It was dropped.

## Consequences

- **Not editable once added.** New text is page content, so changing it later is in-place
  editing (the next M3 steps, after the editor-surface spike).
  - The `/MirsamText` marking is there for that work.
  - The marking does not yet record the source text or style: the edit model will read them
    back from the content.
- **What you see while typing is the shared editor, not the final layout.** The page shows the
  text once added. Drawing the caret and selection with our own layout is the editor-surface
  spike's question (spec §7).
- **Each addition has its own fonts.** Two additions in the same family embed two subsets.
  Reusing an earlier subset, or the PDF's own fonts, belongs with in-place editing. (That is
  spec §7's "no duplicate fonts" acceptance test.)
- **Copied resources:** a page that shared resources with other pages now has its own copy (a
  few hundred bytes).
- **Paragraph direction is not stored in PDF.** A single line with more Latin than Arabic
  letters reads back as left to right in any extractor (ADR 0019).

## Verified

- **Unit tests:**
  - resource names never collide;
  - empty, oversized, misplaced and miscoloured text is refused;
  - the noon-yeh ligature reads back in logical order with nothing else on the page.
- **`tests/added_text.rs`** (rendered by PDFium, read back by our extractor, `qpdf --check`):
  - English, Arabic (Naskh) and mixed text: each draws only in its box, and the page then reads
    its own text followed by each addition, exactly, in logical order. No flattening is needed.
  - Justified Arabic wraps inside its width and reads back.
  - Turned pages (0°, 90°, 180°, 270°): drawn where placed, upright, pixel-matching the
    unturned page.
  - A page whose content scales the graphics state without `q … Q`, and whose font is inherited
    from the page tree: the new text lands exactly in its box. A second addition finds the
    wrapper and does not wrap again.
  - An AES-256 encrypted file: drawn, read back, and `qpdf --check` passes with the password.
  - Undo returns the original bytes; saving (a full rewrite) keeps the text.
  - A page out of range is refused.
- **Shell test:** the UI's shape of new text (family, alignment, direction) is parsed.
- **UI tests:**
  - placing and saving sends the chosen style;
  - empty text adds nothing;
  - a content change repaints and clears stale search results;
  - the strip works in English and Arabic.
- **Real-app self-test,** English and Arabic (right to left, dark): Arabic text placed with the
  Edit tool is painted (1,161–1,164 px), reads back as page text exactly, and undo removes it.
