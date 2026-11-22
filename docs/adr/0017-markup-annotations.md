# 0017 — Markup annotations, and encrypting what an edit adds

Status: Accepted (Milestone 2). Date: 2026-11-22.

## Context

Spec §5.2 (P0): highlight, underline, strikethrough, and squiggly. They must be standard PDF
annotations, so other viewers show them. The floating toolbar over selected text (spec §4)
offers them.

ADR 0014 makes every edit an incremental update from our own writer, and undo a stack of them.
Annotations should work the same way, so one undo covers both.

Two things stood in the way:

- **Encrypted files.** The writer copied strings as read. That works for an object it
  *rewrites*: the object keeps its number, so its strings still decrypt. A *new* object (an
  annotation, its appearance stream) has to be encrypted with a key derived from its own number.
  The writer could not do that.
- **Repainting.** A new revision rebuilt the whole page view, so every page went blank for a
  moment. That is acceptable after a page edit, not after a highlight.

## Decision

### Annotations (`safha_core::annotate`)

**What is written.** Each markup is a standard text-markup annotation:

- `/QuadPoints`, one quad per run of selected text, in the text's own orientation;
- `/C` colour, `/F 4` (prints with the page), `/P`, `/NM` (128 random bits), `/M`,
  `/CreationDate`;
- its own appearance stream (`/AP /N`), a form XObject drawn in page space:
  - **highlight:** each quad filled, blended with `Multiply` so the text stays dark;
  - **underline:** a line just inside the bottom of the line box (below the descenders);
  - **strikethrough:** a line through the middle;
  - **squiggly:** a zigzag along the bottom.
  - Line width is 7% of the line's height (5% for the squiggle).

**Where it is written.** One update holds the stream, the annotation, and the page's
`/Annots`:

- an `/Annots` array that is an object of its own is rewritten in place;
- otherwise the page dictionary is rewritten with the array in it.

**Geometry.** The UI sends the selection's rectangles as it draws them (displayed-page points).
The core maps them back through the page's view transform (`ViewTransform::page_point`), so
rotated pages and offset crop boxes work. What is selected is exactly what gets marked.

**Colours.** Defaults for now:

| Markup | Colour |
|---|---|
| Highlight | Yellow |
| Strikethrough | Red |
| Underline, squiggly | Blue |

Choosing a colour comes with the inspector.

**A selection across pages** gives one annotation per page. All of them go in one update, so one
undo removes them all.

**Removing** takes a position in the page's `/Annots`, as PDFium numbers them. The annotation's
pop-up and any replies to it (`/IRT`) go with it. Only the four markup kinds can be removed so
far; links and form widgets cannot.

**Listing** uses PDFium (`Document::page_markup`), so it works wherever PDFium can read the
annotations, including in object streams.

### Encrypting new objects (`pdf::crypt`)

The standard security handler, for writing:

- the file key is derived from the password the document was opened with (the empty user
  password when it opened without one);
- the key is checked against `/U`, or unlocked through `/O` for an owner password;
- supported:
  - RC4, 40 to 128 bits (revisions 2–4);
  - AES-128 (`/AESV2`);
  - AES-256 (`/AESV3`, revisions 5 and 6, Algorithms 2.A and 2.B);
  - `/Identity` crypt filters;
- the strings of a new object, and its stream, are encrypted with that object's key, with a
  random IV for AES.

Public-key security handlers are refused (`edit_unsupported`).

**Dependencies:** RustCrypto `aes`, `md-5`, `sha2`, and `getrandom` (MIT OR Apache-2.0). Tauri's
dependency tree already had the same versions. RC4 is twenty lines here; no crate is needed.

### Painting again instead of rebuilding

- **Undo history.** `EditHistory` records whether each step changed *pages* or only
  *annotations*. The shell's `EditOutcome` says which (`pagesChanged`); undo and redo report the
  step they undid or redid.
- **Store.** An annotation change bumps `paint` instead of `revision`. Search results, labels,
  and the current page stay.
- **Page view.** `ViewerController::refresh` swaps in a fresh tile client, so renders of the old
  revision are ignored. The tiles and previews on screen stay as stand-ins until new ones cover
  each page, so nothing flashes.
- **Thumbnails and the organizer grid** keep each old image until its new one arrives.

### Selecting and deleting markup

- **Selecting.** A plain click on markup selects it: an outline, and a toolbar with Delete.
  Dragging still selects text.
- **The first click works** even before the page's markup has loaded: the click waits for it.
- **Delete** is `markup.delete`. Its shortcut is the Delete key, or ⌫ on a Mac, which sends
  Backspace (`Shortcut.alternate`).
- **Escape** clears either kind of selection.

## Consequences

- **Small updates.** A highlight on a 1,000-page document is still a few kilobytes appended.
- **Signed documents** keep their signatures valid in the saved file, as with page edits
  (ADR 0015). Whether the signature's permissions (`/DocMDP`) allow annotations is not checked
  until M5.
- **Not yet:**
  - choosing colours;
  - comments on markup (pop-up notes);
  - author names (`/T`);
  - `/Contents` with the marked text;
  - removing other annotation kinds.

  Notes and the comments panel (slices 6 and 8) build on this module.
- **Encrypted object streams** remain unread by our parser (ADR 0014). The decryptor here could
  lift that, but it would also have to re-encrypt strings of objects moved out of a stream.
- **The click-to-select hit test** uses each quad's bounding box. On a page rotated by a
  multiple of 90° that is exact; skewed text would need the quad itself.

## Verified

- **Unit tests:**
  - RC4 against published vectors, and AES-CBC against NIST SP 800-38A;
  - a stream round trip with each method;
  - PDF dates;
  - each kind's appearance content, including a line on a turned page and the squiggle's
    bounds;
  - refusal of bad input;
  - the history's change kinds.
- **`tests/markup.rs`, rendered by PDFium:**
  - **Each kind on its line.** Four kinds on four lines of an English page draw colour on their
    own line and change no pixel elsewhere. They are listed back by kind at the right places.
    Text extraction is unchanged. The file passes `qpdf --check`, and saving keeps all four.
  - **Arabic lines.** A two-line selection gives one annotation with two quads.
  - **Turned pages.** Pages turned 0°, 90°, 180° and 270° are each marked on their first line
    and nowhere else.
  - **Encrypted files:**
    - AES-256 opened with the user password and with the owner password;
    - AES-128, RC4-128 (also with the owner password), and RC4-40;
    - a permissions-only file.

    In each, PDFium draws the highlight, and qpdf decrypts the new date string and the
    appearance stream.
  - **An `/Annots` array of its own** is extended in place. Removing a highlight takes its pop-up
    and reply and leaves the link. Removing the link is refused.
  - **Undo and redo**, and a two-page selection as one step.
- **UI tests:**
  - the viewer repaints in place, keeping its canvases until new ones arrive;
  - a click selects markup, even before the page's markup has loaded;
  - the commands mark, delete with ⌫/Del, and undo as paint-only changes.
- **Browser preview:**
  - the toolbar's buttons, in English and in Arabic (right to left, Arabic labels);
  - highlight, underline and strikethrough drawn;
  - a click selects the highlight, ⌫ deletes it, and undo brings it back;
  - stale tiles are released once replaced.
- **Real-app self-test, English and Arabic (right to left, dark):**
  - selecting a page's text and highlighting it paints yellow in the same view: 66,328 px in
    12 ms for English, 32,317 px in 20 ms for Arabic;
  - the page lists one highlight with a quad per selected run;
  - undo removes it.
