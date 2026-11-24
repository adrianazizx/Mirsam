# 0045 — Form filling, Fill & Sign, and kept signatures

Status: Accepted (Milestone 5). Date: 2026-11-22.

## Context

Spec §5.5 and §5.6 make these P0:

- **Fill AcroForms** (text, check box, radio, drop-down, list, date) with correct Arabic shaping in the
  appearances we generate.
- **Fill & Sign on flat forms**: click anywhere to type, tick, or date.
- **Drawn, typed, and image signatures and initials**, kept locally.

Spec §6.4 forbids the shortcut: "generate field appearance streams ourselves with full shaping; never
rely on `NeedAppearances` and another viewer's renderer". This is the complaint the spec opens with:
mainstream editors write disconnected or reversed Arabic into form fields.

Two facts shaped the design:

- PDFium loads a form handle with every document (pdfium-render does so on load). When a document sets
  `/NeedAppearances true`, PDFium regenerates field appearances itself, with its own unshaped
  generator. So would any other viewer.
- We already have everything needed to draw shaped text into a form XObject. The text engine
  (`typeset`, ADR 0020) lays it out, and `pdf::text` (ADR 0019) writes it with embedded font subsets,
  ToUnicode, and `/ActualText`. The annotation writer (ADR 0017) appends appearance streams and
  annotations as incremental updates, encrypted where the file is.

## Decision

### 1. Fields are read and written by our own parser, as annotations are

`safha_core::forms` walks `/AcroForm /Fields` with the crate's bounded parser (`pdf::File`). For each
terminal field it reads:

