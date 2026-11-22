# 0015 — Saving, and asking before unsaved changes are lost

Status: Accepted (Milestone 2). Date: 2026-11-22.

## Context

ADR 0014 keeps edits as incremental updates in memory. The spec (§9) says:

- the original file is opened read-only;
- saving goes temp file → validate → atomic rename;
- a signed file is saved incrementally;
- Save As is always available.

Nothing may be lost silently: a modified document has to ask before it closes.

## Decision

### What a save writes (`safha_core::save`)

| Document | Bytes written | Why |
|---|---|---|
| Signed (`FPDF_GetSignatureCount` > 0) | The file as opened, plus each edit's update, unchanged | Every byte a signature covers stays as it was, so the signatures stay valid |
| Anything else | PDFium's full rewrite (`save_to_bytes`) | It drops objects nothing references any more |

**Measured:**

- After deleting a page, the page's compressed content stream is still in the edited bytes (an
  earlier revision) but gone from the saved file.
- PDFium's rewrite keeps an encrypted file encrypted. It opens only with its password, and
  `qpdf --check` passes.

**Validation.** The chosen bytes are opened again, with the document's password, and must have
the same page count. If not, nothing is written (`save_failed`).

`qpdf --check` runs in the tests, not in the app: qpdf is not bundled. Bundling it (Apache-2.0)
is a packaging decision for M7.

**Writing (`write_atomically`):**

- a temporary file beside the target (`.name.safha-<pid>-<n>.tmp`; a rename is atomic only
  within one file system);
- `sync_all`;
- the existing file's permissions copied;
- a rename over the target, then the directory synced on Unix.

A failure removes the temporary file and leaves the original untouched (`io_error`).

**Save As** asks with the native dialog (Rust side; the webview has no dialog permission) and
adds `.pdf` when missing. From then on the document *is* the new file: its name, recents, and
session.

**After a save** the history marks the saved revision. Undo past it makes the document modified
again. The in-memory document keeps its revision chain, so the next save rewrites again.

### Unsaved changes

- **A dot on the tab** (`role="img"`, labelled "Unsaved changes") shows a modified document.
- **Closing a modified tab** asks: Save, Don't save, or Cancel. Every answer is a command
  (`file.saveAndClose`; `file.close` with `force`; `app.cancelClose`), as the registry rule
  requires.
- **Closing a window** that holds a modified document:
  - The shell stops `CloseRequested` and emits `close-requested` to that window only, whose UI
    asks Close without saving / Cancel.
  - `close_window_discarding` marks the window as confirmed and closes it.
- **Quitting** (`ExitRequested`) with modified documents anywhere:
  - The shell prevents the exit and emits `quit-requested` to the window of the first such
    document.
  - `quit_discarding` confirms and exits.
  - The session is kept as on any quit.
- **Locks:** `AppState::unsaved` collects each file's history handle under the document-map lock
  and only then locks the histories. An edit takes them in the opposite order, so holding both
  at once could deadlock.

## Consequences

- The window and quit dialogs offer no "save all". A user who wants to keep changes cancels and
  saves each document. A save-all can come with autosave (M2, slice 7).
- **Not yet verified on screen:** the macOS Quit menu (`⌘Q`) path. The self-test verifies the
  window path (`selftest_request_close`, a devtools command, because the webview may not close
  windows).
- **Incremental saves of signed files** append updates. Edits that are not allowed by the
  signature's permissions (`/DocMDP`) still invalidate it in viewers that check. Checking
  `/DocMDP` before editing is M5 (signatures).
- **Large files:** a full rewrite holds the new file in memory once more, on top of the
  document. That is fine at today's sizes. Streaming PDFium's writer to the temporary file would
  remove it.

## Verified

- `tests/save.rs`:
  - a deleted page's stream is absent after saving;
  - an encrypted file stays encrypted;
  - a signed file is saved incrementally, its original bytes a prefix of the saved file;
  - saving over the open file works.
- `save.rs` unit test: the file is replaced and no temporary file is left; a missing directory
  fails cleanly.
- UI tests:
  - Save is enabled only when modified;
  - undo past a save is modified again;
  - Save As renames the tab;
  - closing a modified tab asks, Cancel keeps it, and "Don't save" closes it.
- In the real app (self-test, English/LTR and Arabic/RTL): asking to close the window while a
  page is duplicated shows the prompt and leaves the window open.
- The dialog checked visually in the Arabic UI: right to left, Save focused first.
