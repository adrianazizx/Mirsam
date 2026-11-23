# 0070 — Accessibility: checks, interface size, page text for screen readers

Status: Accepted. Date: 2026-11-23. Milestone 7's accessibility pass (spec §4: WCAG 2.1 AA).

## Context

The shell was built on React Aria, so most controls already had names, roles, states and
keyboard handling. Gaps the pass found (axe-core in Chrome over the main screens, in English
and Arabic, light and dark, plus reading the code):

- **The page canvas said nothing.** Each page was `role="img"` with a label ("Page 3 of …"):
  its text, which the engine extracts in reading order, never reached a screen reader.
- **The document tab nested a button** (its close button) in a `role="tab"`, whose content is
  presentational: unreachable for screen readers and a nested-interactive violation. "Unsaved
  changes" was an image inside it, so it was lost too.
- **A dangling `aria-controls`** on the selected tab while the design gallery or the password
  prompt filled the page area (no tab panel was rendered).
- **No live regions** a screen reader could rely on: errors and notices were inserted together
  with their `role`, which VoiceOver often does not speak, and saves said nothing at all.
- **No way to enlarge the interface.** Font sizes were fixed pixels; Cmd/Ctrl+= zooms the
  document; the webview's own zoom is off. WCAG 1.4.4 (resize text to 200%) failed.
- Smaller: the page organizer's toolbar was a `div role="toolbar"` with no arrow keys; every
  mounted thumbnail was a tab stop; the language switch, named "العربية" in the English UI,
  had no `lang`; scrolling areas with only text could not be scrolled from the keyboard; no h1
  when a document was open; Escape did not leave the password prompt.

## Decision

### Automated checks

- **axe-core 4.14** (Deque, MPL-2.0, no dependencies; a dev dependency only, not shipped and
  not in the licence report).
- **In the unit tests** (`src/a11y/`, jsdom): every main screen (empty window, page view, the
  side panels, each tool's panel, the dialogs) in English light and Arabic dark, the main ones
  in the other two combinations; WCAG 2.0/2.1 A and AA rules and axe's best practices. jsdom
  computes no colours, so contrast is off there. A guard test proves the harness catches a
  nameless button and a nested control.
- **In Chrome** (`scripts/a11y-check.mjs`, CI's Linux test job): the same screens rendered for
  real, in all four language and theme combinations and the reading themes, contrast included,
  with `prefers-reduced-motion` so no screen is measured mid-transition, a contrast failure
  counted only if it is still there a second later, and a 200% interface pass that reports
  clipped text. A short, commented exclusion list: the command palette's list (a combobox
  listbox with virtual focus). **axe skipped most Arabic text's contrast:** its icon-ligature
  test takes joined Arabic letter forms for an icon font. The script records every font as a
  text font in axe's cache (the app's icons are SVG), and fails loudly if axe's internals change.
- **Keyboard tests** (`src/a11y/keyboard.test.tsx`): Tab and Shift+Tab make a full round
  through every area without being kept anywhere; F6 order; arrow keys on the tool rail, the
  tab bar, the organizer's toolbar and the thumbnails, mirrored in Arabic; Escape on the
  palette, a dropdown, Settings and the password prompt, with focus given back.
- The token contrast test (`styles/tokens.test.ts`) stays the gate for colours that axe
  cannot see (hover, pressed, selected states).

### Keyboard

- **Tab order is the document's:** the tab bar, the page area, then the tools, the side panel,
  the inspector and the status bar. The page area right after its tab is deliberate (the
  document is what the window is for). Moving the rail and the side panel before it would put
  them inside React Aria's `Tabs`, whose collection building collides with the lists inside
  them (comment in `AppShell`).
- **F6 / Shift+F6** (`app.nextRegion`, `app.previousRegion`) move between the areas in the order
  they are seen — tabs, tools, side panel, page area, inspector, status bar — landing on each
  area's own place: the selected tab, the active tool, the page view. This is the usual desktop
  pattern for reaching panels without tabbing through the document.
- One tab stop per composite: React Aria's toolbars and radio groups already behave so; the
  thumbnail strip now has one (roving `tabIndex`), with Up/Down, Page Up/Down, Home and End.
