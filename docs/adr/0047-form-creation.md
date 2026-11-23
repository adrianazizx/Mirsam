# 0047 — Making forms: fields drawn on the page, a properties inspector, tab order, and finding fields on flat forms

Status: Accepted (Milestone 5). Date: 2026-11-23.

## Context

Spec §5.5 makes form creation P1 ("create/edit form fields; properties, tab order") and finding fields on a
flat form P2. Spec §6.4 holds here as for filling: "generate field appearance streams ourselves with full
shaping; never rely on `NeedAppearances`". A field made here must:

- show an Arabic default value joined and in order, in every viewer;
- be fillable here, in Chrome (PDFium), and in macOS Preview (PDFKit);
- be written as an incremental update and undone as one step.

Filling already draws shaped appearances (ADR 0045), and the annotation writer appends objects to an update,
encrypted where the file is (ADR 0017).

## Decision

### 1. Standard fields, made and changed by our own writer

`safha_core::forms::design` turns a `FieldDesign` into an incremental update. It is one of seven edits:

| Edit | What it writes |
|---|---|
| `Add` | A new field and its widget at a box on a page. A document without a form gets `/AcroForm`. |
| `AddButton` | Another button for a radio group. |
| `Update` | New properties. A field's kind cannot change. |
| `SetRect` | Moves or resizes one widget, drawn again for its new box. |
| `RemoveButton` | Removes one radio button. A group keeps one at least. |
| `Remove` | Removes the field from its parent's `/Kids` (or `/Fields` and `/CO`) and its widgets from their pages. A parent left with no kids goes too. |
| `TabOrder` | Reorders a page's widgets (below). |

`Document::with_form_design` applies a list of them as one update: one undo step (`Change::Annotations`).
It refuses XFA forms.

- **Shape.** A field with one widget is one object, field and widget merged, as most writers do. A radio
  group is a parent field with one widget per button.
- **Properties** (`FieldSpec`):
  - name (`/T`; no full stops, unique in the form);
  - tooltip (`/TU`);
  - required and read-only;
  - multi-line, longest text (`/MaxLen`);
  - the default value (`/DV`, and `/V` when it changes);
  - alignment (`/Q`), font and size (`/DA`);
  - choices (`/Opt` as `[value label]` pairs);
  - multi-select, typed text in a drop-down;
  - a border (`/MK /BC`);
  - a push button's label (`/MK /CA`).
- **Flags we do not set are kept** (comb, password, do-not-scroll …).
- **Push buttons** have no action. Mirsam writes no scripts (spec §5.7).
- **A widget on a turned page** gets `/MK /R` equal to the page's turn, so its text stands upright as the page
  is shown (ISO 32000-2, 12.5.6.19). `Look` already honours it.
- **Signature fields** are not made here. They are made by signing (ADR 0044, its own session).

### 2. Appearances: the fill code draws them

A new or changed widget gets its appearance from the same `Look` as filling (ADR 0045), through one function,
`value_appearance`:

- text and choices are shaped, with embedded subsets, starting where the value's direction starts;
- lists highlight their chosen rows;
- check boxes and radio buttons get a tick or a dot drawn as paths, with an `Off` state;
- push buttons get their label centred on a light face.

So an Arabic default value is right in every viewer from the moment the field exists. A form that asked
viewers to draw its fields is drawn by us and has the flag turned off, as with filling.

### 3. `/DA` names our font, and that makes `/Q` logical

A field made here has `/DA (/MirsamSans size Tf colour)` or `/MirsamNaskh`. Check boxes and radio buttons have
`/ZaDb`.

- **Here,** the name chooses the bundled family that draws the field: IBM Plex Sans with Plex Sans Arabic, or
  Amiri.
- **`/DR /Font`** maps the two names to the standard Helvetica and Times-Roman, with ZapfDingbats for
  `/ZaDb`, for viewers that draw a field themselves while it is typed into. Embedding whole fonts so that
  other viewers could type Arabic in our face would cost hundreds of kilobytes per form, and those viewers
  would still not shape it (§ Consequences).
- **Alignment is logical in a field whose `/DA` names one of our fonts.**
  - `/Q 0` is the start of the value's own direction (ADR 0045 already reads every field so).
  - `/Q 2` is its end: the left of Arabic, the right of English.
  - The inspector offers start, centre and end, with icons that mirror in the Arabic UI.
  - In a field from elsewhere, `/Q 2` stays "right" (ADR 0045). Arabic forms made in Acrobat use it to right-
    align Arabic, and changing that would move their text.

### 4. Radio buttons are named by position

Each button's "on" state is its position, `/0`, `/1`, …, and `/Opt` holds the export values (ISO 32000-2,
12.7.5.2.4).

- **Why:** macOS PDFKit writes non-ASCII state names back double-encoded. Saved from Preview, a group whose
  buttons were named "نعم" and "لا" would come back with garbled values. Position names survive every reader.
- **Changes renumber the buttons.** Adding, removing or relabelling buttons names them by position again: the
  `/AP` state keys, `/AS`, and the group's `/V` and `/DV` follow. A group made elsewhere is converted the first
  time its buttons are changed. A group whose buttons share states (buttons that turn on together) is
  refused.
- **The reader** reports a group's choices as state and export value, which the panels already show.

### 5. Tab order is the order of `/Annots`, and the page says so

Tab moves through a page's widgets in their `/Annots` order when the page's `/Tabs` is `/W` (ISO 32000-2,
12.5.1).

