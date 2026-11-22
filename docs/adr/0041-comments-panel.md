# 0041 — The comments panel: threads, replies, review states, and authors

Status: Accepted (Milestone 2). Date: 2026-11-22.

## Context

Spec §5.2 (P1): "Comments panel: filter, sort, reply, resolve". Spec §4 puts a comments list in
the left panel. Annotations must be standard, so that other viewers show the same threads.

ISO 32000-2 models a review thread with plain annotations:

- **A reply** is a markup annotation, usually `/Text`, whose `/IRT` ("in reply to") names the
  annotation it answers.
- **A review state** (12.5.6.10) is a `/Text` annotation with `/IRT`, a `/StateModel`
  (`/Review` or `/Marked`) and a `/State`. The `/Review` states are `/Accepted`, `/Rejected`,
  `/Cancelled`, `/Completed` and `/None`. Other viewers write each change of state as a new
  annotation, so the latest one counts.
- **The author** is `/T`.

PDFium's safe API (pdfium-render 0.9) reads `/Contents`, `/T`, `/M` and `/CreationDate`, decrypted.
It does not read `/IRT`, `/StateModel` or `/State`.

## Decision

### Reading threads (`annotate::comments`)

- **Our own parser reads the structure**: for each page, `/IRT`, `/RT`, `/StateModel` and
  `/State`. These are references and names, which are never encrypted. **PDFium reads the text**:
  contents, authors and dates.
- **A reply to a reply** belongs to the thread of the comment at its start; a cycle in a damaged
  file stops at its first repeat.
- **`/RT /Group` members** are part of the annotation they name and are not listed.
- **`/Marked` states** belong to a thread but do not resolve it.
- **Resolved** means the latest `/Review` state is `/Completed`. "Latest" is by `/M`, then by
  position in `/Annots`.
- **`Document::comments`** lists, page by page, every annotation of a kind the app knows (markup,
  ink, notes, text boxes) that answers nothing. Each comes with:
  - its author and date (dates parsed with their offsets);
  - its text;
  - for markup, the page's text under it. `PageText::text_in` takes the spans whose centre lies in
    a quad, in reading order, so Arabic reads back in logical order.
  - its bounds;
  - its replies, oldest first;
  - whether it is resolved.
- **Pages without `/Annots`** are skipped before PDFium loads them.
- **`page_annotations`** (what the page view hit-tests) now leaves replies and states out. They are
  not on the page, so they cannot be clicked there.

### Writing (`AnnotationEdit::Reply`, `AnnotationEdit::SetResolved`)

- **A reply** is a `/Text` annotation with `/IRT`, `/RT /R`, `/Contents`, and the comment's
  `/Rect`.
- **Resolving** appends a state: `/StateModel /Review` with `/State /Completed`. **Reopening**
  appends `/State /None`. Earlier states are kept, as other viewers do.
- **Both draw nothing.** They have an empty appearance stream; without one, viewers (PDFium
  included) draw a note's icon. Flags: a reply has `/F 28` (print, no zoom, no rotate); a state is
  also hidden (`/F 30`), following other viewers' files.
- **Both are incremental updates**, so undo covers them, and they are encrypted in encrypted files
  as any new object is (ADR 0017).
- **Only a comment can be answered.** The target must be an object of its own, of a known kind, and
  answer nothing itself. Empty replies are refused.
- **Removing a comment** now removes its whole thread: replies, replies to those, states, and their
  pop-ups.

### Authors

- **New annotations of every kind carry `/T`** when the user has chosen a name: the
  `commentAuthor` setting, set in the panel's footer ("Your name on comments").
- **No name is guessed.** The system's account name is not written into documents unasked; with no
  name chosen, `/T` is left out and the panel says "No name".
- The shell reads the stored setting when it writes an annotation. Names are trimmed, at most 100
  characters in the UI (the engine allows 256).

### The panel (`CommentsPanel`)

- **A fourth view of the left panel.** It is reloaded whenever the document's annotations or pages
  change.
- **Filters:** status (all, open, resolved), type (text markup, notes, text boxes, drawings), and
  author. The author filter is shown when there are two or more authors.
- **Order:**
  - page order, top to bottom, under page headings;
  - latest activity, where a reply counts as activity;
  - by author, in the UI language's collation, with unnamed comments last.
- **Each comment shows:**
  - its type's icon, author and date;
  - the marked text, as a quotation;
  - its own text;
  - its replies, indented along the inline start;
  - Reply and Resolve (or Reopen).
- **Choosing a comment** goes to its page, scrolls it into view a little below the top, and selects
  it as a click would (a note opens its editor). This is `ViewerController.showAnnotation`.
- **Text in either script** keeps its own direction (`dir="auto"`); names are in `<bdi>`.
- **Every action is a command:**
  - `view.showComments`;
  - `comment.show`, `comment.reply`, `comment.resolve`;
  - `comments.filter`, `comments.sort`, `comments.setAuthor`.

## Consequences

- **Only kinds the app knows are listed.** Shapes, stamps, callouts and file attachments from other
  tools are not listed yet, nor are replies to them. They come with their own tools (P1).
- **Pages in encrypted object streams.** Where our parser cannot read the page (ADR 0014), threads
  are unknown: each reply is listed as a comment of its own. Such files cannot be edited anyway.
- **Not yet:**
  - editing or deleting a single reply from the panel (the engine can remove one);
  - adding a comment to a markup annotation's own `/Contents` (a reply is used instead);
  - XFDF import and export, summaries, and flattening (P1).
- **Cost:** listing loads each annotated page in PDFium, and reads its text when it has markup.
  This was not measured on very large annotated documents. The panel lists lazily from the shell,
  off the UI thread.

## Verified

- **Unit tests:**
  - dates with offsets, and the round trip of the dates we write;
  - the entries of a reply and of a state, and their flags;
  - filters and orders (`state/comments.test.ts`).
- **`tests/comments.rs`:**
  - **A thread written and read back:** a highlight by "Reviewer", whose marked text reads back
    exactly, and an Arabic note by an Arabic name.
    - Two replies are listed in order and draw nothing (identical pixels), and are not on the page.
    - Resolving, then reopening, makes the latest state count.
    - Replies to replies, empty replies, and missing targets are refused.
    - Removing the highlight takes its replies along.
    - `qpdf --check` passes.
  - **Arabic markup** reads back its marked text in logical order. With no name chosen, none is
    written.
  - **An AES-256 encrypted file:** an Arabic author, an Arabic reply, and resolving all decrypt
    and read back; `qpdf --check` passes with the password.
  - **A thread as other viewers write one** (hand-made file):
    - replies to replies, dated out of order, with time-zone offsets;
    - a newer `/Completed` state listed before an older `/Rejected` one;
    - a `/Marked` state and a group member.

    All are followed. Removing the comment removes the whole thread.
- **Shell test:** with no name chosen, no `/T` is written. A chosen Arabic name signs new notes and
  replies. Bounds arrive in displayed-page points.
- **UI tests:**
  - **In English:** the panel lists the comments, and a chosen name signs the next one. Page
    headings, a reply through the form, resolve, the filters (and their empty state), undo, and
    "show" are covered.
  - **In Arabic:** the empty state, the panel's tab, a page heading, and "No name".
- **Browser preview:**
  - **English and Arabic:** the panel mirrors, threads indent along the inline start, mixed text
    keeps its own direction, and choosing a comment selects it on the page.
- **Real-app self-test, English and Arabic (right to left, dark):** the panel lists the note, the
  text box and the stroke. A reply and a resolve read back from the file, and are not page
  annotations. Undo takes them back.
