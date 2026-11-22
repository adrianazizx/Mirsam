import { backend, isBackendError, type OpenOutcome } from "../backend";
import {
  type AppState,
  type LeftPanel,
  type OpenDocument,
  type ReadingTheme,
  selectActiveDocument,
  useAppStore,
} from "../state/appStore";
import { stepZoom } from "../state/zoom";
import type { PageOrder, ViewMode } from "../viewer/layout";
import { getViewer } from "../viewer/registry";
import { isMac } from "./platform";

/**
 * Command registry: every user action is a command (docs/SPEC.md section 9).
 * Buttons, keyboard shortcuts, and the command palette all run the same
 * entries, so an action has one implementation, one title, and one shortcut
 * everywhere.
 */

export interface Shortcut {
  /** `KeyboardEvent.key`, lower-cased for letters. */
  key: string;
  /** Cmd on macOS, Ctrl elsewhere. */
  mod?: boolean;
  /** The Control key itself on every platform (Ctrl+Tab). */
  ctrl?: boolean;
  shift?: boolean;
  /**
   * Leave the key to a focused text field (Cmd+C, Cmd+A and Escape mean
   * something there already).
   */
  notInFields?: boolean;
}

export interface Command {
  /** Translation key suffix under `commands.`. */
  id: CommandId;
  shortcut?: Shortcut;
  isEnabled?(state: AppState): boolean;
  /** `arg` comes from the control that ran the command (e.g. a typed page number); shortcuts pass none. */
  run(arg?: CommandArg): void | Promise<void>;
  /** Hidden from the command palette (needs an argument only a control can give). */
  paletteHidden?: boolean;
}

/** Arguments a control can pass to a command. */
export interface CommandArg {
  /** `view.goToPage`: zero-based page index. */
  page?: number;
  /** `file.openRecent`, `file.forgetRecent`: the recent file's token. */
  token?: number;
  /** `file.unlock`. Used once, never stored. */
  password?: string;
  /** `search.run`. */
  query?: string;
  /** `search.goTo`: index into the hits. `tabs.move`: the new position in tab order. */
  index?: number;
  /** `file.close`, `tabs.move`: a tab other than the active one. */
  documentId?: number;
}

/** DOM id of the status bar's page number field, which `view.goToPage` focuses. */
export const PAGE_NUMBER_INPUT_ID = "page-number-input";
/** DOM id of the search panel's query field, which `search.focus` focuses. */
export const SEARCH_INPUT_ID = "search-input";

export type CommandId =
  | "file.open"
  | "file.close"
  | "file.print"
  | "file.openRecent"
  | "file.forgetRecent"
  | "file.clearRecents"
  | "file.unlock"
  | "file.cancelUnlock"
  | "tabs.next"
  | "tabs.previous"
  | "tabs.moveEarlier"
  | "tabs.moveLater"
  | "tabs.move"
  | "tabs.moveToNewWindow"
  | "window.new"
  | "edit.copy"
  | "edit.selectAll"
  | "edit.deselect"
  | "view.zoomIn"
  | "view.zoomOut"
  | "view.actualSize"
  | "view.fitWidth"
  | "view.nextPage"
  | "view.previousPage"
  | "view.goToPage"
  | "view.modeContinuous"
  | "view.modeSingle"
  | "view.modeSpread"
  | "view.modeBook"
  | "view.pagesLeftToRight"
  | "view.pagesRightToLeft"
  | "view.toggleLeftPanel"
  | "view.showThumbnails"
  | "view.showOutline"
  | "view.readingNormal"
  | "view.readingSepia"
  | "view.readingNight"
  | "search.focus"
  | "search.run"
  | "search.next"
  | "search.previous"
  | "search.goTo"
  | "search.toggleCase"
  | "search.toggleWholeWord"
  | "search.toggleRegex"
  | "search.toggleArabic"
  | "search.toggleTaMarbuta"
  | "app.toggleTheme"
  | "app.toggleLanguage"
  | "app.toggleGallery"
  | "app.commandPalette";

