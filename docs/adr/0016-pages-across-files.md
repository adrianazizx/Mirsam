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
- **Not yet:** a signed document gets no incremental insert. (Sources with passwords and
  splitting by size or bookmarks came in session 8; see the amendment.)

## Amendment (session 8): splitting by bookmarks and size; protected sources

- **Split at top-level bookmarks** (`Document::bookmark_parts`, `split_ranges`):
  - one file per top-level outline entry that points into the document, from its page to the
    next entry's;
  - pages before the first entry go with it;
  - entries on a page an earlier one already starts add nothing;
  - fewer than two parts means no split (the dialog disables the option, the shell answers
    `no_bookmarks`).

  Files are named `name (n) title.pdf`. The title loses path separators, characters file
  systems refuse, control characters, and bidi overrides (which can make a name display as
  something it is not), and is cut to 60 characters.
- **Split by file size** (`Document::split_by_size`):
  - each file holds as many pages as fit under the limit, found by doubling, then halving the
    gap, and measured by writing the part (so fonts shared by its pages count once per file);
  - a page larger than the limit alone gets a file of its own.

  The dialog takes megabytes (10⁶ bytes, as file managers count). The parts are computed off
  the UI's thread, after the folder is chosen.
- **Inserting from password-protected files:**
  - each picked file that needs a password stops the insert, which is held under a token
    (`InsertOutcome::NeedsPassword`);
  - the page area's password prompt asks for that file (wrong passwords ask again), and
    `unlock_insert` goes on to the next file;
  - once all are open, they are inserted as one step;
  - cancelling forgets the held insert.

  Passwords are used once and not kept, as for opening. The inserted pages are unencrypted
  copies in the target, as with extract.
- **Verified:**
  - a hand-made five-page file with an Arabic entry, a duplicate, a child and an entry that
    points nowhere splits into the three expected files, each passing `qpdf --check`;
  - the RTL book split by size: every part within the limit, and none could have taken its
    next page; one page per file below any page's size; one file above the whole;
  - safe file names, including a right-to-left override;
  - the shell holds an insert for a protected file, asks again after a wrong password, and
    inserts with the right one;
  - UI tests for the dialog's three ways and the protected insert.

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
