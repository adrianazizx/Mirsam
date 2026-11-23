# 0051 — Page objects, images and links

Status: Accepted (Milestone 6). Date: 2026-11-23.

## Context

Spec §5.3 (P1): "Images: add, move, resize, rotate, crop, replace, delete, extract"; "Links:
add/edit web and internal links"; "Move/resize/delete any page object; align and distribute".
Spec §9: "links open only after showing the target".

ADR 0005's amendment rules out PDFium regenerating a page's content: it silently damages untouched
content on 27 of 34 pages. Every edit is an incremental update written by our own code
(ADR 0014). ADR 0023 removed and wrote text by byte range; ADR 0021 wrapped a page's content once
in `q … Q` and appended streams; PR #21 made image XObjects (`pdf::image`).

Objects are the next step: what a page *draws* — images, paths, text, form XObjects, shadings —
traced to the bytes that draw them, and changed by changing those bytes only.

## Decision

### 1. Tracing a page (`objects::trace`)

One walk over the page's decoded (and, in an encrypted file, decrypted) content streams with the
M3 tokenizer, which keeps every operator's byte range. It follows `q`/`Q`, `cm`, the clip, and the
text state, and finds every **piece** that draws:

- an image (`Do` of an image XObject, or an inline image `BI … EI`): the unit square through the
  matrix;
- a form XObject (`Do`): its `/BBox` through its `/Matrix`, one object (a group; not entered);
- a painted path: from its first construction operator to its painting operator, its points
  through the matrix, grown by half the line width when stroked;
- a shading (`sh`): the clip's bounds;
- a text-showing operator: its glyphs' advances (the font reader's widths, or a Type 3 font's own
  `/Widths` through `/FontMatrix`) and the font's ascent and descent, through the text and
  graphics matrices.

Bounds are cut to the clip in force. Paths, shadings and forms covering 90% of the page are page
backgrounds and are not offered (a press on an empty spot selects nothing); a full-page scan
image is offered. Invisible text (render mode 3, OCR layers) is not offered.

**Each piece keeps a recipe of the state it was drawn in:** the operators that set line style,
colour spaces and colours, text state and `gs` (the last of each kind, every `gs` in order with
the matrix it ran under, since a soft mask is placed by it), the clips in force as paths in page
space, and the matrix. With it a piece can be drawn again elsewhere in the content exactly as it
was.

**Objects:** each image, path, form or shading is one object. Text-showing operators drawn one
after another join one **block** while each touches the last once grown by 1.5 font sizes along
the line (word spaces, a justified Arabic line's wide ones) and half of one across it (the gap
between lines), but not a column's gutter or a paragraph's space. Chrome draws one glyph per `Tj`
(Type 3 fonts, marks with no advance); these join into lines and paragraphs the same way.

Objects are listed in drawing order with their bounds on the displayed page and a fingerprint of
their bytes. An edit names the object by index and fingerprint; the page is traced again at edit
time, and a stale reference is refused.

### 2. Changing an object (`objects::object_update`)

Every change is a set of byte-range splices of the page's content streams (the changed streams
written as new objects, the page's `/Contents` pointed at them), plus new streams and images.
Nothing else changes.

**The wrapper.** A piece this app changes is wrapped where it is:

    /MirsamObj <</G n>> BDC q a b c d e f cm [x y m … l h W n] … Q EMC

- **Move, resize, rotate:** the UI sends an affine map of the displayed page; it is taken to page
  space (`V·D·V⁻¹`) and to the piece's own space (`C·T·C⁻¹`, `C` the matrix in force there), and
  becomes the wrapper's `cm`. The piece stays at its place in the stream, so **its z-order and
  the clip in force are kept**. A wrapped piece edited again has its `cm` rewritten, never a
  second wrapper. `G` groups the pieces of one object (a moved paragraph's runs stay one block).
- **Crop:** a quadrilateral `W n` inside the wrapper, in the wrapper's own space, so it moves and
  turns with the object; uncropping removes it.
- **Remove:** the piece's bytes go. A path that also clipped (`W` before its paint) leaves its
  construction and `W n`, so what came after it is still clipped.
- **Replace an image:** a new XObject, fitted in the old image's box in proportion and centred,
  drawn by `q … cm /MIn Do Q` in the old `Do`'s place (in the old one's turn, crop and z-order).
  Images of ours no stream draws any more drop out of the page's resources.
- **Insert an image:** an XObject (PNG with its alpha as a soft mask, JPEG as it is), drawn in
  its own wrapper in a new stream after the page's content (wrapped once in `q … Q`, ADR 0021),
  upright on the displayed page.