const store = () => useAppStore.getState();
const hasDocument = (state: AppState) => selectActiveDocument(state) !== null;
const hasSelection = (state: AppState) => selectActiveDocument(state)?.hasSelection ?? false;
const hasHits = (state: AppState) => (selectActiveDocument(state)?.search.hits.length ?? 0) > 0;

/** Makes `page` current and scrolls the viewer to it. */
function navigateTo(active: OpenDocument, page: number) {
  const target = Math.min(Math.max(0, Math.trunc(page)), active.pageCount - 1);
  store().updateDocument(active.id, { page: target });
  getViewer(active.id)?.goToPage(target);
}

/**
 * The page one step away: the next or previous row of the layout when a viewer
 * is showing the document (so spreads step by two), otherwise one page.
 */
function stepTarget(active: OpenDocument, direction: 1 | -1): number | null {
  const viewer = getViewer(active.id);
  if (viewer) return viewer.stepTarget(direction);
  const next = active.page + direction;
  return next >= 0 && next < active.pageCount ? next : null;
}

function setViewMode(viewMode: ViewMode) {
  const active = selectActiveDocument(store());
  if (active) store().updateDocument(active.id, { viewMode });
}

function setPageOrder(pageOrder: PageOrder) {
  const active = selectActiveDocument(store());
  if (active) store().updateDocument(active.id, { pageOrder });
}

function showPanel(panel: LeftPanel) {
  store().setLeftPanel(panel);
}

function setReadingTheme(theme: ReadingTheme) {
  store().setReadingTheme(theme);
}

/** Error codes with their own message under `errors.`; anything else is "generic". */
const KNOWN_ERRORS = ["invalid_pdf", "engine_unavailable", "file_missing", "print_failed"];

function reportError(error: unknown) {
  const code = isBackendError(error) && KNOWN_ERRORS.includes(error.code) ? error.code : "generic";
  console.error("command failed", error);
  store().setError(code);
}

/** Puts an opened file in a tab, or queues its password prompt. */
export function acceptOutcome(outcome: OpenOutcome, options?: { activate?: boolean }) {
  if (outcome.kind === "opened") {
    store().addDocument(outcome.document, options);
  } else {
    store().pushUnlock({ token: outcome.token, name: outcome.name, retry: outcome.retry });
  }
}

/** Reloads the start screen's recent files (after opening, forgetting, clearing). */
export async function refreshRecents() {
  try {
    store().setRecents(await backend.recentFiles(12));
  } catch (error) {
    console.warn("recent files unavailable", error);
    store().setRecents([]);
  }
}

function moveTab(direction: 1 | -1) {
  const { documents, activeDocumentId } = store();
  if (documents.length < 2) return;
  const index = documents.findIndex((open) => open.id === activeDocumentId);
  const next = documents[(index + direction + documents.length) % documents.length];
  if (next) store().activateDocument(next.id);
}

function shiftTab(direction: 1 | -1) {
  const { documents, activeDocumentId } = store();
  const index = documents.findIndex((open) => open.id === activeDocumentId);
  if (index >= 0 && activeDocumentId !== null)
    store().moveDocument(activeDocumentId, index + direction);
}

/** Copies text: through a `copy` event first (works inside the key press), then the async API. */
async function writeClipboard(text: string) {
  let copied = false;
  const onCopy = (event: ClipboardEvent) => {
    event.clipboardData?.setData("text/plain", text);
    event.preventDefault();
    copied = true;
  };
  document.addEventListener("copy", onCopy, { capture: true, once: true });
  try {
    document.execCommand("copy");
  } finally {
    document.removeEventListener("copy", onCopy, { capture: true });
  }
  if (!copied) await navigator.clipboard.writeText(text);
}

