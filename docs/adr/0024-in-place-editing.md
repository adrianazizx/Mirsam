# 0024 — Editing a page's own text in place

Status: Accepted (Milestone 3). Date: 2026-11-22.

## Context

Spec §7's pipeline, steps 3–6:

3. **Font handling:** reuse the PDF's embedded font if it has the glyphs. Otherwise substitute
   for the whole paragraph, and show that a substitution happened.
4. **Layout:** reflow inside the block's box.
5. **Write-back:** remove the block's text objects and write new ones, keeping z-order and
   clipping.
6. **Verify:** what is seen while typing is what gets saved.

What was in place before this decision:

- the analysis tracing each block to the operations that drew it (ADR 0023);
- a writer for text in page content (ADR 0021);
- an editor surface: engine outlines on a canvas over a hidden text field (ADR 0022).

## Decision

### The editor for one block (`text_edit::BlockEditor`)

It is made once per block opened, and holds:

- **The text:** the block's text in logical order, with soft line breaks turned into spaces so
  the paragraph reflows.
- **The font:**
  - The PDF's font is matched by name among the bundled faces. The five candidates are IBM Plex
    Sans and IBM Plex Sans Arabic, Amiri, and two new ones, Noto Naskh Arabic and Noto Sans
    Arabic, all OFL.
  - The text's coverage decides first, then the name and style.
  - Then the **widths as drawn**, measured from each letter's box on the page. Only letters
    and digits that do not join count: an Arabic letter's box is that of its joined form, which
    no isolated advance matches, and on Chrome's pages it drowned the signal.
  - With those widths, Chrome's unnamed Type 3 fonts get Noto Naskh Arabic, which is what drew
    them (its Latin widths match exactly). The sentence therefore keeps its line breaks.
  - The match is reported as exact, same family, or substitute. The whole paragraph is set in
    it, with the bundled Plex faces as fallback.
- **The box:**
  - *A paragraph* (two lines or more, or justified) reflows inside its own width.
  - *A single line* (a heading, a label, a cell) grows along its line, the way its alignment
    points:
    - leftwards for right-aligned Arabic;
    - rightwards for left-aligned English;
    - both ways when centred.

    It grows up to an em short of the next text beside it, or the page's edge, and only then
    wraps. Wrapping a heading inside its own width put its new second line on the next
    paragraph's baseline, where the two read as one line.
  - The first baseline stays where the original's was. Lines keep the original's pitch.
    Alignment and direction are the block's.
- **What it refuses (`Refusal`), and why:**
  - **`needsOcr`:** the page's character mapping is broken, so its text is not what it draws.
  - **`untraced`:** some characters trace to no operation.
  - **`shared`:** an operation also draws other text.
  - **`turned`:** the page is displayed rotated.

  The Edit tool lists each block with its refusal, so a press on a refused block says why,
  instead of opening an editor that would damage the page.

Each layout (one per keystroke) reuses this setting. Applying the edit
(`Document::with_replaced_block`) is one incremental update, and one undo step:

1. it blanks the block's operations (ADR 0023);
2. it writes the new text (ADR 0021) at the same place;
3. it drops the page's own `MT…` font resources that no remaining operation uses, so editing
   an edited block twice leaves one set of fonts.

Blank text removes the block.

### The shell: an editing session

| Command | What it does |
|---|---|
| `text_blocks(document, page)` | Each block's displayed box and refusal. |
| `begin_block_edit(document, page, block)` | Makes the editor and holds it under a token. Returns the text, the font match, the box, the colour, direction, whether the runs differ in size or colour, and the first layout. |
| `layout_block_edit(token, text, known)` | The text laid out, as the writer will write it. |
| `commit_block_edit(token, text)` | Writes the text: one undo step, or nothing when the text is unchanged. Refused (`stale_edit`) if the document changed since the session began. |
| `cancel_block_edit(token)` | Ends the session without writing. |

The layout carries:

- lines, glyph positions, and cluster extents (for selections);
- caret stops, in UTF-16 offsets, as the text field counts them;
- the outlines of glyphs the UI does not hold yet. These are SVG paths from the very subsets
  the writer embeds.

Only one session per document is held. A closed document drops its session.

### The UI

- **The Edit tool:**
  - a press on a block opens it;
  - a press on a refused block shows why (`errors.textRefused.*`, in both languages);
  - a press elsewhere places new text, as before.
- **The surface** (`viewer/blockEditor.ts`) lives inside the page's element, so it scrolls,
  zooms, and takes the reading theme with the page:
  - a paper-coloured mask covers the original block;
  - a canvas draws the engine's glyphs in the block's colour, with the selection (cluster
    extents) and the caret (caret stops);
  - a hidden, single-line text field takes typing, IME composition (drawn underlined), the
    clipboard, and undo of typing;
  - the field follows the caret, so an IME's candidate window opens beside it.
