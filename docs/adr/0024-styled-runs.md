# 0024 — Styled runs in a paragraph edited in place

Status: Accepted (Milestone 3). Date: 2026-11-22.

## Context

Spec §7, step 2: "each block becomes an editable rich-text model as styled runs in logical
order". ADR 0023 rebuilt paragraphs with one font, size and colour (the dominant ones) and
listed among its honest limits:
- bold, italic and weight were not kept;
- paragraphs with mixed styles became one style.

A bold word, a red amount or an italic title in a contract paragraph was lost on the first
edit, and only a note said so.

The editor is a native text field (ADR 0022). It edits plain text, so styles must survive the
edit without the user restating them.

## Decision

### Reading: every character keeps its style (`blocks`)

- **A run's style** is its font (a key into the page's fonts), its size as displayed, and its
  fill colour. Bold and italic come with the font: `/FontDescriptor` flags and the name
  (ADR 0023's font reader).
- **The visual-to-logical reorder** carries each character's style through. A space takes the
  style of the character before it in logical order, as typing would. A line's runs are joined
  into the paragraph's with the line-ending space in its line's last style.
- **The UI** gets the runs with UTF-16 offsets, font, bold, italic, size and colour. A
  paragraph with more than one style is flagged `styled`.
- **Grouping lines into paragraphs** used to require the same dominant font. It now needs one
  font in common. A line of a styled paragraph that is mostly italic stays in it. A bold
  heading over body text shares no font and still stands alone.

### Editing: the old styles mapped onto the new text (`restyle`)

The field returns plain text. The characters the edit kept are found by a longest common
subsequence of characters: a table of at most 4 million cells after the common prefix and
suffix are cut, and past that the middle counts as replaced. Then:

- a character the edit kept keeps its style;
- typed text that replaced some takes the first replaced character's style: typing over a bold
  word stays bold;
- other typed text takes the style of the character before it, or the one after it at the start
  of the paragraph.

### Writing: each run as it was

- **The PDF's own fonts:** when every run's own font has codes for its text, each run is written
  in its font. Font, size and colour switch inside the line (`Tf`, `rg`), and words wrap
  greedily by each font's widths. Justification adjusts at the spaces, in the space's own
  font size. Nothing is embedded.
- **Otherwise, bundled faces:**
  - Each original font gets its closest bundled face, chosen by the ADR 0023 amendment's
    matching on that font's own runs (or the family the user chose). Its runs get that face
    at their weight.
  - New bold faces: IBM Plex Sans and the Noto variable fonts are instanced at weight 700, and
    Amiri Bold and IBM Plex Sans Arabic SemiBold are bundled (both OFL, already in
    `assets/fonts/`).
  - Each run is a layout span with its own size, and fallback faces of its own weight:
    `typeset::Span::fallbacks`, so Arabic inside a bold Latin run is bold too.
  - A span starts a new style: a digit after a bold word is no longer drawn in the bold word's
    font.
- **Italic:** no italic face is bundled, so a run is slanted 12° in the drawing (`pdf::text`'s
  paints). It is written under an oblique name (`IBMPlexSans-Oblique`, …), so the page reads
  back italic and the next edit slants it again.
- **Colour** is per run, in both paths.

## Consequences

- **Fixed:**
  - typing over or beside a styled word gives what a word processor would;
  - bold, italic, size and colour survive any number of edits;
  - the editor's note now says styles are kept and typed text takes the style before it.
- **Still lost or approximate:**
  - weights other than regular and bold become the nearer of the two;
  - a slant stands in for a real italic in bundled faces;
  - underline and strikethrough are drawn as paths, which the analysis does not read;
  - kerning in reused fonts (as before);
  - the user cannot yet change a run's style while editing (since ADR 0025: bold and italic
    can be set).
- **Size:** the app grows by about 650 KB of bundled fonts (Amiri Bold, Plex Sans Arabic
  SemiBold). The variable fonts' bold instances share their data.

## Verified

- **Unit tests:**
  - runs in logical order through right-to-left reordering;
  - spaces taking the style before them;
  - lines' runs joined into the paragraph's;
  - the restyling rules (typed after, inside, over, at the start; deletions; Arabic by
    characters).
- **`tests/block_edit.rs`, on a page set in Helvetica, Helvetica-Bold and Helvetica-Oblique
  with one red word over two lines:**
  - the runs read as drawn;
  - an edit in the page's own fonts keeps the bold, italic and red runs, embeds nothing, and
    the paragraph stays one;
  - the same paragraph substituted (Sans) keeps them in IBM Plex Sans Bold, an oblique and
    red, and keeps them on a second edit;
  - Arabic typed over the bold word is set in a bold Arabic face.
- ADR 0023's tests all still pass: typo, re-wrap, Arabic insert, mixed sentence, round trip,
  turned pages, z-order and clip, kerning.
