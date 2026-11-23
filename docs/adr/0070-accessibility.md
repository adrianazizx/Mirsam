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
  75 s to the Linux job. Screens added later go on both lists. The search panel and the OCR tool are
  left off both lists while other sessions rebuild them; they join with that work.
- VoiceOver itself was not driven: controlling it needs a VoiceOver Utility setting, a system
  change this pass did not make. The accessibility tree was read in Chromium instead.
- Deferred, in `PROGRESS.md`: the PDF's own language and structure tree for the page text; a
  shortcut for the interface size (Option-key combinations do not reach `KeyboardEvent.key`
  reliably on macOS); WCAG 2.2's focus-appearance contrast for hover-only states; Windows
  Narrator and Orca runs.
