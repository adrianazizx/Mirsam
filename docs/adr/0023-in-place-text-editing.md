# 0023 — In-place text editing: page analysis, the edit model, and write-back

Status: Accepted (Milestone 3). Date: 2026-11-22.

## Context

Spec §7: PDF has no paragraphs, only positioned glyph runs. Editing text means rebuilding
structure, laying it out again, and rewriting the content. ADR 0005's amendment ruled out
PDFium regenerating content (it damages untouched content on 27 of 34 test pages), so the
removal of old text and the write-back are done by our own content-stream code, as
incremental updates (ADR 0014). ADR 0021 wrote new text into pages; ADR 0022 chose the editor
surface (a native text field).

## Decision

### 1. Reading a page's content (`pdf::content`, `pdf::fontinfo`)

- **A tokenizer** keeps every operator's byte range in its (decoded) content stream:
  octal escapes, hex strings, nested arrays and dictionaries, inline images. Bounded for
  untrusted input (operators, nesting, operand stack).
- **An interpreter** follows `q Q cm BT ET Tf Tc Tw Tz TL Ts Tr Td TD Tm T* Tj TJ ' "`, fill
  colour, and marked content. For each text-showing operator it records the font, size,
  matrix, text matrix, colour, each glyph's origin and advance, and the text each glyph
  stands for, with `/ActualText` spans giving their text to the span's first glyph.
- **Font reading** is from the font dictionary only: `/ToUnicode` (bfchar, bfrange), or a
  simple font's encoding (WinAnsi, `/Differences` with standard glyph names); `/Widths` or
  `/W`; descriptor flags and name for serif, bold, italic. Simple fonts and Type 0 Identity-H
  are read; anything else (Type 3, other CMaps, vertical writing) is not editable.
- Not read: text in form XObjects; encrypted files (refused: their streams would need
  decrypting and re-encrypting under their own numbers); streams with filters other than
  Flate.

### 2. Lines → blocks (`blocks`)

Everything in the page as displayed (the view transform applied), y down. Only text that is
upright on the displayed page is read: turned pages work, rotated text does not.

1. **Glyphs → lines:** one baseline within 0.3 em; split where the gap exceeds 2 em (columns,
   table cells); a space where the gap exceeds 0.2 em.
2. **Logical order:** the visual glyph sequence becomes logical text from the glyphs' own
   directions: Arabic/Hebrew runs reversed, Latin and digit runs kept, neutrals resolved from
   their neighbours, runs reversed when the line is right to left, marks kept on their
   letters. A line's direction is the majority of its letters.
3. **Lines → paragraphs:** same font, size within 12%, baselines one pitch apart (as the
   paragraph set it), overlapping and sharing an edge or centre (or an indented first line).