async function runSearch(active: OpenDocument, query: string) {
  const serial = active.search.serial + 1;
  const viewer = getViewer(active.id);
  if (!query.trim()) {
    store().updateSearch(active.id, {
      query,
      status: "idle",
      hits: [],
      truncated: false,
      active: -1,
      error: null,
      serial,
    });
    return;
  }
  store().updateSearch(active.id, { query, status: "searching", error: null, serial });
  try {
    const results = await backend.searchDocument(active.id, query, store().searchOptions);
    const current = store().documents.find((open) => open.id === active.id);
    if (!current || current.search.serial !== serial) return;
    // Start from the reader's position rather than jumping back to page 1.
    const from = results.hits.findIndex((hit) => hit.page >= current.page);
    const first = results.hits.length === 0 ? -1 : Math.max(0, from);
    store().updateSearch(active.id, {
      status: "done",
      hits: results.hits,
      truncated: results.truncated,
      active: first,
    });
    const hit = results.hits[first];
    if (hit) viewer?.revealHit(hit);
  } catch (error) {
    const current = store().documents.find((open) => open.id === active.id);
    if (!current || current.search.serial !== serial) return;
    const code =
      isBackendError(error) && error.code === "invalid_query" ? "searchInvalid" : "searchFailed";
    if (!isBackendError(error) || error.code !== "cancelled") {
      store().updateSearch(active.id, { status: "error", error: code, hits: [], active: -1 });
    }
  }
}

function stepHit(direction: 1 | -1) {
  const active = selectActiveDocument(store());
  if (!active || active.search.hits.length === 0) return;
  const count = active.search.hits.length;
  const current = active.search.active;
  const index =
    current < 0 ? (direction === 1 ? 0 : count - 1) : (current + direction + count) % count;
  goToHit(active, index);
}

function goToHit(active: OpenDocument, index: number) {
  const hit = active.search.hits[index];
  if (!hit) return;
  store().updateSearch(active.id, { active: index });
  getViewer(active.id)?.revealHit(hit);
}

function toggleSearchOption(key: keyof AppState["searchOptions"]) {
  const options = store().searchOptions;
  store().setSearchOptions({ ...options, [key]: !options[key] });
  const active = selectActiveDocument(store());
  if (active?.search.query) void runSearch(active, active.search.query);
}