- **`TabOrder`** puts the listed fields first, in the order given. The page's other widgets keep their order,
  and annotations that are not widgets keep their places.
- **It sets `/Tabs /W`.** PDFium treats any `/Tabs` other than `/R` and `/C` as array order.
- **The designer orders Tab** by hand (earlier, later) or by rows, top to bottom. Within a row it goes from the
  right or from the left: the user chooses, since a document's direction is its own, not the UI's.
- **On the page,** each field shows its number at its top-left corner.

### 6. Finding fields on a flat form: an assist, reviewed before anything is made

`Document::detect_fields(page)` collects what the page draws through PDFium:

- every path, through nested forms, split into straight pieces and axis-aligned rectangles;
- the page's analysed text.

`forms::detect` then finds:

- **Lines to write on.** Rules, thin filled bars, and runs of three or more underscores. The field sits on the
  line, as long as it is where no text overlaps it, as tall as fits under what is above.
- **Boxes,** stroked or closed by four lines, a table's cells included. A small square is a check box. An
  empty box is a text field, several lines tall above 36 pt. A box with text in it is a label.
- **Box characters** (☐, □, ▢): check boxes.

**Labels.** Each suggestion is named after the text just before it on its line, in that text's direction:
an Arabic label is to the right of its field. Otherwise it takes the text just above it. Names are cleaned
(no leaders, colons or full stops). Suggestions over a widget the page already has are dropped.

**The UI never makes them silently.** They show as dashed boxes on the page and as a list in the panel, each
ticked. The ticked ones are made in one undo step, borderless, since the page draws the blank already. Unnamed
ones get the kind's default name ("Text 1", "نص 1").

### 7. The designer: the Forms tool's second mode

The Forms panel switches between Fill in (ADR 0045) and Design.

- **Tools** draw a text field, check box, radio group, drop-down, list box, or push button.
  - Dragging on the page draws one. A click places one of a default size.
  - The tool stays armed; Select stops drawing.
- **Selecting and changing fields.** Pressing a field selects it, dragging moves it on its page, and the corner
  handles resize it freely. A field's appearance is drawn again for its box, so unlike Fill & Sign items it is
  not kept in proportion.
- **Keys.** The arrow keys nudge the selected field 1 pt (Shift: 10 pt), and Delete removes it. A key that
  several commands share runs the one enabled now. Before, the first command bound to it ran.
- **The inspector** edits the selected field. Text is sent when its box is left or Enter is pressed, so typing
  a name is one undo step.
- **Lists in the panel:** the page's tab order, and the review of found fields.
- **Commands.** Everything is a registry command: `forms.fillMode`, `forms.designMode`, and the
  `formDesign.*` commands.
  - Changing the form needs the `modify` permission (ISO 32000-2 asks for it with `annotate` to create
    fields).
  - Finding fields only reads, and is allowed.
- **Shell.** It has one command for every change, `design_form`, which takes a list of edits with boxes on the
  displayed page and returns the field to select. `detect_form_fields` returns suggestions.

## Consequences

### Checked

- **Here.** Fields of every kind are made on a page without a form, in a plain, a turned, and an encrypted
  file. Their boxes on the page are checked by rendering.
- **Shaped text.** Arabic reads back in order once qpdf merges the widgets into the page.
- **Filling** works here, and survives a save and reopening.
- **Moving, resizing, renaming, relabelling radio buttons, removing, and tab order** are all tested.
- **PDFium** (a unit test) reads every kind with its name and value, and fills a created text field through
  its own form API, in a way we read back.
- **Chrome** (by hand, in the Chrome PDF viewer) shows every field as we drew it. Typing English, ticking,
  and choosing a radio button there all work.
- **macOS PDFKit** (`tests/support/pdfkit_form.swift`) lists every field with its kind and value, fills the
  Arabic name, and writes a file we read back.
- **Finding fields** on a flat form, made in one step: a hand-written page with a rule, a box, a check box,
  a two-cell table, and an Arabic label written by our text writer.

### Limits

- **Another viewer draws what is typed there itself.** Chrome draws Arabic typed into a field in the
  standard font `/DR` gives, poorly joined (PDFium's form editor does not shape). This is ADR 0045's known
  limit. Mirsam draws such a value correctly the next time it fills the field.
- **Preview drops a radio group's `/Opt`** when it saves a file, and moves the group's flags to its buttons.
  After that the buttons' export values are their positions. Our reader takes the flags from the buttons, so
  the group stays a radio group.
- **Not done:**
  - date fields with a format;
  - calculations and validation (the spec's "simple calculations" need JavaScript, which Mirsam does not run);
  - field actions;
  - rich text;
  - comb and password set from the inspector (kept when present);
  - moving a widget to another page;
  - several widgets for one text field;
  - FDF/XFDF import and export, and flattening (P1, separate).
- **Finding fields:**
  - grids drawn as separate rules are found, but a cell with neither a label beside it nor above it is
    unnamed;
  - dotted leaders drawn as many tiny paths are not lines;
  - scanned forms (no vector content) need OCR's layout analysis first.
- **Shared code.**
  - `pdf::Parser::literal_string` now reads octal escapes and line continuations (ISO 32000-2, 7.3.4.2).
    PDFKit writes UTF-16 strings as `\376\377…`, which were misread before.
  - `commandForEvent` prefers the enabled command among several bound to one key.
