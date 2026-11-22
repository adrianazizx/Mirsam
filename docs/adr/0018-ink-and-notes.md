# 0018 — Ink and sticky notes; the Comment tool

Status: Accepted (Milestone 2). Date: 2026-11-22.

## Context

Spec §5.2 (P0):

- freehand ink, with pen and tablet pressure, and an eraser;
- sticky notes, text boxes, and callouts, with right-to-left-aware text entry;
- standard annotations, so other viewers show them.

ADR 0017 added markup as incremental updates with their own appearance streams, so every edit
is one undo step and new objects are encrypted in encrypted files. Ink and notes use the same
machinery.

**Text boxes are left for the next step.** A text box (`/FreeText`) draws its text on the page.
Its appearance stream must hold shaped Arabic with an embedded font subset (spec §6.4).
That is the content-stream writer the Milestone 3 text engine (ADR 0020) is waiting for.
A note is different: its text lives in `/Contents`, and every viewer shows it in its own
pop-up. The page shows only an icon, so nothing has to be shaped.

## Decision

### The engine (`safha_core::annotate`, now a folder)

| Module | What it does |
|---|---|
| `mod.rs` | Edits (`AnnotationEdit`), the `/Annots` plumbing, the common annotation dictionary, listing, and helpers (dates, text strings, numbers) |
| `markup.rs` | ADR 0017, unchanged |
| `ink.rs` | Freehand ink |
| `note.rs` | Sticky notes |

**Ink** (`AddInk`): `/InkList` for every viewer, `/BS /W` for the nominal width, and an
appearance stream with round caps and joins.

- **Pressure changes the width:** half the nominal width at no pressure, 1.5 times at full
  pressure. A mouse counts as middle pressure (0.5), so it draws the nominal width.
- **How it is written:** each run of segments of the same width (rounded to a tenth of a point)
  is one path.
- **Other viewers** that redraw from `/InkList` lose the pressure; ours keeps it.
- **Defaults:** a dark-blue pen, 2 points wide.

**Sticky notes** (`AddNote`): a `/Text` annotation with:

- `/Contents` as a PDF text string (ASCII as is, anything else UTF-16BE with a byte-order mark);
- `/Name /Comment` and `/Open false`;
- a 20-point speech-bubble icon.

On a turned page the icon's form gets a `/Matrix` that turns it back by the page's `/Rotate`, so
it stands upright as displayed. It needs no `NoRotate` flag, which viewers honour unevenly.

**Changing a note's text** (`SetContents`) rewrites the annotation under its own object number.
Strings already in it stay as read; only the new `/Contents` and `/M` are encrypted, under that
number.

**Removing** now covers ink and notes as well as markup, with their pop-ups and replies. Erasing
several strokes is one update: the indices are removed highest first, so each index still
points at the right annotation.

**Listing** (`Document::page_annotations`, replacing `page_markup`):

- the kind, position and boxes come from PDFium, and so does a note's text (decrypted);
- ink strokes are read from `/InkList` with our own parser, because PDFium's safe API does not
  expose ink paths. Where that parser cannot read the page (an encrypted object stream), the
  stroke's box is used.

### The Comment tool (UI)

- **The Comment tool** in the rail shows a strip above the page with three modes:
  - **Note:** a click places a note there;
  - **Draw:** dragging draws with the pen;
  - **Erase ink:** dragging removes each stroke it touches, whole.

  Markup stays on the toolbar over selected text. The rail's Select tool still selects text and
  annotations.
- **The viewer** reports what was drawn, placed or erased. Each becomes a command:
  - `ink.add`, `ink.erase` and `note.place`, hidden from the palette because they take
    arguments only a gesture gives;
  - `comment.note`, `comment.pen` and `comment.eraser` for the modes, in the palette.
- **A stroke** stays on the page it started on, and is clamped to it. Its points come from the
  pointer's coalesced events, so a fast stroke is smooth.
- **The pen's preview** is an SVG drawn the way the engine writes the appearance. It stays until
  the page's tiles, rendered with the new ink, cover it, so nothing flickers between pen-up and
  the repaint.
- **Clicking an annotation** selects it in Select mode, and in Note mode for notes:
  - Strokes are hit along their segments, not across their bounding box. A diagonal line's box
    covers much more than the line.
  - A selected note opens the note editor under its icon. Other annotations get the Delete
    toolbar.
- **The note editor:**
  - its text field uses `dir="auto"`, so an Arabic note reads right to left and an English
    one left to right, in either UI language;
  - the card itself follows the UI language;
  - Save, Cancel, and (for an existing note) Delete are commands;
  - Escape cancels and ⌘/Ctrl+Enter saves;
  - pressing anywhere outside keeps what was written;
  - an empty new note is discarded;
  - saving unchanged text writes nothing.

## Consequences

- **Not yet:**
  - text boxes and callouts (next: the content-stream writer, with shaping and embedded font
    subsets);
  - choosing colours and pen widths;
  - moving or resizing notes and ink;
  - replies;
  - the author's name (`/T`);
  - a partial eraser that splits strokes.
- **A note's reply** (`/IRT`) is listed as a note of its own. A comments panel (slice 8) will
  thread replies.
- **Every stroke is its own annotation and undo step**, as in most editors. Erasing removes whole
  strokes.

## Verified

- **Unit tests:**
  - **Ink:** pressure gives wider runs; a tap draws a dot; strokes stay apart; bad input is
    refused.
  - **Notes:** the icon's matrix for each page rotation; Arabic `/Contents` in UTF-16BE; a
    square on a turned page; the length limit.
  - **Shared:** text strings, and which kinds can be removed.
- **`tests/ink_and_notes.rs`, rendered by PDFium:**
  - **Pressure:** a stroke at mouse pressure is 3 to 6 pixels thick at 2× scale, and the part
    pressed hard is thicker. Nothing changes outside the stroke, and the stroke reads back at
    its points.
  - **Turned pages:** ink on pages turned 0°, 90°, 180° and 270° lands where it was drawn.
  - **An Arabic note on each turned page:**
    - its text reads back exactly;
    - its icon stands upright (tail at the bottom left, nothing at the bottom right, bubble on
      top);
    - it changes nothing outside its square.
  - **Editing a note's text:** done in place, and undo restores it. Setting text on ink is
    refused.
  - **An AES-256 encrypted file:** a note, its edited text, and ink all decrypt and draw;
    `qpdf --check` passes.
  - **The eraser** removes two of three strokes in one step and leaves the middle one.
- **UI tests:**
  - stroke hit-testing and the preview's width runs, matched to the engine;
  - the pen kept on its page, with the preview kept until the repaint;
  - the eraser touching only the stroke under it;
  - a note placed where it is pressed, and an existing one opened;
  - the commands: note placed, saved in Arabic, edited, unchanged save writes nothing,
    cancelled, undone; ink added and erased, each one step and a paint-only change.
- **Browser preview, English and Arabic:**
  - the Comment strip, a note placed and saved with mixed text, a pen stroke drawn then erased;
  - the note reopened in the Arabic UI, with the card right to left and Arabic labels; the text
    field turns right to left when the note starts in Arabic.
  - Found and fixed there: the text field did not take focus, because it mounted inside a host
    the viewer had not shown yet.
- **Real-app self-test, English and Arabic (right to left, dark):**
  - an Arabic note and a pressure stroke are painted in the same view: 807 and 1,866 px for
    English, 170 and 412 px for the Arabic book at its smaller zoom;
  - the note's text reads back exactly;
  - the stroke is erased, and three undos clear everything.