/** The theme currently on screen, with "system" resolved. */
export function resolvedTheme(theme: AppState["theme"]): "light" | "dark" {
  if (theme !== "system") return theme;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

const COMMANDS: Command[] = [
  {
    id: "file.open",
    shortcut: { key: "o", mod: true },
    async run() {
      const outcome = await backend.openDocumentDialog();
      if (outcome) acceptOutcome(outcome);
      void refreshRecents();
    },
  },
  {
    id: "file.close",
    shortcut: { key: "w", mod: true },
    isEnabled: hasDocument,
    async run(arg) {
      const id = arg?.documentId ?? selectActiveDocument(store())?.id;
      if (id === undefined) return;
      store().removeDocument(id);
      await backend.closeDocument(id);
    },
  },
  {
    id: "file.print",
    shortcut: { key: "p", mod: true },
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (active) await backend.printDocument(active.id);
    },
  },
  {
    id: "file.openRecent",
    paletteHidden: true,
    async run(arg) {
      if (arg?.token === undefined) return;
      acceptOutcome(await backend.openRecent(arg.token));
      void refreshRecents();
    },
  },
  {
    id: "file.forgetRecent",
    paletteHidden: true,
    async run(arg) {
      if (arg?.token === undefined) return;
      await backend.forgetRecent(arg.token);
      await refreshRecents();
    },
  },
  {
    id: "file.clearRecents",
    isEnabled: (state) => (state.recents?.length ?? 0) > 0,
    async run() {
      await backend.clearRecents();
      await refreshRecents();
    },
  },
  {
    id: "file.unlock",
    paletteHidden: true,
    isEnabled: (state) => state.unlocks.length > 0,
    async run(arg) {
      const pending = store().unlocks[0];
      if (!pending || arg?.password === undefined) return;
      const outcome = await backend.unlockDocument(pending.token, arg.password);
      if (outcome.kind === "opened") {
        store().resolveUnlock(null);
        store().addDocument(outcome.document);
        void refreshRecents();
      } else {
        store().resolveUnlock({ token: outcome.token, name: outcome.name, retry: outcome.retry });
      }
    },
  },
  {
    id: "file.cancelUnlock",
    paletteHidden: true,
    isEnabled: (state) => state.unlocks.length > 0,
    async run() {
      const pending = store().unlocks[0];
      if (!pending) return;
      store().resolveUnlock(null);
      await backend.cancelUnlock(pending.token);
    },
  },
  {
    id: "tabs.next",
    shortcut: { key: "Tab", ctrl: true },
    isEnabled: (state) => state.documents.length > 1,
    run: () => moveTab(1),
  },
  {
    id: "tabs.previous",
    shortcut: { key: "Tab", ctrl: true, shift: true },
    isEnabled: (state) => state.documents.length > 1,
    run: () => moveTab(-1),
  },
  {
    // Tab order is reading order: "earlier" is to the left in English and to
    // the right in Arabic, so the commands are named by order, not side.
    id: "tabs.moveEarlier",
    shortcut: { key: "PageUp", ctrl: true, shift: true },
    isEnabled: (state) => state.documents.findIndex((d) => d.id === state.activeDocumentId) > 0,
    run: () => shiftTab(-1),
  },
  {
    id: "tabs.moveLater",
    shortcut: { key: "PageDown", ctrl: true, shift: true },
    isEnabled: (state) => {
      const index = state.documents.findIndex((d) => d.id === state.activeDocumentId);
      return index >= 0 && index < state.documents.length - 1;
    },
    run: () => shiftTab(1),
  },
  {
    id: "tabs.move",
    paletteHidden: true,
    isEnabled: (state) => state.documents.length > 1,
    run(arg) {
      if (arg?.documentId !== undefined && arg.index !== undefined) {
        store().moveDocument(arg.documentId, arg.index);
      }
    },
  },
  {
    id: "tabs.moveToNewWindow",
    isEnabled: (state) => state.documents.length > 1,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      await backend.moveToNewWindow(active.id);
      // The document stays open; it now belongs to the new window.
      store().removeDocument(active.id);
    },
  },
  {
    id: "window.new",
    shortcut: { key: "n", mod: true },
    run: () => backend.newWindow(),
  },
  {
    id: "edit.copy",
    shortcut: { key: "c", mod: true, notInFields: true },
    isEnabled: hasSelection,
    async run() {
      const active = selectActiveDocument(store());
      const text = active ? (getViewer(active.id)?.selectedText() ?? "") : "";
      if (text) await writeClipboard(text);
    },
  },
  {
    id: "edit.selectAll",
    shortcut: { key: "a", mod: true, notInFields: true },
    isEnabled: (state) => hasDocument(state) && state.activeTool === "select",
    run() {
      const active = selectActiveDocument(store());
      if (active) getViewer(active.id)?.selectAllOnPage();
    },
  },
  {
    id: "edit.deselect",
    shortcut: { key: "Escape", notInFields: true },
    isEnabled: hasSelection,
    run() {
      const active = selectActiveDocument(store());
      if (active) getViewer(active.id)?.clearSelection();
    },
  },
  {
    id: "view.zoomIn",
    shortcut: { key: "=", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateDocument(active.id, { zoomMode: "custom", zoom: stepZoom(active.zoom, 1) });
      }
    },
  },
  {
    id: "view.zoomOut",
    shortcut: { key: "-", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateDocument(active.id, { zoomMode: "custom", zoom: stepZoom(active.zoom, -1) });
      }
    },
  },
  {
    id: "view.actualSize",
    shortcut: { key: "1", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { zoomMode: "custom", zoom: 1 });
    },
  },
  {
    id: "view.fitWidth",
    shortcut: { key: "2", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { zoomMode: "fit-width" });
    },
  },
  {
    id: "view.nextPage",
    shortcut: { key: "PageDown" },
    isEnabled: (state) => {
      const active = selectActiveDocument(state);
      return active !== null && stepTarget(active, 1) !== null;
    },
    run() {
      const active = selectActiveDocument(store());
      const target = active && stepTarget(active, 1);
      if (active && target !== null && target !== undefined) navigateTo(active, target);
    },
  },
  {
    id: "view.previousPage",
    shortcut: { key: "PageUp" },
    isEnabled: (state) => {
      const active = selectActiveDocument(state);
      return active !== null && stepTarget(active, -1) !== null;
    },
    run() {
      const active = selectActiveDocument(store());
      const target = active && stepTarget(active, -1);
      if (active && target !== null && target !== undefined) navigateTo(active, target);
    },
  },
  {
    // With a page argument (the status bar field, the outline, thumbnails),
    // goes there; from the shortcut, puts the cursor in that field.
    id: "view.goToPage",
    shortcut: { key: "n", mod: true, shift: true },
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active) return;
      if (arg?.page !== undefined && Number.isFinite(arg.page)) {
        navigateTo(active, arg.page);
        return;
      }
      const field = document.getElementById(PAGE_NUMBER_INPUT_ID);
      if (field instanceof HTMLInputElement) {
        field.focus();
        field.select();
      }
    },
  },
  { id: "view.modeContinuous", isEnabled: hasDocument, run: () => setViewMode("continuous") },
  { id: "view.modeSingle", isEnabled: hasDocument, run: () => setViewMode("single") },
  { id: "view.modeSpread", isEnabled: hasDocument, run: () => setViewMode("spread") },
  { id: "view.modeBook", isEnabled: hasDocument, run: () => setViewMode("book") },
  { id: "view.pagesLeftToRight", isEnabled: hasDocument, run: () => setPageOrder("ltr") },
  { id: "view.pagesRightToLeft", isEnabled: hasDocument, run: () => setPageOrder("rtl") },
  {
    id: "view.toggleLeftPanel",
    // Acrobat's key for the navigation pane.
    shortcut: { key: "F4" },
    run() {
      store().setLeftPanel(store().leftPanel ? null : "thumbnails");
    },
  },
  { id: "view.showThumbnails", run: () => showPanel("thumbnails") },
  { id: "view.showOutline", run: () => showPanel("outline") },
  { id: "view.readingNormal", run: () => setReadingTheme("normal") },
  { id: "view.readingSepia", run: () => setReadingTheme("sepia") },
  { id: "view.readingNight", run: () => setReadingTheme("night") },
  {
    id: "search.focus",
    shortcut: { key: "f", mod: true },
    isEnabled: hasDocument,
    run() {
      showPanel("search");
      // The field mounts with the panel; focus it once it exists.
      requestAnimationFrame(() => {
        const field = document.getElementById(SEARCH_INPUT_ID);
        if (field instanceof HTMLInputElement) {
          field.focus();
          field.select();
        }
      });
    },
  },
  {
    id: "search.run",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (active && arg?.query !== undefined) await runSearch(active, arg.query);
    },
  },
  {
    id: "search.next",
    shortcut: { key: "g", mod: true },
    isEnabled: hasHits,
    run: () => stepHit(1),
  },
  {
    id: "search.previous",
    shortcut: { key: "g", mod: true, shift: true },
    isEnabled: hasHits,
    run: () => stepHit(-1),
  },
  {
    id: "search.goTo",
    paletteHidden: true,
    isEnabled: hasHits,
    run(arg) {
      const active = selectActiveDocument(store());
      if (active && arg?.index !== undefined) goToHit(active, arg.index);
    },
  },
  { id: "search.toggleCase", run: () => toggleSearchOption("caseSensitive") },
  { id: "search.toggleWholeWord", run: () => toggleSearchOption("wholeWord") },
  { id: "search.toggleRegex", run: () => toggleSearchOption("regex") },
  { id: "search.toggleArabic", run: () => toggleSearchOption("arabicNormalize") },
  { id: "search.toggleTaMarbuta", run: () => toggleSearchOption("taMarbutaAsHa") },
  {
    id: "app.toggleTheme",
    run() {
      store().setTheme(resolvedTheme(store().theme) === "dark" ? "light" : "dark");
    },
  },
  {
    id: "app.toggleLanguage",
    run() {
      store().setLocale(store().locale === "ar" ? "en" : "ar");
    },
  },
  {
    id: "app.toggleGallery",
    // Not Shift+Cmd+G: that is "previous search result" everywhere.
    shortcut: { key: "d", mod: true, shift: true },
    run() {
      store().setGalleryOpen(!store().galleryOpen);
    },
  },
  {
    id: "app.commandPalette",
    shortcut: { key: "k", mod: true },
    run() {
      store().setPaletteOpen(!store().paletteOpen);
    },
  },
];