- the fully qualified name and the kind (`/FT`, `/Ff`);
- inherited `/V`, `/Opt`, `/MaxLen`, `/Q`, and `/DA` (falling back to the AcroForm's own);
- its widgets, with their pages (from the pages' `/Annots`) and their "on" states.

Strings are decrypted with the owning object's key. In an encrypted file, fields inside object
streams are not readable (the parser does not decrypt object streams) and are not listed.

`Document::form` returns the fields, each widget with its box on the displayed page.
`Document::with_form_edits` applies `FormEdit { field, values }` as one incremental update per edit,
which together make one undo step (`Change::Annotations`: the pages are only painted again).

Kinds and values:

| Kind | Value (`values`) | Written |
|---|---|---|
| Text | one string; line breaks become spaces unless multi-line; `MaxLen` enforced | `/V` (text string), new `/AP /N` per widget |
| Drop-down (`/Ch` + Combo) | one export value; must be a choice unless editable | `/V`, `/I` removed, new `/AP` showing the choice's *label* |
| List | the chosen export values; several only if multi-select | `/V` (string or array), `/I` (indices), new `/AP` with the chosen rows highlighted |
| Check box, radio | the "on" state, or none for off | `/V` (name) on the field, `/AS` on every widget |
| Push button, signature | — | refused |

Read-only fields are refused. A value that is not a field's choice is refused (`invalid_edit`).

### 2. Appearances are ours, shaped, and drawn the way the widget asks

`forms/appearance.rs` builds each widget's normal appearance as a form XObject:

- **Box:** `/BBox` is the widget's size, and `/Matrix` carries the widget's own turn (`/MK /R`).
- **Background and border** come from `/MK /BG`, `/MK /BC`, `/BS` and `/Border` (solid, dashed,
  underline; beveled and inset are drawn solid).
- **Text** is wrapped in `/Tx BMC … EMC` and clipped inside the border, as other writers do.
- **Size and colour** come from `/DA`. A size of 0 means "fit": as tall as the box allows, at most
  12 pt, then shrunk to fit the width (4 pt at least). A multi-line field fits at 12 pt.
- **Alignment (`/Q`):**
  - 1 centres; 2 aligns right.
  - **0 (the default) starts where the text's own direction starts**, so an Arabic value sits at the
    right even in a field its author left at the PDF default. This departs from a literal reading
    of `/Q 0` ("left"), deliberately: a left-aligned Arabic name is the very defect spec §1 lists.
- **Single-line text** is centred vertically. When it is too wide, the start of the text shows: the
  left of English, the right of Arabic.
- **Comb fields** put one cluster in each of `MaxLen` cells across the whole width. The value is
  shaped first, then each cluster is moved to its cell's centre; right-to-left text fills the cells
  from the right.
- **Passwords** show bullets.
- **Lists** show one row per choice from `/TI`, the chosen rows highlighted (the colour other viewers
  use), laid out as one text so the rows share their font subsets.
- **Check boxes and radio buttons** keep the appearances their widgets already have. A widget without
  any gets two of ours, "on" and `Off`: a tick, cross, or dot drawn as paths (no ZapfDingbats), its
  style from `/MK /CA`, and a round frame for radio buttons.

Text is set in the bundled Sans family (IBM Plex Sans and Sans Arabic, the text's own script first,
ADR 0019), whatever font `/DA` names. `/DA` is left as it was, so a viewer that redraws the field on
editing still has what it expects.

### 3. `NeedAppearances` is turned off, after we draw every field

When a document asks viewers to draw its fields, the first fill also draws every other field that
has a value (and every button widget without states), then writes `/NeedAppearances false`. This
happens in the AcroForm object, or in the catalog when the AcroForm is direct. From then on, PDFium and
other readers show what we drew. Values are never dropped. Fields never filled keep their `/V`.

### 4. Fill & Sign items are stamps of ours

`annotate::fill` places items as `/Stamp` annotations whose `/Name` says what they are:

| `/Name` | Item |
|---|---|
| `MirsamText` | text or a date |
| `MirsamCheck`, `MirsamCross`, `MirsamDot` | a mark |
| `MirsamSignature` | a signature |

Why stamps:

- **They are not comments.** Text boxes (`/FreeText`) are listed as comments (ADR 0041); filled-in
  values must not be. `AnnotationKind` gains `FillText`, `Mark` and `Signature`, which the comments
  list and replies leave out.
- **Viewers draw them from their appearance alone**, which is what we want.
- **Moving or resizing is only a new `/Rect`.** Each appearance's `/BBox` is the item's natural size,
  and viewers fit it to the `/Rect` (ISO 32000-2, 12.5.5). `AnnotationEdit::SetRect` changes
  only that. For text, it also scales the size kept in `/DA`, so changing the text later keeps the
  size it was shown at.

What each item draws:

- **Text** is laid out unwrapped (one line per line typed), as wide as its longest line, and drawn
  with embedded subsets. `/Contents` holds the text, for accessibility and search.
- **Dates** are text: today's date as the UI language writes it, with Western digits in the Arabic
  UI (`formatTag`, the open digits question in PROGRESS.md).
- **Marks** are vector paths in a 100-unit box.
- **Colour:** text, dates and marks are placed in one of the four document colours the Edit tool
  offers (black, blue, red, green; `state/colors.ts`), black by default.
  - Choosing a colour in the Sign panel applies to the next items. With a text or a mark selected,
    it also recolours that one (`AnnotationEdit::SetColor`, one undo step). The appearance is drawn
    again in the new colour, in the same box, and `/C` and fill text's `/DA` are updated, so later
    changes keep the colour.
  - Signatures keep their own ink (dark blue for drawn and typed, the image's own colours), and
    refuse `SetColor`.

Every item stands upright on a turned page, as a text box does. Its appearance is turned back by the
page's `/Rotate`, and changed text keeps its displayed top-left corner.

### 5. Signatures: three kinds, kept in the state store, placed as stamps

`Signature` is one of three kinds:

- **Drawn**: strokes with pressure, in the pad's units, y down. The appearance is paths, with widths
  stepped by pressure as ink's are (ADR 0018).
- **Typed**: text set at 36 pt, in Naskh (Amiri, the default) or Sans. The appearance is shaped text
  with embedded subsets.
- **Image**: a PNG or JPEG's bytes, at most 8 MB and 4,096 px a side.
  - A JPEG goes in as it is (`/DCTDecode`), read only as far as its frame header (`jpeg::frame`:
    baseline or progressive, 8-bit, gray or RGB). CMYK JPEGs are refused: they disagree on inversion.
  - A PNG is decoded with the `png` crate (MIT OR Apache-2.0, already in the tree). It is written as
    8-bit samples under `/FlateDecode`, with alpha as an `/SMask`, so a signature with a transparent
    ground leaves the page showing through.

The state store keeps them (schema version 4, a `signatures` table: kind, initials flag, data,
created). That is Drawn and Typed as JSON, an image as its bytes, newest first, at most 24. They
never leave the device. Placing one keeps its aspect: the UI gives a width (150 pt for a signature,
60 pt for initials) and the height follows.

The webview never handles signature files:

- **Images** are picked through the shell's native dialog.
- **Image previews** load from `safha://localhost/signature?id=N`, decoded by the webview (ADR 0011).
- **Drawn and typed previews** are SVG paths and styled text, from data the shell sends.

These are **not digital signatures**. A stamp proves nothing cryptographically. Certificate
signatures (PAdES) are M5's P1, ADR 0044 on its own branch. A signature *field* (`/FT /Sig`) is
listed but not filled here.

### 6. UI: two tools, every action a command

- **Forms** (inspector panel): every fillable field, grouped by page, as native controls.
  - Text fields take their direction from what is typed (`dir="auto"`).
  - Text is set when the field is left or Enter is pressed, so one name is one undo step.
  - Read-only fields are shown, not editable.
  - An XFA form is reported as unsupported (spec §5.5).
  - A document without fields points to Fill & Sign.
- **Sign** (inspector panel): what a press on the page places (select, text, date, tick, cross, dot,
  or a kept signature), the kept signatures and initials, and Smaller / Larger / Delete for the
  selected item. The selection toolbar offers the same three over a selected mark or signature.
  - **After each placement:** marks stay armed, to tick several boxes. Text stays armed for the
    next field. A date or a signature goes back to selecting.
  - **Selecting:** pressing an item placed before selects it, never places over it. Fill text opens
    the note editor, to change its text.
  - **Moving:** an item placed before is dragged to move it, with the Sign tool (armed or not) or
    the Select tool.
    - A press that travels 3 px is a drag; one that does not is a click, which selects. A dashed
      outline shows where the item will land, and it is kept on its own page.
    - Letting go moves it as one undo step (`fill.move`, the same `SetRect` as resizing), and it
      stays selected.
    - The arrow keys nudge the selected item 1 pt, and with Shift 10 pt (`fill.moveLeft`,
      `fill.moveLeftMore`, and so on). They do so only while the page view has the focus, so a
      toolbar's or a list's arrow keys keep their meaning. Each nudge is one undo step.
    - After an edit the viewer reads the visible pages' annotations again at once, so an item just
      placed can be dragged without first moving the pointer over it.
  - **To another page:** an item dragged over another page follows the pointer there; the view
    scrolls on near its top or bottom edge, to reach pages out of sight. Letting go there, or the
    panel's "previous page" and "next page" (`fill.toPreviousPage`, `fill.toNextPage`, the item
    where it was on its page), moves it as one undo step (`AnnotationEdit::MoveToPage`, the shell's
    `move_annotation_to_page`).
    - The annotation keeps its object. It leaves one page's `/Annots` for the end of the other's,
      with `/P` and `/Rect` set for the new page, and stays selected there.
    - On a page turned otherwise, its appearance stream is written again under its own number with
      the new page's `/Matrix` and its data as it was (so encrypted data stays valid): it stands
      upright on its new page.
  - **Resizing:** a selected item shows a handle at each corner. Dragging a handle resizes the item
    about the opposite corner.
    - It keeps its proportions: the appearance is fitted to the box, so a change of aspect would
      stretch it. It is at least 6 pt, and stays on its page.
    - Letting go is the same `fill.move` (one `SetRect`). Fill text takes the new size into its
      `/DA`, as with Smaller and Larger.
    - A handle reaches 7 px outward but only a quarter of the item inward, so a small item (a
      14 pt tick) still has a middle to drag it by.
    - Smaller and Larger stay as the keyboard and single-click way to resize.
- **New signature** dialog: Draw (an SVG pad with pressure), Type (Naskh or Sans, previewed), or
  Image (the native dialog).

Every control runs a registry command: `forms.openTool`, `form.setValue`, `sign.openTool`, `fill.*`,
`signature.*`, and `signatures.load`. They follow ADR 0043's permission gates: in a file opened with
its user password, `form.setValue` needs `fillForms` and the `fill.*` commands need `annotate`. The
panels say so when a gate is off. The viewer has a new `place` mode. Signature previews are ink on
paper in both themes (`--color-ink` on `--color-page-paper`, a new contrast pair).

## Consequences

- Filled fields and placed items look the same in PDFium (tested by rendering), after a save through
  PDFium (tested), and in any viewer that honours appearance streams. Text reads back in logical order
  once flattened (tested with `qpdf --flatten-annotations`).
- A viewer that regenerates appearances on its own when a field is edited there will draw with the
  `/DA` font and its own (often unshaped) layout. We cannot prevent that; we only never ask for it.
- **Line direction in extraction is not ours to fix here.** On an English page, our extractor reads a
  mixed Arabic/English line left to right. Both scripts come out intact, but their order follows the
  page's direction (ADR 0019's limitation, unchanged).
- **Not done:**
  - **Forms:**
    - JavaScript formatting, calculation, and validation (`/AA`) are not run. A date field is a text
      field.
    - Rich text (`/RV`) is not written.
    - Fields are filled in the panel, not on the page itself.
    - Form creation, import/export (FDF/XFDF), and flattening are P1.
  - **Fill & Sign:**
    - Moving to another page keeps the item's displayed size; on a smaller page it is kept inside
      the page's edges, not shrunk.
    - Handles resize in proportion only; text cannot be reflowed into a different shape.
    - Fill text and marks are placed at 11 pt and 14 pt (then resized as above).
    - The panel shows the colour chosen for new items, not the selected item's own colour.
  - **Signatures and the shell:**
    - Typed-signature previews use the webview's fonts (Amiri where the system has it). The PDF is
      set in the bundled Amiri.
    - The native image dialog and the signature image URL are untested in the real shell.

## Amendment (2026-11-24): moving text boxes, notes and drawings too

The owner asked to "move the signature and the text" after placing them. Signatures and Fill &
Sign items already moved (§6: drag, arrow keys, corner handles, another page). Text boxes, notes
and drawings — M2's annotations, whose "moving annotations" PROGRESS.md had left open — did not.
They now move with the same machinery, not a second one.

### Engine: `SetRect` takes them

`AnnotationEdit::SetRect` (the shell's `set_annotation_rect`) accepts `TextBox`, `Note` and `Ink`
as well as Fill & Sign items (`AnnotationKind::is_movable`, `is_comment_movable`). One incremental
update, one undo step, as before; the annotation keeps its object and its number.

- **A move** (the new box within 0.5 pt of the old one's size) is a translation: the old size is
  kept exactly, so a move never stretches anything. The appearance stays as it was: viewers fit
  its `/BBox` to the new `/Rect` (ISO 32000-2, 12.5.5), which a translation leaves unscaled.
- **What else places it moves along**, so a viewer that redraws from the annotation's own data
  agrees with ours: ink's `/InkList`, a callout's `/CL`, and the pop-up's `/Rect` (written under
  its own number). Arrays reached through references are written into the annotation directly.
- **A note** is only moved: its icon has one size, and a resize is refused (`invalid_edit`).
- **A text box given a new size** is laid out again at the new displayed width, its height
  fitted to its text and its displayed top kept — as changing its text does (ADR 0019). Its
  text and `/DA` are decrypted to do so; the new appearance is a new stream.
- **A drawing given a new size** is scaled: `/InkList` is mapped from the old box to the new one
  and the appearance is fitted to the new `/Rect` (its strokes' width scales with it).
- **Markup** (highlight, underline, strikethrough, squiggly) is refused: it belongs to the words
  it marks.
- **Not to another page.** `MoveToPage` stays for Fill & Sign items: a comment's replies and
  review states are annotations of their own on its page (ADR 0041), and would be left behind.

### UI: the same drag, handles and nudges

- **Who drags what** (`dragsIn`): the Select tool drags everything the user placed (Fill & Sign
  items, text boxes, notes, drawings); the Sign tool its own items, as before; the Comment
  tool's Note and Text box modes their own kind (a press on a note in Text box mode still places
  a text box). Markup is selected, never dragged. The move cursor shows where a press would drag.
- **Kept on its page:** a comment dragged past its page's edge stops there; only Fill & Sign items
  follow the pointer onto another page.
- **Handles** (`RESIZABLE_KINDS`): a selected Fill & Sign item or drawing shows four, resizing in
  proportion as before; a text box shows four that change its width only (`widenedRect`: the
  opposite side stays, the height is the engine's); a note shows none. A drawn line thinner than
  6 pt can still be made smaller: the 6 pt least side then applies to its longer side.
- **Commands:** the drag and the handles go through `fill.move`, the arrow keys through
  `fill.moveLeft` … `fill.moveDownMore` (1 pt, Shift 10 pt), now enabled for any movable
  selection (`MOVABLE_KINDS`) and kept on the page. The ids are kept: they are what the
  shortcuts list, the self-test, and earlier tests name. Arrows move in the page's own
  directions in both languages (the page is drawn as it is, whatever the UI language).
- **The note editor and the keyboard.** Selecting a note or a text box opens its editor, which
  takes the focus, so the arrow keys move the caret. Escape in the editor of a *placed* one now
  drops what was typed and returns to the page with the item still selected (`note.toPage`):
  the arrow keys then move it, and Escape again deselects (`edit.deselect`). A new, unsaved one
  is still cancelled by Escape.
- **Fixed on the way:** the note editor saves on any press outside it, and saving deselected the
  item synchronously, before the viewer saw the press. A press on a selected text box's (or Fill
  & Sign text's) handle therefore found nothing selected and did not resize. The editor now asks
  the viewer (`grabsSelected`) whether the press grabs the selected item or a handle; if so it
  saves without deselecting (`note.save` with `keepSelected`).

### Verified

- **`tests/moving.rs`** (PDFium renders, `qpdf --check`): on Chrome's English and Arabic pages,
  two signatures (drawn and typed), Fill & Sign text, a tick, a text box, a note and a drawing are
  each moved right after placing, saved, reopened, moved again and saved again; each move changes
  pixels only where the item was and where it went, the box moves exactly, kinds, text and strokes
  read back. On pages turned 0°, 90°, 180° and 270°, a moved text box and signature draw the same
  pixels in the new place as in the old. A text box narrowed wraps and grows down, its top-left
  kept; a drawing doubled scales its strokes; a note's resize, markup moves and `MoveToPage` for
  comments are refused; undo returns the bytes before the move; all of it in an AES-256 file with
  Arabic strings read back.
- **UI:** `viewer/controller.test.ts` (dragging each kind with the Select tool, kept on its page;
  the Comment tool's modes and the Sign tool moving only their kinds; a text box's width handles,
  a drawing's proportional ones, none for a note; `grabsSelected`); `commands/moving.test.tsx`, in
  English and Arabic (each kind dragged, saved, nudged, kept on the page, undone back to the saved
  position; a text box widened, a note's resize refused, no page change for comments; Escape from
  the editor keeps the box selected and writes nothing).
- **Real shell self-test** (`scripts/selftest.sh`, checked by `scripts/check-selftest.mjs`): the
  text box dragged and nudged by the arrow-key command and the stroke dragged, through the
  commands and IPC, each read back where it went with its text and points, then undone.

### Limits

- Comments do not change page; Fill & Sign items do.
- A text box's handles change its width only; its height always follows its text.
- A drawing's handles scale it in proportion; a free (stretching) resize is not offered.
- Callouts (`/CL`) written by other apps move with their box; they are not drawn by us, and a
  resize scales the line with the box.
