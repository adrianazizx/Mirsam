# 0010 — Navigation, search, selection, and local state in the UI

Status: Accepted (Milestone 1). Date: 2026-11-22.

## Context

ADRs 0008 and 0009 gave the core text extraction, search, outline, page labels, and a tiled
viewer. Milestone 1 still needed the user-facing half (spec §4, §5.1, §6.1–6.2): a left panel
with thumbnails, outline, and search results; bidi-correct selection and copy; recents, session
restore, and per-file position; reading themes; the command palette; and opening
password-protected files — in English and Arabic, LTR and RTL.

## Decision

### Shell commands (thin adapters, `apps/desktop/src-tauri/src/{commands,navigation}.rs`)

- **Files are named by id or token, never by path.** Open documents by id; recent files and files
  waiting for a password by a token minted per listing. The UI still receives file names only.
- **Opening returns an outcome**: `opened { document }` or `needsPassword { token, name, retry }`.
  `unlock_document(token, password)` tries a password once; it is never stored or logged. A wrong
  password returns `needsPassword` again with `retry: true` (PDFium reports both cases as
  "password required").
- **Startup** is one command: the first call restores the last session's tabs and opens the launch
  document; a later call (the UI reloaded) returns what is still open, so a reload re-attaches
  instead of leaking documents in the shell.
- **State store** (`safha_core::store`, SQLite) opens in the app data directory, or in
  `SAFHA_STATE_DIR` — the self-test and benchmarks use a fresh one per run so they never read or
  overwrite the user's recents and session. Settings are a closed list of keys; the webview cannot
  use the store as general storage. Opening the same file twice switches to its tab.
- **Text and search cross IPC in the UI's terms**: UTF-16 string indices (JavaScript), not the
  engine's UTF-8 byte offsets; rectangles on the displayed page (`view::ViewTransform`, which
  follows `/Rotate` and crop-box origins), not PDF page space. The conversion happens at the
  boundary and nowhere else.
- **Search** builds the document's index once (`search::IndexCache`, shared by concurrent searches,
  cancelled when the document closes), returns at most 5,000 hits with context snippets cut at
  word boundaries, and serves page text for selection from the index once it exists.
- **Thumbnails render through their own lane** (ADR 0009 §6) at `Urgency::Thumbnail`.

### UI

- **Left panel** at the inline start (right in Arabic): thumbnails (a virtualized, imperative
  strip like the viewer), the outline (React Aria `Tree`), search (field, five option toggles,
  status, a virtualized `ListBox` of hits with their context in its own direction).
- **Selection model** (`viewer/selection.ts`, pure and unit-tested): a selection runs between two
  carets, each a line in reading order plus a horizontal position. Lines in between are whole; on
  the first line everything visually *after* the caret in the block's direction; on the last,
  everything before; on a single line, between the two. A glyph is selected when the pointer
  passes its middle. Copy emits the selected glyphs' characters in logical order (pieces that are
  not contiguous in reading order, possible in mixed lines, are joined by a space); lines by `\n`,
  blocks and pages by a blank line. This is spec §6.1's "selection moves visually but copies
  logically" made exact. Double-click selects a word, triple-click a line, Shift-click extends,
  Cmd/Ctrl+A selects the current page's text (not the whole document: that would need every
  page's text layer at once). The viewer counts clicks itself: `PointerEvent.detail` is always 0
  in Chromium.
- **Text layers load on hover**, not on mount: extraction runs on PDFium's single thread and would
  compete with rendering during fast scrolling. Layers of pages inside a selection are never
  evicted, so a long selection copies whole.
- **Copy** goes through `edit.copy` (a `copy` event inside the key press, falling back to the async
  clipboard API) and also answers the Edit menu's copy. Shortcuts that mean something in a text
  field (Cmd/Ctrl+C, Cmd/Ctrl+A, Escape) are left to the field.
- **Highlights** (search hits, selection) are divs in a per-page overlay, positioned in
  percentages of the page like the tiles, blended with `multiply` so glyphs stay legible.
- **Reading themes** (normal, sepia, night) are CSS filters on page and thumbnail images, keyed off
  the root element; the UI theme is independent.
- **Command palette** (Cmd/Ctrl+K): every registry command not marked `paletteHidden`; each matches
  by its title in *both* languages whatever the UI language, loosely (ignoring case, tashkeel,
  alef forms; words as subsequences), with shortcuts shown left to right.
- **Password prompt** is an inline panel in the page area, not a modal (spec §4). Its "wrong
  password" message uses React Aria's `aria` validation: native validation would block trying
  again (found by a test).
- **Persistence**: settings write to the store and to a `localStorage` copy used only for the first
  frame; the session is written on every tab change (not debounced: a delay lost the last change
  when the app quit within it); the reading position is written 0.8 s after scrolling stops and on
  closing the tab.
- **`<Tabs>` wraps only the tab bar and the page area** (`display: contents`). React Aria builds a
  tab collection by rendering everything inside `<Tabs>`; the search results list placed in there
  collided with the tabs' keys (a hit with key 1 against document 1) and crashed the UI.

## Measured (Apple-silicon MacBook)

| | Result |
|---|---|
| Select all, page 1 of the bilingual contract (real app, EN and AR) | copies exactly the page's text layer; Arabic in logical order |
| First search, 1,000-page file (debug app, includes building the index) | 2.9 s |
| Index / search, 1,000 pages (release, `examples/search_bench.rs`) | 1.09 s / 18 ms; index ≈ 34 MB |
| Search on an indexed 1-page document (debug app) | 13–17 ms |
| Session restore | a second launch with the same state store reopens the tab |

## Open

- Thumbnail generation for a 1,000-page document while the page view scrolls is not measured
  against the frame budget (part of the scroll benchmark that still needs a visible window).
- Outline targets with a `/Top` are converted assuming an unrotated page with its origin at the
  corner; rotated pages land at the page top instead.
- Printing, multiple windows, tab reordering by drag, and reading `/ViewerPreferences /Direction`
  are not done.
- Page text is only fetched for selection; screen readers get no text layer yet (M7's
  accessibility pass).
