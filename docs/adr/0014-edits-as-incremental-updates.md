# 0014 — Edits as incremental updates, undo as a stack of them

Status: Accepted for page edits (Milestone 2); saving (ADR 0015) and markup annotations
(ADR 0017) build on it. Date: 2026-11-22.

## Context

Milestone 2 starts editing: pages first (rotate, delete, reorder, duplicate, insert), then
annotations. The spec (§9) asks for three things:

- every change recorded in an undo journal that covers text, annotations, and pages alike;
- saves that keep digital signatures valid (an incremental update when the file is signed);
- the original file opened read-only.

What PDFium offers through `pdfium-render`'s safe API, measured against those needs:

| Need | PDFium route | Verdict |
|---|---|---|
| Rotate, delete, insert blank | `set_rotation`, `delete`, `create_page_at_index` | possible |
| Reorder pages | no safe binding for `FPDF_MovePages` | only by re-importing pages, which gives them new objects |
| Keep the outline, links, and form fields pointing at a moved page | they point at the page *object* | lost by re-import |
| Incremental save (signed files) | `save_to_bytes` writes the whole file; the crate does not expose the incremental flag | not possible |
| Undo | no history; would mean saving and reloading whole copies | costly |

`unsafe` is denied workspace-wide, so the raw bindings are not an option.

## Decision

**Each edit is an incremental update** (ISO 32000-1, 7.5.6), written by our own code
(`safha_core::edit`, `safha_core::pdf::write`). The update holds the changed objects and a
cross-reference section whose `/Prev` points to the previous one.

- **How each edit is written:**
  - Rotating rewrites the turned pages' dictionaries with a new `/Rotate`.
  - Deleting, moving, duplicating, and inserting rewrite the root `/Pages` node, with every
    page as a direct kid in the new order.
  - A page moving out from under an intermediate node takes along the attributes it inherited
    from there (`/Resources`, `/MediaBox`, `/CropBox`, `/Rotate`).
  - A duplicate is a new page dictionary sharing the original's content and resources. It
    leaves out `/Annots`, which belong to one page, and structure-tree links.
- **Page objects keep their numbers,** so outlines, links, form fields, and annotations' `/P`
  still point at the right page.
- **The section's form follows the file's newest one:** a classic table after a table, an
  uncompressed cross-reference stream after a stream.
- **Strings are written back as hexadecimal of the bytes read.** An encrypted file's strings
  stay valid because a rewritten object keeps its number. A duplicate of an encrypted page that
  holds strings is refused rather than broken. A page tree inside an *encrypted* object stream
  cannot be read (the reader does not decrypt), and such edits are refused with
  `edit_unsupported`.
- **The parser** is the M1 catalog reader's (ADR 0012), moved to `safha_core::pdf` and shared.

**Undo is a stack of updates** (`EditHistory`):

- Revision *n* is the original bytes plus the first *n* updates.
- Undo and redo move along the stack, and nothing is recomputed. A new edit after an undo drops
  the undone updates.
- It tracks the saved revision, for the "modified" state.
- Storage is the original plus the updates (a 1,000-page move adds 8 KB), never a copy per
  revision.

**Documents keep their bytes and reopen.**

- `Document` keeps the bytes PDFium reads, shared through a reader rather than copied, and
  `with_page_edit` / `revision` open the next revision.
- **The password stays in memory** while the document is open, so an edited revision of an
  encrypted file can be opened. That changes ADR 0010, under which a password was used once and
  dropped. It is still never written or logged.
- **The shell:**
  - It swaps the revision in.
  - `RenderService::replace_document` bumps the render revision, so a tile of the old content
    that is still rendering is not cached.
  - The search index starts over.
  - Each open file's history has its own lock, so a document's edits apply in order.
- **The UI:** it bumps the document's revision, and the page view and thumbnails start over,
  keeping the current page.

## Consequences

- **Saving (next slice):**
  - A signed file is saved with its updates appended, so its signatures stay valid.
  - Any other file is rewritten in full by PDFium (`save_to_bytes`). That drops deleted pages
    and earlier revisions, which would otherwise still be in the file: a deleted confidential
    page must really go.
  - Both go through a temporary file, then validation, then an atomic rename.
- **Annotations (later slice)** will be written the same way: an annotation object, the page's
  `/Annots`, and our own appearance stream. Undo then covers pages and annotations alike, as
  the spec asks.
- **Large files:** each edit concatenates the current bytes once, to reopen. A 200 MB scan
  costs about 200 MB copied per edit (tens of milliseconds) and twice the file in memory
  meanwhile. A reader over the segments would avoid both; it is not needed yet.
- **Content stays reachable:** pages deleted by an edit stay in the earlier revision until a
  full save. The same goes for intermediate page-tree nodes left without a parent. Both are
  harmless to readers.
- **What PDFium can open but this reader cannot** (badly damaged cross-reference tables, page
  trees in encrypted object streams) can be viewed but not edited. The UI says so
  (`edit_unsupported`).

## Measured (debug build, this Mac)

| Check | Result |
|---|---|
| Rotate, delete, move, duplicate, insert on real files | text in the expected page order; every result passes `qpdf --check` and reopens in PDFium |
| Encrypted file (AES-256): insert, then rotate | reopens with its password; `qpdf --check` passes |
| Linearized file; files with object streams and a cross-reference stream | edits apply; the catalog's reading preferences are untouched |
| Outline after duplicating and moving its page | every entry points at the page's new position |
| Undo by dropping the last update | the previous revision, byte for byte |
| 1,000 pages: move the last page to the front | 28 ms; the update is 8 KB |
| In the real app: duplicate, repaint, undo (self-test, EN/LTR and AR/RTL) | 21–25 ms per edit |