const BY_ID = new Map(COMMANDS.map((command) => [command.id, command]));

export function getCommand(id: CommandId): Command {
  const command = BY_ID.get(id);
  if (!command) throw new Error(`unknown command: ${id}`);
  return command;
}

/** Every command, in registry order (the palette lists them). */
export function allCommands(): readonly Command[] {
  return COMMANDS;
}

export function isCommandEnabled(id: CommandId, state: AppState = store()): boolean {
  return getCommand(id).isEnabled?.(state) ?? true;
}

/** Runs a command if it is enabled. Failures surface as a localized error banner. */
export async function runCommand(id: CommandId, arg?: CommandArg): Promise<void> {
  if (!isCommandEnabled(id)) return;
  try {
    await getCommand(id).run(arg);
  } catch (error) {
    reportError(error);
  }
}

export function matchesShortcut(shortcut: Shortcut, event: KeyboardEvent, mac = isMac): boolean {
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  // "+" is Shift+"=" on most layouts; accept either for zoom in.
  const keyMatches = key === shortcut.key || (shortcut.key === "=" && key === "+");
  const shiftMatches = shortcut.key === "=" || event.shiftKey === Boolean(shortcut.shift);
  if (shortcut.ctrl) {
    return keyMatches && event.ctrlKey && !event.metaKey && !event.altKey && shiftMatches;
  }
  const mod = mac ? event.metaKey : event.ctrlKey;
  const otherMod = mac ? event.ctrlKey : event.metaKey;
  return keyMatches && mod === Boolean(shortcut.mod) && !otherMod && !event.altKey && shiftMatches;
}

