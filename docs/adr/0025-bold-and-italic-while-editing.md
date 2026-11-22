# 0025 — Bold and italic while editing in place

Status: Accepted (Milestone 3). Date: 2026-11-22.

## Context

ADR 0024 keeps a paragraph's styled runs through an edit. But the editor (ADR 0022) is a plain
`textarea`: the user could not make a word bold or italic, nor see which words are. Spec §7
asks for a rich-text edit model.

## Decision

### The field: the browser's own rich-text editing, held to bold and italic

- **The paragraph editor's field is a `contenteditable` element** (`StyledField`). Notes and
  text boxes keep the `textarea`.
- This stays within ADR 0022: the browser still owns the caret, selection, bidi movement, IME
  composition, accessibility, and now undo of formatting too.
- **It holds only text, `<br>` line breaks, and bold and italic:**
  - Enter inserts a line break, not a block element;
  - pasted text arrives plain;
  - drops are refused.
- **Reading it back** (`richText.readField`) gives the text and bold or italic marks in UTF-16
  offsets.
  - Each element either says (`<b>`, `<strong>`, `<i>`, `<em>`, or an inline `font-weight` or
    `font-style`) or inherits. WebKit un-bolds part of a bold element with an inline
    `font-weight: normal`.
  - The trailing `<br>` the browser keeps after a last line break is not text.
- **Filling it** from the paragraph's runs wraps their bold and italic stretches in `<b>` and
  `<i>`.

### Commands

- **`text.bold` and `text.italic`** (Cmd/Ctrl+B, Cmd/Ctrl+I) run only while a paragraph is
  open. They run the browser's own `bold` and `italic` editing commands on the field's
  selection, so the field's undo covers them.
- The registry's shortcut handler calls `preventDefault`, so WebKit's built-in Cmd+B does not
  toggle a second time.
- **The B and I buttons** above the field are toggle buttons that do not take focus, so the
  selection stays. They show whether the selection is bold or italic (the browser's
  `queryCommandState`).

### The engine: marks on the edit

- **`BlockEdit::marks`** (UTF-16 ranges with bold and italic) set each character's bold and
  italic. Without them, a character keeps its font's own (ADR 0024).
- **Font, size and colour** still come from the old text through `restyle` (ADR 0024).
- **In the page's own fonts:** a run whose font lacks the bold or italic it is to have takes
  another font of the page in the same family that has it. The family is the name before its
  style, without vendors' "MT" and "PS": "Helvetica-Bold" for a Helvetica word made bold.
- **If the page has no such font,** the paragraph is set in bundled faces at the run's weight,
  slanted for italic (ADR 0024).
- **Text unchanged but looks changed is an edit.** Unchanged text and looks are not.

## Consequences

- **What this enables:** a word made bold or italic, or no longer so, reads back as such from
  the page, in either script. The real-app self-test does this with WebKit's own command on an
  Arabic word.
- **The field still uses the UI's fonts.** Bold and italic show there as the UI font's bold and
  italic, not the PDF's (ADR 0022: a text input, not a preview).
- **`document.execCommand` is deprecated,** but it is the only way to make formatting part of
  the browser's own undo. WebKit, WebView2 and WebKitGTK all support it. If it ever goes,
  formatting moves to our own model of the field, and undo with it.
- **Underline, sizes and colours** cannot be set from the editor yet. They are kept as the page
  has them.

## Verified

- **`richText` unit tests:**
  - fill and read back with bold, italic, line breaks and Arabic;
  - WebKit's inline un-bold, `strong`, `em` and block elements;
  - the browser's trailing `<br>`;
  - comparing looks.
- **Editor tests, in English and Arabic:** the buttons run the browser's commands, and saving
  sends the marks read from the field.
- **Registry test:**
  - unchanged text with unchanged looks is no edit;
  - bold set on unchanged text is one;
  - the commands are enabled only while a paragraph is open.
- **Engine tests:**
  - a red word made bold, a bold word made regular, and a word made italic all use the page's
    own Helvetica-Bold, Helvetica and Helvetica-Oblique, embedding nothing;
  - bold-italic the page lacks is set in IBM Plex Sans BoldOblique;
  - the family key;
  - marks by UTF-16 offset.
- **Real-app self-test (English light, Arabic dark on two fixtures):** a word of an edited
  Arabic paragraph is selected in the field and made bold by `text.bold`. It is in a `<b>` in
  the field, saved with Cmd+Enter, and read back from the page as a bold run.
- **The browser build:** the buttons kept the field's focus and selection, and B showed pressed
  after bolding. The toolbar sits at the inline start in the Arabic UI.