**Text is moved in runs.** The consecutive text-showing operators of one block inside one text
object are one run, from just after the show before it (and the marked content that show closes)
to just after its own last show (and what that closes). Taking a run out of its text object:

    … EMC… ET  <wrapper: BT [reopened marked content] Tm [TJ move] run [EMC…] ET>
    [state the run set] BT [reopened marked content] Tm [TJ move] …

- The text after it gets the line matrix and position it expected (`Tm`, then a `[n] TJ` for the
  distance along the line), so it does not move.
- Marked content open around the run is closed before and opened again after, so `BDC`/`EMC`
  stay balanced. Per-glyph splitting was tried first: Chrome wraps each Arabic letter's glyphs in
  `/Span <</ActualText …>>`, and splitting inside the span broke the letters' text ("المشروع"
  read as "المش" + "روع"). Runs keep a span whole.
- State the run sets (`Tf`, `Tc`, colours, `gs`, …) is set again after it, since the wrapper's
  `Q` would otherwise take it from the text that follows.
- A text object with a `cm` inside it, or spread over two streams, is refused (`textState`).

**Reorder** draws the object again elsewhere from its recipe and removes it from where it was:

- **To front / to back:** in a new stream after (or before) the page's content, where the state
  is the initial one: always possible.
- **Forward / backward:** just after (before) the next object above (below) it that it overlaps,
  at an operator boundary outside text objects. Allowed only where the clips and `gs` in force
  there are a prefix of the object's own: the rest are set again (clips as page-space paths),
  parameters the object does not set go back to their initial values, and the matrix is set.
  Elsewhere it is refused, rather than clipping or blending the object differently.
- Text clipped by text (render modes 4–7) cannot be drawn again: such a reorder is refused.

**Encrypted files:** streams are decrypted to read and new streams and images encrypted under
their own numbers; the page keeps its number. Pages whose resources hold strings are refused when
resources must change (as ADR 0021).

### 3. Links (`links`)

- **Read:** a page's `/Link` annotations, each with its areas on the displayed page (`/QuadPoints`,
  else `/Rect`) and where it goes: a web or mail address (`/URI`), a page (`/Dest` or `/GoTo` to an
  array), a named destination (by name, with its page through the catalog's `/Dests` and the
  `/Names` tree), or another action (JavaScript, launch, another file, a viewer command), shown
  for what it is and never followed. Strings are decrypted in encrypted files.
- **Write:** add over a drawn rectangle or a text selection's runs (`/QuadPoints`), point
  elsewhere, move or resize, remove — each one incremental update and one undo step. Only `http`,
  `https` and `mailto` are written; a bare domain gets `https://`, an e-mail address `mailto:`,
  letters outside ASCII are percent-encoded. A page target is `[page /XYZ left top null]` at the
  page's displayed top-left corner. A named target must exist (it is written as the name or
  string the document uses). New links have no border (`/Border [0 0 0]`).
- **Follow:** with the Select tool, the pointer over a link outlines it and shows where it goes; a
  press on a page or a named destination goes there; a press on a web link pins its address with
  an **Open** button, and only that button opens it. The shell opens it with the `open` crate
  (MIT, ShellExecute on Windows, so no command interpreter reads the address) after checking it
  again: web and mail only, no characters a launcher reads specially, letters outside ASCII
  percent-encoded.

### 4. The UI

The Edit tool has three modes in its strip: **Text** (ADR 0021/0023), **Objects**, **Links**.

