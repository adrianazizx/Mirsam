# 0042 — True redaction, pattern search, and sanitize

Status: Accepted (Milestone 5). Date: 2026-11-22.

## Context

Spec §5.7 asks for **true redaction** (mark → review → apply, removing text, image pixels, and vector
content under the area), **search-and-redact by pattern** (phone, e-mail, IBAN, ID numbers, Arabic-aware),
and **sanitize** (metadata, hidden layers, attachments, scripts, comments). Spec §11 sets the test: after
applying, the redacted string must be absent from extracted text, raw streams, and metadata.

A black rectangle over text is not redaction: the text stays in the content stream and copies out.
Removing it takes a change to the page's content, and ADR 0005's amendment rules out the obvious tool:
**PDFium must not regenerate existing page content** (`FPDFPage_GenerateContent` silently damaged 27 of
34 test pages). So redaction is done by our own code, as every edit since M2 is (ADR 0014).

## Decision

### 1. Rewrite the content stream, copying what is not touched byte for byte

`safha_core::redact` tokenizes a page's content (`content.rs`; operations keep their byte ranges, inline
images are read by length or by `EI`), walks it with a graphics and text state (`interp.rs`: `cm`, `q/Q`,
`BT`, `Tf`, `Td/TD/Tm/T*`, `Tc/Tw/Tz/TL/Ts/Tr`, paths, `Do`, `BI`, marked content), and writes it out again.
An operation the rewrite does not change is copied from the source; a changed one is re-serialized by the
crate's own writer. The result is one new content stream, wrapped in `q … Q`, followed by a black box
for every area; the page dictionary gets it as `/Contents` (and a patched `/Resources` where an image or
form was replaced). The page's old streams are left unreferenced. Nothing is regenerated.

What happens to each kind of content:

- **Text.** Every glyph's box is worked out on the page (font size, `Tz`, rise, text and current
  matrices) and a glyph that touches an area is **cut out of the string by its character code**: its shape,
  and the text its ToUnicode map gives, leave the file. The gap is closed with a `TJ` adjustment of exactly
  the glyph's advance (width + `Tc` + `Tw`), so the text around it does not move. Boxes come from
  `/Widths` (simple fonts, Type 3 with its font matrix), `/W` and `/DW` (CID fonts, two-byte codes or the
  first codespace range of an embedded CMap), `/MissingWidth`, and a Helvetica table for the standard
  fonts. A **Type 3 glyph's ink box is read from its `d1`** operator; Chrome draws Arabic dots as
  zero-advance Type 3 glyphs whose box lies beside their origin, and without it they were missed. Any other
  zero-advance mark gets a span around its origin (over-redaction of a neighbour's mark is the lesser harm).
  A glyph is cut when it overlaps an area by more than 3 % of its width (at most half a point), so a mark
  made from a search hit does not eat the next letter.
- **Images.** The pixel whose centre falls in an area (through the whole matrix, so rotated and skewed
  placements work, with half a pixel of margin) is overwritten with black (the darkest palette entry for
  indexed images; stencil masks are cleared), in a **copy of the image used by that one drawing**, under a
  fresh resource name: the original may be drawn elsewhere, and the original name is removed from the page's
  resources when no drawing of it is left, so the unredacted pixels are not reachable from the page.
  Supported: unfiltered or Flate, 1/2/4/8 bits, Gray, RGB, CMYK, ICC, Indexed, Cal, Lab, Separation.
  **JPEG, JPEG 2000, CCITT, JBIG2, and 16-bit images are dropped whole** (reported as `imagesDropped`),
  not left holding what was meant to go; see "Open".
- **Paths.** A filled rectangle (`re f`) under an axis-aligned matrix is **cut into the pieces outside the
  area**. Any other path with a point (vertex or control point) inside an area is dropped (a clip it carries
  stays). A path that only passes under an area keeps its shape: it holds no information inside the area, and
  the black box covers it.
- **Form XObjects** are rewritten the same way, in copies, recursively (depth ≤ 12, cycles refused), when
  their transformed `/BBox` meets an area. Hidden-content sanitizing rewrites every form.
- **Annotations** whose `/Rect` meets an area are removed from `/Annots`, with their pop-ups and replies.
  A **widget** stays but loses `/V`, `/DV`, `/AP` and its field's (and ancestors') values: the field outlives the
  widget in `/AcroForm`.