/** True when keyboard focus is somewhere that edits text. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["button", "checkbox", "radio", "range", "submit", "reset"].includes(target.type);
  }
  return false;
}

/** The command bound to this key event, if any. */
export function commandForEvent(event: KeyboardEvent, mac = isMac): Command | undefined {
  const inField = isEditableTarget(event.target);
  return COMMANDS.find(
    (command) =>
      command.shortcut &&
      !(inField && command.shortcut.notInFields) &&
      matchesShortcut(command.shortcut, event, mac),
  );
}

/** Human-readable shortcut, e.g. "⌘O" on macOS and "Ctrl+O" elsewhere. */
export function shortcutLabel(id: CommandId, mac = isMac): string | null {
  const shortcut = getCommand(id).shortcut;
  if (!shortcut) return null;
  const names: Record<string, string> = {
    "=": "+",
    PageDown: "PgDn",
    PageUp: "PgUp",
    Escape: "Esc",
    Tab: mac ? "⇥" : "Tab",
  };
  const key = names[shortcut.key] ?? shortcut.key.toUpperCase();
  if (mac) {
    return `${shortcut.ctrl ? "⌃" : ""}${shortcut.shift ? "⇧" : ""}${shortcut.mod ? "⌘" : ""}${key}`;
  }
  return [shortcut.mod || shortcut.ctrl ? "Ctrl" : null, shortcut.shift ? "Shift" : null, key]
    .filter(Boolean)
    .join("+");
}
