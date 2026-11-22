# 0019 — Text boxes, and writing shaped text into PDF

Status: Accepted (Milestone 2). Date: 2026-11-22.

## Context

Spec §5.2 (P0) asks for text boxes with right-to-left-aware text entry. Spec §6.4: text boxes,
notes, stamps, and typed signatures all go through the same shaping pipeline.

A text box (`/FreeText`) draws its text on the page. Unlike a note (ADR 0018), its appearance
stream must hold the text itself:

- shaped (Arabic joined, marks placed);
- in visual order;
- with a font embedded, since viewers have none of their own for Arabic.

Other viewers' `/DA`-based redraws cannot be relied on for Arabic.

The Milestone 3 groundwork (ADR 0020, merged into this branch) provides the parts:

- `typeset::layout`: bidi, script and font itemization, shaping with harfrust, UAX #14
  breaking, and per-line positioning;
- `typeset::subset_fonts`.

Its test writer proved a recipe that PDFium draws and our extractor reads back in logical order.
ADR 0005's amendment decided that text is written by our own content-stream writer.

## Decision

### The text writer (`pdf::text`, for the product)

- **`draw_text`** turns a layout into content-stream operators:
  - one `BT … ET` per glyph run, each glyph placed with `Tm` and shown with `Tj`;
  - a cluster drawn with several glyphs (a letter and its separate dots) is wrapped in
    `/ActualText` marked content.

  It also returns the font subsets to embed (resource names `F0`, `F1`, …).
- **`write_font`** writes a subset's five objects:
  - the font program (`/FontFile2`, Flate-compressed);
  - the font descriptor;
  - a CIDFontType2 with `/W` widths and an identity `/CIDToGIDMap`;
  - a ToUnicode CMap for glyphs that draw a whole cluster;
  - the Type 0 font with Identity-H.

  `/BaseFont` is a subset tag plus the font's PostScript name. In an encrypted file the streams
  and the strings (`/CIDSystemInfo`) are encrypted under each object's number.

This is the text half of the content-stream writer. Writing into page content (M3's add-text
and in-place editing) will reuse it.

### Text boxes (`annotate::text_box`)

- **Fonts:** IBM Plex Sans (regular, instanced from the variable font) and IBM Plex Sans Arabic,
  the UI's own families. Both are bundled into the engine (`include_bytes!`, about 770 KB, OFL).
  - The text's own script comes first: Arabic first when the first strong character is right to
    left.
  - Characters the first font lacks fall back to the other.
- **Layout:**
  - wrapped to the box's width inside 2 points of padding;
  - aligned to the start of the text's own direction (auto: the first strong character);
  - 12 points, black, for now.
- **Size:** the width is chosen (240 points, or what the page allows); the height always fits
  the text.
- **The appearance** has `/BBox` `[0 0 w h]` (the box as displayed) and, on a turned page, a
  `/Matrix` that turns it back by the page's `/Rotate`. The text then stands upright as
  displayed, as a note's icon does.
- **The annotation** has:
  - `/Contents` (the text);
  - `/DA` (`/Helv 12 Tf 0 0 0 rg`, for viewers that redraw the box themselves; size and colour
    are read back from it);
  - `/Q` (right-aligned for right-to-left text);
  - no border.
- **Changing the text** (`SetContents`):
  - sets the text again at the box's width;
  - fits its height, keeping the displayed top edge, whatever the page's rotation;
  - writes a new appearance with new font subsets;
  - rewrites the annotation under its own number.

  In an encrypted file its `/DA` is decrypted to read the size and colour.
- **In the UI:**
  - the Comment strip gains **Add text box**: a click places the box's corner there, and the
    shared editor (ADR 0018) takes the text;
  - a dashed outline shows where the box goes;
  - clicking a text box opens it;
  - saving it empty removes it.

### Extraction: word spaces inside form XObjects

The extractor's word-space markers (`collect_space_marks`, ADR 0008) now also look inside form
XObjects, nested ones too. Each child's bounds are mapped through the forms' matrices to page
space.

- **Why:** flattened annotations, and producers that wrap content in forms, draw their word
  spaces there. Arabic lost its spaces otherwise: the flattened text boxes' Arabic read back
  without them.
- **The corpus is unchanged** at 86 of 118 blocks verbatim. No fixture draws its spaces inside
  a form.

## Consequences

- **The engine binary grows by the two fonts.** A font choice, other fonts (Naskh, the user's
  own), sizes and colours come with the inspector.
- **Not written into the page.** A text box's text is not part of the page's text, as in other
  viewers: search and selection do not see it. Flattening (P1) would make it page content.
- **Paragraph direction is not stored in PDF.** A single line with more Latin than Arabic
  letters, starting with Arabic, is read back by text extractors (ours included) as a
  left-to-right paragraph. This is the reader's guess and applies to any PDF; the box itself is
  drawn right to left.
- **Editing** is in the shared editor, not on the page itself. Typing directly into the box is
  M3's editor-surface spike.
- **"Plex" is a Reserved Font Name.** Subsets of IBM Plex fonts are now embedded in the user's
  PDFs. Embedding subsets in documents is common practice, and the font keeps its name, but
  `assets/fonts/README.md` asked for a re-check when this landed. It is listed for the owner.

## Verified

- **Unit tests:**
  - the subset tag;
  - the ToUnicode CMap;
  - text box heights (longer text, Arabic);
  - the font order by script, and `/Q`;
  - two embedded fonts for mixed text;
  - the box and matrix on a turned page;
  - resizing that keeps the displayed top edge at each rotation;
  - reading `/DA` back;
  - refusal of empty or cramped boxes.
- **`tests/text_boxes.rs`.** Each result is rendered by PDFium, then flattened into the page with
  `qpdf --flatten-annotations` and read back with our extractor. These checks cover shaping,
  subsets, and ToUnicode together:
  - **Three boxes** (English, Arabic with Arabic-Indic digits, mixed): each draws only in its
    box, and the page reads back its own text followed by the boxes' text exactly, in logical
    order.
  - **A long Arabic text** wraps to its width.
  - **Turned pages** (0°, 90°, 180°, 270°): the box is drawn where it was placed, and its pixels
    match the unturned page's, so it is upright.
  - **Changing the text:**
    - the same annotation is rewritten;
    - the top edge and width stay, and the box grows;
    - the new text reads back;
    - undo restores the old one.
  - **An AES-256 encrypted file:**
    - a box is added, and its text changed (its `/DA` decrypted);
    - the encrypted fonts and text read back after decryption and flattening.
  - **Saving** (a full PDFium rewrite) keeps the box and its fonts.
- **UI tests:**
  - a text box placed at the press, as wide as the page allows;
  - placed, saved in Arabic, rewritten, removed when emptied, undone.
- **Browser preview, Arabic UI:**
  - the strip's four modes, mirrored;
  - a text box placed, typed in right to left, and saved;
  - the pending box drawn as a dashed outline. It was a yellow fill before; fixed.
- **Real-app self-test, English and Arabic (right to left, dark):**
  - an Arabic text box is painted: 1,055 px in English, 232 px for the Arabic book;
  - its text reads back exactly;
  - undo removes it.
