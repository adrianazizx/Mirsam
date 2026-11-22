# 0040 — Autosave and crash recovery

Status: Accepted (Milestone 2). Date: 2026-11-22.

## Context

Spec §5.12 (P0) asks for autosave and crash recovery. Spec §9: "Autosave to a recovery file in
the app data directory every N seconds while dirty". Spec §2's first quality bar is to never lose
or corrupt a file, and the original stays untouched until an explicit save (ADR 0015).

Edits are incremental updates kept in memory (ADR 0014): a document's current revision is the
file as opened, with each edit's update appended. Until this slice, a crash, a power cut or a
killed process lost every unsaved edit.

## Decision

### What is kept

- **The recovery file is the current revision's bytes,** a valid PDF by itself. No journal format
  of our own is needed: restoring is opening a PDF.
- **Where:** `<state dir>/recovery/<key>.pdf`, next to the state store. Keys are chosen by the app
  (`<millis>-<pid>-<id>`) and limited to letters, digits and dashes, so a key cannot reach outside
  the directory.
- **Written atomically** (`write_atomically`, as saves are): a crash mid-write leaves the previous
  recovery file.
- **What it belongs to** is a row in the state store's new `recovery` table (schema version 3):
  - the original path and display name;
  - when it was written;
  - the original file's size and modification time (`FileStamp`) when it was opened or last
    saved.
- **Encrypted documents** keep an encrypted recovery file: it is the same bytes. The password is
  never written; restoring asks for it again.

### When

- **After every change** (an edit, undo or redo), a document's recovery file is brought up to
  date **2 seconds later**, unless the document changes again meanwhile (a generation counter per
  document). A burst of edits, such as several deletions, writes once. The spec's "every N
  seconds while dirty" becomes "N seconds after the last change": nothing is written while
  nothing changes.
- **A document with no unsaved changes has no recovery file.** The delayed write removes it
  instead when the document is back at its saved revision (undo past the edits), and checks again
  after writing, in case a save landed meanwhile.
- **Removed** when the document is saved, closed without saving (a tab, a window), or when the
  app quits discarding changes.

So a recovery file left at startup means a session ended without saving or discarding.

### Restoring

- **At startup** the main window's first `startup` call lists the recovery files that belong to
  no open document. Entries whose file is gone are forgotten. The UI gets tokens, names and times,
  never paths.
- **The recovery dialog** (a modal: this is not a routine action) lists each document with its
  last change. Each one is restored or discarded on its own; "Decide later" keeps them all for the
  next start. Every answer is a command (`recovery.restore`, `recovery.discard`,
  `recovery.later`).
- **A restored document** opens with unsaved changes, under its original path and name:
  - **When the original is unchanged** (same `FileStamp`), the history is
    `EditHistory::recovered(original, recovered)`. Undo goes back to the file as it is on disk,
    and Save writes over it as usual.
  - **When the original changed or is gone,** the history starts at the recovered bytes
    (`EditHistory::unsaved`), and the document is marked "save as first". Save then answers
    `save_as_required`, and the UI asks where (Save As). The dialog says so beforehand.
  - **The original's tab,** when the restored session reopened it, gives way to the recovered
    document.
  - The restored document keeps its recovery key, so a second crash keeps its changes too.
- **An encrypted recovery file** answers with the usual password prompt (`unlock_recovery`). A
  wrong password asks again; cancelling leaves the file for the next start.

## Consequences

- **A full copy per document with unsaved changes.** For a large scanned file each burst of edits
  writes the whole file again. Writes happen off the UI thread, after edits pause, and only while
  a document is modified. Appending only the new updates to the recovery file would be cheaper;
  it is not needed yet.
- **Edits are kept 2 seconds after they happen.** A crash within 2 seconds of an edit loses that
  edit.
- **Two copies of the app** keep their own recovery files (the key holds the process id), and each
  lists only files that no document of its own holds. A second copy started while the first is
  still running would offer the first one's files. Running a single copy is not enforced yet; it
  is listed as a gap in `PROGRESS.md`.
- **Recovery files hold document contents in the app data directory** (encrypted when the
  document is). They are removed as above; "Decide later" keeps them until they are restored or
  discarded.

## Verified

- **Core unit tests:**
  - a recovery file written, replaced, read and removed;
  - keys that would reach outside the directory refused;
  - a `FileStamp` that changes with the file;
  - store entries kept until forgotten;
  - a recovered history that is modified and undoes to its original.
- **Shell tests (`recovery.rs`):**
  - edits survive a simulated crash; the restored document undoes back to the original, and
    saving removes its recovery file;
  - a changed original is still offered, and the restored document must be saved somewhere new;
  - undo back to the saved revision, and closing, remove the recovery file;
  - an encrypted recovery file asks for its password, again after a wrong one.
- **UI tests:**
  - the dialog offers each document, restores one as modified with undo, and discards the other;
  - only the entry whose original changed says it will be saved as a new file;
  - an encrypted one asks for its password, again after a wrong one;
  - "Decide later", in Arabic;
  - Save falls back to Save As on `save_as_required`, and Save-and-close does not close when the
    picker is cancelled.
- **Browser preview,** English and Arabic: the dialog's layout mirrors, and Restore opens the
  document as a modified tab.
- **Real-app self-test,** English and Arabic (right to left, dark): after a page is duplicated the
  document has a recovery file, and undo back to the file removes it.
