# 0023 — A page's text as editable blocks, traced to its content

Status: Accepted (Milestone 3). Date: 2026-11-22.

## Context

Spec §7's editing pipeline begins with two steps:

1. **Analyse the page:** read text objects with font, size, matrix and colour, cluster them into
   lines and blocks, and detect direction, alignment, line spacing, indentation and lists.
2. **Edit model:** each block becomes an editable rich-text model, as styled runs in logical
   order.

Step 5, write-back, then removes "the original text objects for that block" and writes new ones,
"keeping z-order and clipping". So the model must know exactly which bytes of the page's content
drew each block. PDFium regenerating content is ruled out (ADR 0005's amendment); our writer
edits content as incremental updates (ADRs 0014, 0021).

The text extractor (ADR 0008) already rebuilds lines and blocks in reading order from PDFium's
characters, correctly for Arabic.

- **What PDFium gives:** each character's font name, size, fill colour, origin, and text object.
- **What it does not give:** a way to identify a text object in the content stream (its handle
  is private in pdfium-render).

## Decision

### Reading the content (`pdf::content`)

A small interpreter of a page's content streams lists every text-showing operation (`Tj`, `TJ`,
`'`, `"`). It tracks:

- the graphics state (`q`/`Q`, `cm`, the fill colour by its components);
- the text state (`Tf`, `Tc`, `Tw`, `Tz`, `TL`, `Ts`, `Tr`);
- the text and line matrices;
- marked content.

Each operation records:

- its stream and byte range;
- its text object;
- its font resource and size;
- its fill colour;
- whether it is inside `/MirsamText`;
- **its starting point in page space:** `(0, rise)` through the text matrix and the CTM, which
  needs no font metrics. A show that follows a show with no positioning between them continues
  the pen and has no starting point of its own.

Inline images are skipped. Form XObjects are not entered.

### The edit model (`Document::edit_blocks`)

For each of the extractor's blocks:

- **its text** in logical order, lines joined by `\n`, with each line's baseline and edges;
- **styled runs** (font name without its subset tag, size, colour), from PDFium's per-character
  data, merged where equal. Inferred spaces take the style before them.
- **alignment** from the lines' edges, in the block's direction:
  - left, right, centred, or justified (`align_seen`);
  - justified needs three lines or more, since two lines cannot tell ragged text from justified;
  - one line takes its direction's start, flagged as assumed;
- **line pitch:** the median distance between baselines;
- **its sources:** the operations that drew it, plus the ones that continue them;
- **`untraced`:** characters traced to no operation;
- **`shared`:** an operation that also draws another block;
- **`written_here`:** text this app wrote (ADR 0021).

### Tracing characters to operations

What the corpus taught, in order:

1. **Character order is not content order.** PDFium makes one text object per show operation,
   but numbers a right-to-left run's characters in reading order, while Chrome draws Arabic left
   to right, one glyph per operation. Tracing therefore matches by position, not by sequence.
2. **Characters are grouped by their PDFium text object** (its bounds and matrix). Each group is
   given an operation not yet given to another group, that starts at one of the group's origins
   (within a tenth of a point), with the same font.
   - The groups with the fewest choices choose first: a letter and the mark drawn from its
     origin compete for the same starting points.
3. **An offset mark** has no exact match: it takes the nearest operation left within half an em.
4. **A continuation with no starting point** goes to a group on the same baseline as the
   operation before it.
5. **Word spaces in no text object** (PDFium reports them at the next letter's origin) take no
   operation, and do not count as untraced.
6. **Operations that draw no character at all** belong to the block they are drawn in: the
   smallest block whose box holds their starting point. Examples are the dots Chrome's Type 3
   fonts and CoreText draw apart from Arabic letters, which have no Unicode mapping.

### Removing a block (`text_edit::remove_text_update`, `Document::without_text`)

The write-back's first half:

- each content stream holding the block's operations is written again, with those operations'
  bytes blanked out;
- the new stream gets a new number, so a stream other pages share is left alone;
- the page's `/Contents` points at the new streams.

Every other operator keeps its bytes, so graphics, z-order and clipping are untouched. Text
positioned by `Td`/`Tm` after a removed show is unaffected, since positioning is relative to the
line matrix, not the pen. Encrypted files work. Removal is an incremental update, so undo covers
it.

## Results (macOS fixtures, page 1 of each)

- **146 blocks, 8,450 characters.** 6 characters trace to no operation: marks on two CoreText
  pages (lam-alef and vocalized Arabic). Every other block traces completely.
- **The removal check, on all 144 fully traced blocks:**
  - Chrome, CoreText, encrypted, linearized, turned pages, an object-stream book;
  - each block's operations were removed and PDFium rendered the result;
  - **no pixel changed outside any block** (3 pt margin), and its ink, where no other block
    overlaps it, went (at least 90% fewer dark pixels; in practice all of it);
  - `qpdf --check` passes, with the password for the encrypted file.
- **Styles read back:**
  - CoreText's `IBMPlexSans-Regular` at 20 pt (the heading) and 12 pt;
  - `NotoNaskhArabic-Regular` for Arabic, right-aligned;
  - text this app wrote: justified Arabic in Amiri at 14 pt in red, and centred English in IBM
    Plex Sans at 12 pt in blue. Both read back with their font, size, colour, alignment and line
    pitch, marked as written here.
- **Unchanged:** the extraction corpus stays at 86 of 118 blocks verbatim. The span and line
  data added for this (`chars`, `baseline`) is not sent to the UI.

## Consequences

- **Not yet seen by the analysis:**
  - indentation and lists;
  - text inside form XObjects, which is listed as untraced;
  - content filtered with anything but Flate;
  - Type 3 fonts, which have no name (runs report an empty font).
- **Gate for editing in place:** a block with `untraced > 0` or `shared` cannot be edited in place
  safely. Its remaining glyphs would stay, or another block's would go. The editor must refuse
  or warn.
- **Matching is per page and greedy.** Two operations at the same point with the same font (fake
  bold drawn twice) are told apart only by order. Both belong to the same block, so removing
  that block takes both.
- **The orphan rule cuts both ways.** It gives a block any character-less operation inside its
  box. That is what editing the block means visually, but it would also take invisible text
  without a Unicode mapping, if a producer drew some there.
- **Platforms:** CI's Linux and Windows fixtures come from their own Chrome. The corpus test
  asserts shares (at most 0.5% of characters untraced, at least 98% of blocks removed cleanly),
  not macOS counts.

## Verified

- **Unit tests:**
  - **the content reader:** origins through `Td`, `cm` and `Ts`; continuations; `'` and `"`;
    colours in grey, RGB, CMYK and `sc`; marked content; inline images and strings that contain
    operator names; content split across streams;
  - **tracing:** right-to-left objects whose starting character comes last; continuations;
    text in forms left untraced;
  - **alignment**, including two-line paragraphs;
  - **subset tags.**
- **`tests/page_analysis.rs`:** the corpus check above, styles of fixtures and of text this app
  wrote, and an encrypted file's block removed (with `qpdf --check`).
