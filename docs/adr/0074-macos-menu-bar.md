# 0074 — The macOS menu bar runs registry commands, titled and enabled by the window in front

Status: Accepted. Date: 2026-11-24.

## Context

Mirsam built no menu of its own, so on macOS Tauri installed its default one: File held only
"Close Window", and Save, Save as…, Print… and Open… were nowhere in the menu bar, where Mac users
look for them first (reported by the owner). Windows and Linux show no menu bar at all; there the
toolbar, the command palette and shortcuts are the way in (ADR 0071 noted the missing Help menu).

Constraints: every action is a registry command (ADR 0009); every string is in `en.json` and
`ar.json`; the UI's state (active document, unsaved changes, language) lives in each window's
UI, and a window can be a print preview with no UI at all (ADR 0012).

## Decision

- **The shell builds the menu bar on macOS** (`src-tauri/src/menu.rs`) once a window's UI first
  sends its menu; the system default stands in for the moment before. App menu: About, Settings…,
  Services, Hide, Hide others, Show all, Quit. File: New window, Open PDF…, Close, Save, Save as…,
  Print…. Edit (Undo, Redo, Cut, Copy, Paste, Select all), View (Full screen) and Window
  (Minimize, Zoom; macOS lists the windows) keep the system's native items. Help: Welcome guide,
  Keyboard shortcuts…, and the system's search. No menu bar on Windows and Linux.
- **Each window's UI says what the menu shows** (`commands/menu.ts`, `set_menu`): the submenu and
  system item titles (`menu.*`), each item's command title (`commands.*`), its Mac shortcut as an
  accelerator from the registry, and whether the command can run now (`isCommandEnabled`). It is
  sent again only when the language or one of those states changes. The shell keeps each
  window's menu and shows the one in front; a window without a UI leaves it as it was.
- **Picking an item runs the command in the window in front** (`menu-command` event →
  `runMenuCommand` → `runCommand`), else in the window whose menu is shown. Shortcuts are
  unchanged: the page sees a key first and runs its command; the menu acts only on a key the page
  left alone, so nothing runs twice.
- **Close is always available.** It closes the document in front ("Close document"); once the
  window shows none it reads "Close window" and runs the new `window.close` command (the unsaved
  changes prompt still applies, ADR 0015). In a print preview it closes the preview. So ⌘W still
  closes an empty window, as the system default did.

## Consequences

- The menu bar follows the interface language, Arabic included; macOS lays it out by the
  system's language, not the app's.
- Edit's Undo and Redo stay the system's (text fields); the document's undo remains ⌘Z and the
  toolbar. Open Recent is not in the menu yet.
- Tests: `menu.rs` unit tests (which window shows, where a command goes); `menu.test.ts` (the
  command list matches the shell's, titles in both languages, enabled states, Close's two forms,
  sending only on change, Save/Save as/Print/Close run on the document in front).