- **Dragging pages without a pointer:** each page in the organizer has React Aria's drag handle
  (Tab to it, Enter, arrows, Enter; named by React Aria in the UI's language), beside the
  existing move earlier/later commands.

### Screen readers

- **Pages are named groups holding their text.** Once the view rests (400 ms), the pages on
  screen (at most six) get their text layer — the same cached `page_text` the selection uses —
  as hidden paragraphs: one per block, with the block's `dir`, its language guessed from its
  script, and each run of another script marked with its own `lang` (an English contract number
  in an Arabic sentence). A page with no text, or text that may read wrongly, says so and that
  OCR can help. The tiles and overlays are `aria-hidden`. Scrolling never waits on any of it.
  Latin script is taken as English; the PDF's `/Lang` and tagged structure are not read yet.
- **Two live regions, always present** (`LiveRegions`), marked `data-live-announcer` so React
  Aria leaves them audible while a dialog hides the rest of the window. Errors are assertive;
  notices, saves, the interface size, and the page and zoom a command moved to (once they settle,
  not while scrolling, and not when a fitted zoom follows the window) are polite. The visible
  banner lost its roles, so nothing is said twice.

### Interface size (WCAG 1.4.4)

- **A setting, 100% to 200%** (Settings → Display; `app.uiLarger`, `app.uiSmaller`,
  `app.uiDefaultSize`), stored as `uiScale`. It sets `--ui-scale`, which every size, spacing and
  font token of the chrome multiplies; the page canvas's geometry tokens stay plain lengths, as
  the pages have their own zoom. Side panels grow but are capped by the window's width; the
  status bar, the side panel's header and the tool rail wrap or scroll instead of clipping.
- **Not the webview's zoom.** WKWebView's page zoom does not raise `devicePixelRatio`, so the
  page tiles would be drawn at half the resolution they are shown at; it would also take
  Cmd/Ctrl+=, which zooms the document.
- `tokenPx` now resolves a token through a probe element, since a scaled token is a `calc()`
  that a custom property's computed value leaves unworked.

## Consequences

- The jsdom suite (about 5 s) runs on every platform; the Chrome check (100 screens) adds about
  75 s to the Linux job. Screens added later go on both lists. The OCR tool is left off both lists
  while another session rebuilds it; it joins with that work. The search panel joined with find and
  replace (ADR 0052): results, the replace row and the replace preview.
- VoiceOver itself was not driven: controlling it needs a VoiceOver Utility setting, a system
  change this pass did not make. The accessibility tree was read in Chromium instead.
- Deferred, in `PROGRESS.md`: the PDF's own language and structure tree for the page text; a
  shortcut for the interface size (Option-key combinations do not reach `KeyboardEvent.key`
  reliably on macOS); WCAG 2.2's focus-appearance contrast for hover-only states; Windows
  Narrator and Orca runs.

## Amendment (2026-11-23): the second pass

The OCR tool, the Settings sections added since (language and digits, OCR models with the
Tesseract note, crash reports), the certificate dialog's time-stamp field, and the items
deferred above.

- **Live regions, not `role="status"`.** The OCR panel had seven status roles, each inserted with
  its text (often skipped by VoiceOver) or updated at every progress step (spoken at every
  step, where it was spoken). They are plain text now, and `a11y/ocrAnnouncements.ts` says the
  changes nobody is looking at through the two shared live regions: a job starting, pausing,
  resuming, finishing or failing (failures assertively), a layer check's verdict, a model
  download ending (installed, paused, or why it stopped, with the disk space a full disk needs),
  an area set, a page converted, review mode done. Progress itself is not spoken; the progress
  bars give it when asked. The same goes for the OCR models' failure line and the certificate
  dialog's error and "Signing…" lines (the signing command announces them).
- **Review mode keeps focus.** The field describes itself with the card's "N of M left" line,
  why the word is listed, and the engine's confidence (`aria-describedby`), so stepping to the
  next word reads all of it. Escape leaves review mode and focus goes to "Review"; deciding the
  last item moves focus to "Done reviewing" and announces it; a page to check as a whole
  focuses "Check page". The word image's text alternative says what it is (the scan of page N
  around the word read as X, the word outlined), not what it shows: the pixels are the one
  thing a reviewer using a screen reader cannot get, and the reasons, the other readings and the
  confidence are given as text beside it.
- **The flagged words are one tab stop** (Up/Down, Home, End move between them), like the
  thumbnails, so Tab gets past a long list.
- **An area without a pointer** (WCAG 2.1.1): "Use the whole page" sets the area to the page
  shown (`ocr.regionWholePage`), and four number fields (from the left edge, from the top,
  width, height, in percent of the page) narrow it. The page's geometry is physical, so "left"
  is left in Arabic too.
- **`aria-keyshortcuts`** was dropped by React Aria's buttons (they filter DOM props), so the
  review card's attribute never reached the DOM, and held display symbols ("⌘↩") rather than
  ARIA's syntax. `Button` and `IconButton` now set it through a ref (`keyShortcutsRef`), every
  `CommandButton` carries its command's shortcut, and `ariaShortcut(id)` gives "Meta+Enter".
- **Repeated buttons say what they act on:** each OCR model row's and each crash report's
  buttons are described by the model's name or the report's kind and time.
- **The time-stamp address** is `required`, `aria-invalid` while empty, and described by its hint
  and by why signing is off (WCAG 3.3.1, 3.3.2).
- **Deferred items done:**
  - the PDF's `/Lang` (`catalog::CatalogHints::language`, a well-formed BCP 47 tag, not read from
    encrypted files) decides the language of the script it is written in (`fr` for Latin text,
    `fa` or `ur` for Arabic script); a language in another script leaves the guess alone;
  - interface size shortcuts: Cmd/Ctrl+Option/Alt+`=`, `-`, `0`, matched by the physical key
    (`KeyboardEvent.code`), since Option changes `key` on macOS;
  - the search panel's row height follows the interface size;
  - hover and pressed states: `--color-border-strong` was 2.55:1 on a hovered control in the
    light theme and 2.88:1 in the dark one; both now pass 3:1 on the app, hover and pressed
    backgrounds, and the focus ring is checked against hover, pressed and selected (tokens test);
  - **Keyboard shortcuts** (`app.shortcuts`, Cmd/Ctrl+/): every command's shortcut from the
    registry, F6 and F8 included (the palette leaves those out), with a line on Tab, F6, the
    arrows and Escape. A skip link stays unneeded: the page area follows the tab bar.
- **Still deferred:** the tagged structure tree's own `/Lang` and reading order; WCAG 2.2's focus
  appearance (AAA) beyond the contrast pairs; Windows Narrator and Orca; VoiceOver itself
  (owner item 24).
