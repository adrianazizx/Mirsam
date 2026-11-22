# 0016 — Pages across files: insert, merge, extract, split

Status: Accepted (Milestone 2). Date: 2026-11-22.

## Context

Spec §5.4 (P0): merge files, insert pages from another PDF, extract, split.

Within one file, ADR 0014's incremental writer moves page objects around. Across files it would
have to copy each page's whole object graph (content streams, fonts, images, annotations) under
new numbers. In an encrypted source, every string and stream of that graph is encrypted under
its old number, and this writer does not decrypt.

PDFium's `FPDF_ImportPagesByIndex` does all of that, through pdfium-render's safe
`copy_page_range_from_document`.

## Decision

- **Extract** (`Document::extract_pages`):
  - an empty PDF (`Engine::new_pdf`), the chosen pages imported in the chosen order, saved;
  - written where the user chooses, through Save As;
  - the new file has no outline and no encryption.
- **Split** (`Document::split_every`):
  - parts of N pages, each made like an extract;
  - written into a folder the user picks as `name (1).pdf`, `name (2).pdf`, …;
  - an existing file is never overwritten: `name (1) 2.pdf` instead.
- **Insert pages from files, which is also merging** (`Document::with_pages_from`):
  - The pages of the picked files are imported into a working copy of the current revision,
    which PDFium then writes in full.
  - Inserting lands after the selection in the organizer, else at the end, which makes it
    a merge.
  - The inserted pages are selected afterwards.
- **History:**
  - A full rewrite cannot be an incremental update, so `EditHistory` gains *snapshot* steps:
    a revision is the newest snapshot in effect (or the file as opened) plus the updates after
    it.
  - Undo past a snapshot returns to the revision before it, byte for byte. Later page edits
    append updates to the snapshot as usual.
- **Validation:** every file written is opened again and its page count checked before an
  atomic write (ADR 0015's `write_atomically`).
- **Refused:**
  - inserting into a *signed* document (`edit_unsupported`): a full rewrite would invalidate
    its signatures;
  - inserting from a source that needs a password (`password_required`).
- **Telling the user:** extract and split write files elsewhere, so a *notice* (`role="status"`,
  accent border) says what was written. It shares the error banner's place and dismissal.
- **Split dialog:** asks for pages per file. It suggests half the document, rounded up, and
  disables confirming outside 1 … page count − 1.

## Consequences

- **Copies, not shared objects:** inserted pages are copies. A file that inserts the same source
  twice carries its fonts twice.
- **Page labels** (`/PageLabels`) are defined by page index, so they stay with positions, not
  pages. Inserted pages take the labels of the positions they land in, as with any page tree
  change. Re-basing labels is an M6 item along with label editing.
- **Extracting from an encrypted file** gives an unencrypted file. The reader opened the original
  with its password, so this is the user's own choice. The owner's permission flags (for example
  "assembly not allowed") are not enforced yet; that comes with M5 (security).
- **Not yet:** a signed document gets no incremental insert, sources with passwords cannot be
  inserted (no prompt in the insert flow), and there is no split by size or by bookmarks (spec
  §5.4 names them). Each is a later step.

## Verified

- `tests/combine.rs`:
  - extracting pages 3 and 1 gives a two-page file in that order;
  - splitting three pages by 2 gives files of 2 and 1 pages;
  - an encrypted file's pages extract;
  - inserting a file twice after page 1 gives five pages in the right order, keeping the
    document's outline and its first label;
  - undo after the insert restores the original text;
  - every result passes `qpdf --check`.
- `EditHistory` unit test: undo and redo across a snapshot.
- UI tests: inserted pages are selected, and extract and split leave their notices.
- In the browser preview: the organizer's new buttons, the split dialog, and the notice. In the
  real app the shell's file dialogs are untested: they need a person at the machine.