- **Keys:**
  - Left and Right are the field's own. It is one line in the block's direction, so its visual
    order is the engine's.
  - Up, Down, Home and End follow the engine's lines.
  - Escape cancels. Cmd/Ctrl+Enter, Done, or a press anywhere outside commits.
- **Presses on the canvas:** a press places the caret, a drag selects, a double-click selects a
  word.
- **After a commit:** the editor stays drawn, taking no input, until the page's new tiles cover
  it, so the old text never flashes back.
- **A bar above the block** (`BlockEditBar`) holds Done and Cancel and names the font. A
  substitute is flagged with the original's name, isolated in `<bdi>`. So is a paragraph whose
  sizes or colours will become one.

## Results (this Mac, debug build)

- **Spec §7's acceptance tests, in `tests/in_place.rs`:**
  - **A typo fixed in English** ("efficiently" → "efficaciously", which re-wraps): no pixel
    outside the block changes, and the first baseline stays within 0.05 pt. Re-setting the
    same text changes under 5% of its ink pixels.
  - **A word inserted into an Arabic line:**
    - it joins;
    - the line grows leftwards with its right edge kept within 1 pt;
    - nothing changes outside the line's band;
    - it reads back exactly.

    Inserting more than fits on the line wraps it right to left: every line starts at the
    original's right edge, within a letter's side bearing.
  - **A mixed sentence** ("iPhone 15 Pro" → "16", "اشتريت 3" → "4") reads back in order, in
    the matched substitute, on one line.
  - **Edit, save, reopen, edit again:** no drift, one set of fonts, still known as this app's
    text after a full save.
  - Refusals are tested, and so is blanking a block. `qpdf --check` passes on every result.
- **The self-test, in the real webview:**

  | Run | Block | Typing (median, p95) | Read back, undone, Escape |
  |---|---|---|---|
  | English UI | Chrome's mixed page, Noto Naskh Arabic substitute, right to left | 17 ms, 26 ms | all pass |
  | Arabic UI, dark | an English block of the page-labels fixture | 17 ms, 23 ms | all pass |
  | Arabic UI, macOS CI fixture (CoreText contract) | none: every block is `needsOcr` | — | — |

  - Typing is within spec §11's 32 ms per keystroke.
  - The contract run is not a silent skip: the checker requires the refusal.

## Consequences

- **The PDF's own embedded font is not reused yet.** A subset rarely has the glyphs new text
  needs, and reusing it means reading its programs and widths and extending its encoding. Until
  then every edited paragraph is set in a bundled face. The bar says so when it is not the
  same font.
- **Installed fonts are not offered.** `FontLibrary` does not read a font's embedding
  permissions (`OS/2.fsType`), and embedding a restricted font would break its licence. They
  come back once it does.
- **One style per paragraph.** The paragraph is set in its first run's font, size and colour.
  - The bar warns when sizes or colours differ.
  - Bold or italic words inside a paragraph are lost without a warning, since a different font
    name often only means another script.
  - Styled runs in the editor are the next step.
- **Extraction across lines:** a parenthesis split across two lines of right-to-left text reads
  back mirrored. Each bracket is alone on its line, and lone brackets keep their mapping
  (ADR 0008). The width matching above avoids most such re-wraps. The extractor should pair
  brackets across a block's lines.
- **The mask is paper-coloured.** On a page with a coloured or patterned background, the
  original's area shows as a white box while editing; the written result is unaffected.
- **Caret movement:** Left and Right are the field's own movement, visual along one line. At a
  bidi boundary where the engine has two stops (`split`), the first is drawn.
- **Still to check by hand:**
  - real input methods: macOS and Windows Arabic keyboards, press-and-hold, the emoji picker,
    dictation;
  - the surface on WebView2 and WebKitGTK, which run in CI's self-test only through the
    automated check.
- **Turned pages** are refused. Their lines are not horizontal on screen, and the surface lays
  out upright.

## Verified

- `tests/in_place.rs` (7 tests). The width evidence is checked through the mixed sentence's
  substitute.
- The shell: UTF-16 offsets and outline caching (`text_editing` tests).
- The UI:
  - `viewer/blockLayout.test.ts`: caret per line, hit-testing, vertical moves, line edges,
    selections in both directions, words in both scripts, block hits;
  - `viewer/controller.test.ts`: the Edit tool's presses, opening, typing, commit on an
    outside press, Escape, closing.
- The self-test's in-place check (above) in English and Arabic.