- **Objects:** hover outlines an object; a press selects the topmost; dragging moves it; the
  corner handles resize it in proportion (Shift: freely); arrow keys move it 1 pt (Shift 10).
  The inspector shows its kind, position, size and pixels, and runs commands: insert image (the
  shell's file dialog, centred on the page at 96 dpi, at most half the page), replace image, crop
  (handles on the crop box, Enter applies, Escape cancels), uncrop, rotate a quarter turn either
  way, bring to front / forward, send backward / to back (offered only towards objects it
  overlaps), delete. After an edit the object is selected again where it went.
- **Links:** every link is outlined; drag on the page to draw one; press one to change where it
  goes in the inspector (web or mail address, page, named destination), drag it or its corners to
  move or resize it, Delete removes it. The selection toolbar has **Link…** for selected text.
- Every action is a registry command with English and Arabic titles; keys several commands share
  (Delete, Escape, arrows) go to the one that applies now. Object edits are gated by the file's
  `modify` permission, links by `annotate` (ADR 0043). Physical rotation icons do not mirror; the
  address field is left to right in either language.

## Consequences

- **Honest limits:**
  - Text blocks are grouped by geometry: a table's cells on one line can join, two close
    paragraphs can join. A block is moved and resized as it is drawn; it is not re-flowed.
  - Objects inside form XObjects are not traced: a form is one object.
  - Invisible text is not offered, so moving a scan's image leaves its OCR text where it was.
  - Moving an object out of its clip cuts it, as the clip is kept on purpose.
  - Reordering one step is refused where clips or blending differ; tagged content (`MCID`) moved
    to the front or back leaves its structure element pointing at nothing.
  - Not done: align and distribute, selecting several objects, free rotation from the UI (the
    engine takes any map), extracting an image, editing a path's points or colours.
  - A link over a selection spanning pages covers its first page only.
- **Files grow modestly:** a moved block adds its runs' wrappers (2–4 KB updates on Chrome pages,
  under 1 KB on CoreText ones); the content streams of the page are written again compressed.
- **The fuzz harness** (`fuzz/src/lib.rs`) now traces page objects, moves, removes and reorders the
  first few, and reads links.

## Verified

- **`tests/objects.rs`** (PDFium renders, our extractor reads back, `qpdf --check`):
  - a page traced to its objects (a shape, an image, two text blocks, a shape over them), with
    their displayed bounds and the image's pixels;
  - moving a path changes pixels only where it was and where it went; a second move rewrites the
    wrapper;
  - an image halved, turned a quarter, cropped (the crop moves with it), uncropped, and replaced
    by a wide PNG (fitted, centred, turned as the old one);
  - text moved and removed while the text after it in the same text object keeps its pixels;
  - one show of a text object moved while the next keeps its place;
  - reordering: to front, one step back under the shape it overlaps, to back, forward again;
  - a dashed, thick, clipped stroke under a scaled matrix brought to the front draws exactly as
    before;
  - a new image upright where placed on pages turned 0°, 90°, 180°, 270°, and moved as displayed;
  - an AES-256 file: an image placed and moved, a text block moved, `qpdf --check` with the
    password, reopened;
  - Chrome's Arabic (`/ActualText` per letter) and mixed pages: a block moved, the page's words
    read back unchanged in logical order;
  - undo returns the original bytes; a full save keeps the edit; edited again after reopening;
  - a stale reference is refused.
- **`tests/links.rs`:** a drawn link to an Arabic address (percent-encoded) reads back where drawn,
  the page text unchanged; unsafe addresses refused; page and named destinations (name tree and
  `/Dests`) read and written, unknown names refused; pointed elsewhere, moved, removed, all
  undone; a selection's two runs on pages turned four ways; an encrypted file's address encrypted,
  read back, saved and reopened.
- **Unit tests:** paths and clips told apart; the recipe keeps the last of each kind and every
  `gs`; the wrapper recognised; a show's line and position; addresses normalised and held to web
  and mail.
- **Shell:** the UI's shapes of object and link edits parse; a new image's place; only web and mail
  addresses open.
- **UI (`commands/objects.test.tsx`, `viewer/controller.test.ts`):** move, turn, delete, undo,
  crop, insert (and cancel), replace; shared keys; permissions; links added, retargeted, deleted;
  a page link followed at once and a web link opened only from its pinned address; the panels in
  English and Arabic; the viewer's hit-testing (topmost), drag, proportional resize, crop inside
  the object, drawing a link, moving one, hover and press with the Select tool.
- **Real shell self-test** (`scripts/selftest.sh`, checked by `scripts/check-selftest.mjs`), English
  (Chrome's mixed page, light) and Arabic (CoreText's contract, dark): a text block moved through
  the commands and IPC, read back where it went, undone byte for byte; a link to an Arabic address
  added, read back percent-encoded, undone. The page's words are unchanged on the Chrome page. On
  CoreText's page, whose Arabic draws each letter's dots as glyphs with no text ("suspect" text,
  ADR 0023), the moved English heading shares a baseline with the Arabic one and a dot regroups
  with it in extraction; the drawing is unchanged, so the check applies to pages that read well.
- **Measured** (release, this Mac, `examples/objects_report.rs`): tracing a fixture page takes
  1–8 ms (6 objects on Chrome's Arabic page, 57 on the bilingual table: 40 paths, 17 blocks);
  moving or reordering a block takes 0.3–3 ms, 9–10 ms on the 1,000-page file (its bytes are
  copied to reopen, ADR 0014).