4. **Attributes:** direction (letter majority), alignment (flush left / right / centred /
   justified from the lines' edges), size, colour, line pitch.
5. **Flags:** an operator that drew text of two paragraphs makes both uneditable; so does
   text that cannot be read exactly (dotless letters drawn with separate dots, CoreText's
   Arabic, whose glyphs carry no text for the dots). Text we cannot read is never offered.
   Mixed fonts, sizes or colours flatten into the dominant one (the UI says so).

Not editable text still takes room: its box is an obstacle that single-line growth keeps off.

### 3. Editing a paragraph (`block_edit`)

An edit names the block by index and a fingerprint of its text and position; the page is read
again at edit time and a block that changed is refused.

- **Removal:** by byte range, per text-showing operator.
  - If a later operator of the same text object remains, it depended on the removed one's
    advance: a `[n] TJ` that moves the same distance replaces it. `'` and `"` always keep
    their line move (`T*`).
  - A text object holding only this paragraph, in a font this app wrote, goes whole (`BT … ET`
    sets its own state): the font can then drop out of the page. Other producers' `BT … ET`
    stay (they set state later text may use).
  - Every other byte of the content is unchanged.
- **Placement and z-order:** the new text goes where the first removed operator's `BT` was,
  so what was drawn after it still covers it, and the clip in force still clips it.
  It is drawn under one `cm` that cancels the matrix in force there and places the box on the
  page as displayed (ADR 0021's placement). The paragraph's own first baseline is kept.
- **Box:** the paragraph's own width (its longest line), so lines that fit still fit and the
  wrap of unchanged text is stable; it grows downward. A single line may also grow along its
  reading direction to the page margin or the next text.
- **Fonts, per paragraph:**
  - **Reuse** the paragraph's own font when every character is Latin (no shaping needed) and
    the font has a code for each (reverse `/ToUnicode`), including the space: words wrapped
    greedily by the font's own widths, aligned or justified with `TJ` adjustments; the same
    font resource, nothing embedded. No kerning (the original's kerning is lost).
  - **Substitute** otherwise, for the whole paragraph: the text engine (`typeset`) lays it out
    in a bundled family (Naskh if the original was a serif, else Sans), embedded as a subset.
    The UI says "Font substituted" and offers Sans or Naskh. Arabic always takes this path:
    embedded Arabic subsets hold letter forms, not the letters, so they cannot be shaped.
- **Our own fonts no stream uses any more** drop out of the page's `/Resources` (named
  `MT…`), so edit after edit does not pile up subsets. The dead objects leave the file at
  the next save (PDFium's full rewrite).
- **An empty text removes the paragraph.** One undo step (`Change::Content`).

## Consequences

- **Honest limits (v1):**
  - bold, italic and weight are not kept by the substitute (the bundled families are regular);
  - first-line indents, lists' hanging indents and tab stops are not kept;
  - paragraphs with mixed styles become one style;
  - kerning is lost in reused fonts;
  - text in forms, encrypted files, and Type 3 or CMap-encoded fonts is not edited;
  - a paragraph that grows can overlap text below it (no reflow across blocks, spec §7);
  - direction is the majority of letters, so a short Arabic sentence with a long English
    address can be read as left to right.
- **Installed fonts** and `match_font` (ADR 0020) are not used for substitution yet: the
  picker offers the bundled families only. That belongs with the font picker.
- Inline `/ActualText` marked content and `/MirsamText` marks our own writing leaves read
  back like any other text: our own paragraphs can be edited again, with no duplicate fonts.

## Verified

`tests/block_edit.rs` (PDFium renders, our extractor reads back, `qpdf --check` passes):

- **English typo:** the paragraph's own font is reused (no font embedded in the update); the
  page's other lines are pixel-identical and keep their positions; the line starts where it
  did.
- **Wrap stability:** the same text again keeps two lines and the width within 3 pt; longer
  text grows downward inside the same width.
- **Arabic insert:** a word inserted in the middle of a right-to-left paragraph of several
  lines re-wraps right to left, keeps the first line's place and the right edge, and reads
  back in logical order; the old subset is gone.
- **Mixed sentence** (Arabic, a product name, numbers): read back in logical order and
  edited without disturbing the order.
- **Round trip:** edit, save (full rewrite), reopen, edit again: no drift (< 0.6 pt) and no
  extra fonts.
- **Turned pages** (0°, 90°, 180°, 270°): edited as displayed; nothing outside the paragraph
  changes.
- **Z-order and clip, on a page that scales without `q … Q`:** a rectangle drawn afterwards
  still covers the new text, and the clip still clips it.
- **`TJ` kerning, `'`, and a later operator that depended on a removed one's advance:** the
  other paragraph does not move.
- **Refusals:** a changed block, an out-of-range block, an encrypted file, CoreText Arabic.
- **Real app, English and Arabic:** an Arabic paragraph is found as one block in logical
  order, edited through the Edit tool's commands, read back, and undone.
- Spikes: `spikes/editor-surface/` (ADR 0022).