- **Thumbnails** (`/Thumb`) of redacted pages are dropped.
- **Everything else that may mention the text** is scrubbed (`scrub.rs`): when a mark came from a search or a
  selection it carries its text, and every text string in every object (document information, outline
  titles, annotation contents, form values, structure-tree alternates, named destinations) and the XMP
  packet is searched with the search engine's own folding and the matches replaced by `[redacted]`. Only
  strings that are text (UTF-16BE or UTF-8 with a byte-order mark, or ASCII) are touched, so palettes and keys
  are never corrupted. A mark drawn by hand has no text to scrub.

### 2. Applying is an incremental update, saving drops the rest; a signed file is rewritten at once

Applying writes an update (undo is the history's, ADR 0014). The earlier revision still holds the old
content in the *in-memory* bytes, as any edit does; **saving rewrites the file in full through PDFium
(ADR 0015), which drops every object nothing refers to**, and that is what puts the redacted content out of
the file. (A test saves and scans the decompressed result with qpdf.) A **signed** document is saved
incrementally, which would keep the old bytes, so `Document::with_redactions` returns a **snapshot** (a
full rewrite) for it instead of an update. Its signature stops validating; no redaction of a signed file can
avoid that, and the dialog's undo-until-saved note is the only warning for now.

### 3. Check the result before accepting it

`with_redactions` opens the new revision and extracts its text with PDFium (the same extraction search and
copy use); if any glyph whose centre is inside a mark is still readable, it returns `Error::Redaction`
(`redaction_failed`) and nothing changes. This guards what the rewrite cannot know: a font whose widths are
wrong, a vertical writing mode, a Type 3 glyph with no `d1`. It is a floor, not a proof: it sees text, not
pixels or vectors.

### 4. Pattern search

`redact::find_matches` runs a regular expression, or a preset, over the document's search index. The
pattern runs on **folded text** (ADR 0008 and spec §6.2): Arabic-Indic and Persian digits are 0–9,
tashkeel and tatweel are gone, alef variants are one letter, and every match maps back to the original
range and, through its spans, to glyph boxes, which are merged into one box per run on a line.
That makes the presets Arabic-aware without a second rule set: `٠٥٥ ١٢٣ ٤٥٦٧` is a phone number, `١٠٢٣٤٥٦٧٨٩` an
ID. Presets: e-mail, phone (8–15 digits), **IBAN (mod-97 checked, and trimmed when the pattern ran into
the next word)**, ID number (9–14 digits), card number (**Luhn checked**). Matches are logical-order ranges
of a line, so bidi needs no handling here; their boxes are the glyphs' own, which are wherever the bidi
algorithm put them.

### 5. Sanitize

`redact::sanitize_update` unlinks, in one update: metadata (`/Info` is emptied in place, because readers
merge trailers across revisions and a trailer without `/Info` would leave the old one visible; `/Metadata` and
`/PieceInfo` everywhere), JavaScript (`/AA`, `/XFA`, the `/JavaScript` name tree, `/A` and `/OpenAction` that
run a script, launch, or import data), attachments (`/EmbeddedFiles`, `/AF`, file-attachment annotations),
comments (every annotation but links and widgets), and hidden content (invisible text, text lying wholly
off the page box, and everything in optional-content groups switched off in the default configuration,
including forms and images with an `/OC`). `Document::with_sanitized` then **rewrites the file in full**
(PDFium), which is what removes the unlinked objects, and the step is always a snapshot. Unused resources
(fonts and images no page draws) are not pruned; they go only if no page refers to them.

### 6. UI

The Protect tool (the rail's shield) opens the panel in the inspector: drag on a page to mark an area
(`redact.addArea`), mark selected text from the selection toolbar (`redact.markSelection`), find text,
a regular expression, or a preset and mark one hit or all (`redact.find`, `redact.findPreset`,
`redact.markHit`, `redact.markAllHits`), review and remove marks, apply (`redact.apply` asks first,
`redact.confirm` runs it), and sanitize. Marks live in the UI until applied, so **undo before apply** is
`edit.undo` stepping back through the marks while the tool is open (`redact.undoMark`/`redoMark` do the
same); applying is one engine undo step. Marks are hatched and outlined in `--color-redact-*`, drawn by
the viewer's overlay; hits are listed with their context in `<bdi>`.

## Not supported, on purpose (each fails closed or says so)

- **Encrypted files** (`edit_unsupported`): new objects and rewritten streams would have to be encrypted
  and the reader does not decrypt streams. Fail closed rather than half-redact.
- **Page content in a filter other than none/Flate** (LZW, ASCII85 chains): the page is refused.
- **Vertical writing** (`Identity-V`) is laid out as horizontal; the extraction check catches the leftover.
- **Shadings, patterns, and soft masks** are not edited; they are covered by the black box.
- **Redaction does not alter the original file** until it is saved: the undo stack and the autosave recovery file
  (ADR 0040: the file as opened plus each update) keep the old content on the user's own disk until
  the document is saved or closed.

## Open

- **JPEG (DCT) images** are dropped whole. Scanned pages are usually one JPEG, so redacting an area of a
  scan removes the picture of the page (reported in the summary). Editing the pixels takes a JPEG decoder and
  encoder: `zune-jpeg` (MIT/Apache/Zlib, 0.5.16 is a release candidate) and `jpeg-encoder`
  (`(MIT OR Apache-2.0) AND IJG`, an IJG licence not on the spec's list). **Needs the owner's decision.**
- **OCR text layers** (M4) are text like any other here, and a scan's hidden text under a mark is cut; but the
  *image* under it is the JPEG problem above.
- The standard 14 fonts use Helvetica's widths where a PDF names a standard font and gives none.

## Verified

`crates/safha-core/tests/redact.rs` (hand-built PDFs; the generated corpus; qpdf; PDFium extraction and render):

- English: text cut, neighbours' pixels unchanged outside the mark (zero differing), black box inside, the
  saved file's decompressed bytes hold no trace in UTF-8 or UTF-16, unrelated metadata kept, one update,
  undo and redo; a card preset removes only a Luhn-valid number; 24 image pixels overwritten in a copy and the
  original red image gone from the saved file; a rectangle cut around a hole; marks over nothing change only
  their own box; bad marks, and encrypted files, refused.
- Arabic and English on Chrome (Type 3 fonts, per-glyph `Td`) and CoreText (simple fonts) fixtures
  (`personal-data`: Arabic-Indic IDs and phone numbers, an e-mail, an IBAN): the presets find them, Western
  digits find Arabic-Indic digits, an Arabic word is cut and the rest of its line stays, a marked area removes a
  whole Arabic line, and nothing is found in the saved file in either encoding.
- **Every text block of every corpus fixture** (all producers, derived files, object streams) can be redacted
  with the readable-text check passing.
- Sanitize: metadata, scripts, attachments, comments, and the three kinds of hidden text go and visible text
  stays; options apply separately; **every corpus file sanitized keeps its text and passes `qpdf --check`**.
- Unit tests: the tokenizer (inline images with binary data, comments), rectangle cutting, matrix order,
  the image bit packing, the scrubber (UTF-16, Arabic through digit styles, binary strings untouched),
  the presets, IBAN and Luhn.
- UI (`redaction.test.tsx`, `redaction.test.ts`): the commands end to end on the mock backend, undo before apply, the
  panel and the confirmation in English and Arabic, marks reset when pages change.
- By eye in the browser build (mock backend), dark theme, English and Arabic.

**Not checked:** a signed file's snapshot path (no signed fixture); the native shell's drag-to-mark on a
real PDF (the self-test does not drive it); JPEG, JPEG 2000, CCITT, and JBIG2 images (dropped whole, untested
beyond the code path); fonts with `Identity-V` or custom CMaps that are not identity.
