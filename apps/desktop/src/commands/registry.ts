import {
  type BlockMark,
  backend,
  type EditOutcome,
  isBackendError,
  type MarkupKind,
  type OpenOutcome,
  type PageEdit,
  type ParagraphBlock,
  type RedactPattern,
  type RedactPreset,
  type RestoreOutcome,
  type SanitizeOptions,
  type SplitBy,
  type StrokePoint,
  type TextStyle,
  type ViewRect,
} from "../backend";
import { marksOfRuns, sameMarks } from "../components/comments/richText";
import {
  type AppState,
  type CommentTool,
  type LeftPanel,
  type OpenDocument,
  type ReadingTheme,
  selectActiveDocument,
  useAppStore,
} from "../state/appStore";
import type { CommentFilter, CommentSort } from "../state/comments";
import {
  addMark,
  MIN_MARK_SIDE,
  marksForApply,
  redoMark,
  removeMark,
  undoMark,
} from "../state/redaction";
import { stepZoom } from "../state/zoom";
import { NOTE_SIZE, type SelectedAnnotation, TEXT_BOX_WIDTH } from "../viewer/controller";
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
  /** Another key that does the same (Backspace for Delete: a Mac's delete key). */
  alternate?: string;
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
  /** `file.close`: close even with unsaved changes (the user chose not to save). */
  force?: boolean;
  /** `pages.move`: the pages to move (zero-based, ascending). */
  pages?: number[];
  /** `pages.move`: the index the first moved page takes in the result. */
  to?: number;
  /** `pages.splitInto`: how to split. */
  split?: SplitBy;
  /** `note.save`: the note's text. */
  text?: string;
  /** `note.place`, `textBox.place`: the top-left corner, in displayed-page points. */
  x?: number;
  y?: number;
  /** `textBox.place`: the box's width, in points. */
  width?: number;
  /** `ink.add`: the pen's strokes, in displayed-page points. */
  strokes?: StrokePoint[][];
  /** `ink.erase`: the ink annotations touched, by index. */
  indices?: number[];
  /** `recovery.restore`, `recovery.discard`: the recovery file's token (`token` above). */
  /** `comment.show`: the annotation to scroll to and select (`page`, `index` as for replies). */
  annotation?: SelectedAnnotation;
  /** `comment.resolve`: resolved (true) or open again (false). */
  resolved?: boolean;
  /** `comments.filter`: the filters to change. */
  filter?: Partial<CommentFilter>;
  /** `comments.sort`. */
  sort?: CommentSort;
  /** `redact.addArea`: the area, on the displayed page (`page` above). */
  rect?: ViewRect;
  /** `redact.findPreset`. */
  preset?: RedactPreset;
  /** `redact.removeMark`. */
  markId?: number;
  /** `sanitize.setOption`. */
  option?: keyof SanitizeOptions;
  value?: boolean;
  /** `text.setStyle`: what to change in how new page text is set. */
  textStyle?: Partial<TextStyle>;
  /** `text.editBlock`: the paragraph to edit in place (ADR 0023). */
  block?: ParagraphBlock;
  /** `note.save` of a paragraph: the bundled family to set it in; absent, its own font where it can. */
  blockFamily?: "sans" | "naskh";
  /** `note.save` of a paragraph: bold and italic as its field set them (ADR 0025). */
  blockMarks?: BlockMark[];
}

/** DOM id of the status bar's page number field, which `view.goToPage` focuses. */
export const PAGE_NUMBER_INPUT_ID = "page-number-input";
/** DOM id of the search panel's query field, which `search.focus` focuses. */
export const SEARCH_INPUT_ID = "search-input";

export type CommandId =
  | "file.open"
  | "file.close"
  | "file.print"
  | "file.saveAndClose"
  | "window.closeDiscarding"
  | "app.quitDiscarding"
  | "app.cancelClose"
  | "file.save"
  | "file.saveAs"
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
  | "markup.highlight"
  | "markup.underline"
  | "markup.strikeOut"
  | "markup.squiggly"
  | "annotation.delete"
  | "redact.openTool"
  | "redact.addArea"
  | "redact.markSelection"
  | "redact.find"
  | "redact.findPreset"
  | "redact.toggleRegex"
  | "redact.markHit"
  | "redact.markAllHits"
  | "redact.removeMark"
  | "redact.clearMarks"
  | "redact.undoMark"
  | "redact.redoMark"
  | "redact.apply"
  | "redact.confirm"
  | "redact.cancel"
  | "sanitize.setOption"
  | "sanitize.run"
  | "recovery.restore"
  | "recovery.discard"
  | "recovery.later"
  | "comment.note"
  | "comment.textBox"
  | "textBox.place"
  | "comment.pen"
  | "comment.eraser"
  | "note.place"
  | "note.save"
  | "note.cancel"
  | "ink.add"
  | "ink.erase"
  | "edit.undo"
  | "edit.redo"
  | "pages.rotateClockwise"
  | "pages.rotateCounterclockwise"
  | "pages.delete"
  | "pages.duplicate"
  | "pages.insertBlank"
  | "pages.moveEarlier"
  | "pages.moveLater"
  | "pages.move"
  | "pages.open"
  | "pages.insertFromFile"
  | "pages.extract"
  | "pages.split"
  | "pages.splitInto"
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
  | "view.showComments"
  | "comment.show"
  | "comment.reply"
  | "comment.resolve"
  | "comments.filter"
  | "comments.sort"
  | "comments.setAuthor"
  | "text.place"
  | "text.editBlock"
  | "text.bold"
  | "text.italic"
  | "text.setStyle"
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
const hasAnnotationSelected = (state: AppState) =>
  Boolean(selectActiveDocument(state)?.selectedAnnotation);
/** A note is being written: a new one, or the selected one. */
/** A paragraph of page text is open in the editor (ADR 0023). */
const isEditingBlock = (state: AppState) =>
  state.pendingNote?.kind === "block" && state.pendingNote.documentId === state.activeDocumentId;

/**
 * Bold or italic on the paragraph field's selection, by the browser's own
 * editing command, so the field's undo covers it (ADR 0025). The field keeps
 * focus: the editor's buttons do not take it.
 */
function formatBlock(kind: "bold" | "italic") {
  const field = document.querySelector<HTMLElement>('[data-testid="block-field"]');
  if (!field) return;
  if (!field.contains(document.activeElement)) field.focus();
  document.execCommand(kind);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}
const isWritingNote = (state: AppState) =>
  (state.pendingNote !== null && state.pendingNote.documentId === state.activeDocumentId) ||
  ["note", "textBox"].includes(selectActiveDocument(state)?.selectedAnnotation?.kind ?? "");
/** While marking areas to redact, undo and redo step through the marks first (ADR 0042). */
const stepsMarks = (state: AppState, direction: "undo" | "redo") => {
  const redaction = selectActiveDocument(state)?.redaction;
  if (state.activeTool !== "protect" || !redaction) return false;
  return direction === "undo" ? redaction.marks.length > 0 : redaction.undone.length > 0;
};
const canUndo = (state: AppState) =>
  stepsMarks(state, "undo") || (selectActiveDocument(state)?.canUndo ?? false);
const canRedo = (state: AppState) =>
  stepsMarks(state, "redo") || (selectActiveDocument(state)?.canRedo ?? false);
const hasMarks = (state: AppState) =>
  (selectActiveDocument(state)?.redaction.marks.length ?? 0) > 0;

/**
 * The pages a page command acts on: the organizer's selection when it has
 * one, else the page being read. Ascending.
 */
export function targetPages(state: AppState = store()): number[] {
  const active = selectActiveDocument(state);
  if (!active) return [];
  if (state.activeTool === "pages" && active.selectedPages.length > 0) return active.selectedPages;
  return [active.page];
}

/**
 * Applies a page edit to the active document (ADR 0014). `select`: the pages
 * to select afterwards, in the edited document.
 */
async function editActive(
  edit: (active: OpenDocument, pages: number[]) => PageEdit,
  select: (pages: number[], edit: PageEdit) => number[] = () => [],
): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active) return;
  const pages = targetPages();
  const change = edit(active, pages);
  store().applyEdit(active.id, await backend.editPages(active.id, change));
  store().setSelectedPages(active.id, select(pages, change));
}

/** Where moved pages land: the first at `to` of the result, the rest after it. */
function landed(edit: PageEdit, count: number): number[] {
  return edit.kind === "move" ? Array.from({ length: count }, (_, i) => edit.to + i) : [];
}

/**
 * `to` for moving `pages` one step earlier or later: the index the first of
 * them takes in the result. Null when they cannot move that way.
 */
export function moveTarget(pages: number[], pageCount: number, step: -1 | 1): number | null {
  const first = pages[0];
  if (first === undefined) return null;
  const contiguous = pages.every((page, i) => page === first + i);
  if (step < 0) return first === 0 && contiguous ? null : Math.max(0, first - 1);
  const last = pageCount - pages.length;
  return first >= last && contiguous ? null : Math.min(last, first + 1);
}

/**
 * `to` for dropping `pages` before or after `target` (an index before the
 * move): how many unmoved pages precede the drop point.
 */
export function dropTarget(pages: number[], target: number, after: boolean): number {
  const point = target + (after ? 1 : 0);
  const moving = new Set(pages);
  let to = 0;
  for (let page = 0; page < point; page += 1) if (!moving.has(page)) to += 1;
  return to;
}

/** Marks the selected text (ADR 0017): one annotation per page, in one undo step. */
async function markSelection(kind: MarkupKind): Promise<void> {
  const active = selectActiveDocument(store());
  const viewer = active ? getViewer(active.id) : undefined;
  if (!active || !viewer) return;
  const pages = viewer.selectionMarks();
  if (pages.length === 0) return;
  const outcome = await backend.addMarkup(active.id, kind, pages);
  viewer.clearSelection();
  store().applyEdit(active.id, outcome);
}

/** Switches to the Comment tool in one of its modes (ADR 0018). */
function chooseCommentTool(tool: CommentTool) {
  store().setActiveTool("comment");
  store().setCommentTool(tool);
}

async function stepActive(step: "undo" | "redo"): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active) return;
  if (stepsMarks(store(), step)) {
    store().updateRedaction(active.id, step === "undo" ? undoMark : redoMark);
    return;
  }
  const outcome =
    step === "undo" ? await backend.undoEdit(active.id) : await backend.redoEdit(active.id);
  store().applyEdit(active.id, outcome);
}

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
const KNOWN_ERRORS = [
  "invalid_pdf",
  "engine_unavailable",
  "file_missing",
  "print_failed",
  "save_failed",
  "io_error",
  "edit_unsupported",
  "invalid_edit",
  "text_failed",
  "no_bookmarks",
  "redaction_failed",
];

function reportError(error: unknown) {
  const code = isBackendError(error) && KNOWN_ERRORS.includes(error.code) ? error.code : "generic";
  console.error("command failed", error);
  store().setError(code);
}

/**
 * Puts a restored recovery file in a tab (ADR 0040), dropping the tab of its
 * original that it replaces, or queues its password prompt.
 */
function acceptRestore(restored: RestoreOutcome) {
  if (restored.replaced !== null) store().removeDocument(restored.replaced);
  const { outcome } = restored;
  if (outcome.kind === "opened") {
    store().addDocument(outcome.document);
  } else {
    store().pushUnlock({
      token: outcome.token,
      name: outcome.name,
      retry: outcome.retry,
      recovery: true,
    });
  }
}

/**
 * Saves a document to its file. A recovered document whose original changed
 * since (ADR 0040) asks where instead. False when the user cancelled that.
 */
async function saveDocument(id: number): Promise<boolean> {
  try {
    store().applySave(id, await backend.saveDocument(id));
    return true;
  } catch (error) {
    if (!isBackendError(error) || error.code !== "save_as_required") throw error;
    const outcome = await backend.saveDocumentAs(id);
    if (outcome) store().applySave(id, outcome);
    return outcome !== null;
  }
}

/** Pages inserted from files: the new revision, with the inserted pages selected. */
function applyInsert(
  insert: { documentId: number; at: number; pageCount: number },
  edit: EditOutcome,
) {
  store().applyEdit(insert.documentId, edit);
  const added = Math.max(0, edit.pageCount - insert.pageCount);
  store().setSelectedPages(
    insert.documentId,
    Array.from({ length: added }, (_, i) => insert.at + i),
  );
}

/** The recovery file `token` is dealt with: it leaves the list. */
function settleRecovery(token: number) {
  store().setRecoveries(store().recoveries.filter((entry) => entry.token !== token));
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

/** Runs a search for things to redact, keeping the newest answer (ADR 0042). */
async function runRedactFind(
  active: OpenDocument,
  pattern: RedactPattern,
  field: { query: string; preset: RedactPreset | null },
) {
  const serial = active.redaction.find.serial + 1;
  store().updateRedaction(active.id, (redaction) => ({
    ...redaction,
    ...field,
    find: { ...redaction.find, status: "searching", error: null, serial },
  }));
  const current = () => store().documents.find((open) => open.id === active.id);
  try {
    const results = await backend.findRedactions(active.id, pattern);
    if (current()?.redaction.find.serial !== serial) return;
    store().updateRedaction(active.id, (redaction) => ({
      ...redaction,
      find: {
        status: "done",
        hits: results.hits,
        truncated: results.truncated,
        error: null,
        serial,
      },
    }));
  } catch (error) {
    if (current()?.redaction.find.serial !== serial) return;
    const code =
      isBackendError(error) && error.code === "invalid_query" ? "searchInvalid" : "searchFailed";
    store().updateRedaction(active.id, (redaction) => ({
      ...redaction,
      find: { status: "error", hits: [], truncated: false, error: code, serial },
    }));
  }
}

/** Marks hits for redaction: those not marked already. */
function markHits(active: OpenDocument, indices: number[]) {
  store().updateRedaction(active.id, (redaction) => {
    let next = redaction;
    for (const index of indices) {
      const hit = redaction.find.hits[index];
      if (!hit) continue;
      next = addMark(next, {
        page: hit.page,
        rects: hit.rects,
        text: hit.text,
        source: "search",
      }).state;
    }
    return next;
  });
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
      const open = store().documents.find((candidate) => candidate.id === id);
      if (open?.modified && !arg?.force) {
        store().setClosePrompt({ kind: "tab", documentId: id, name: open.name });
        return;
      }
      store().setClosePrompt(null);
      store().removeDocument(id);
      await backend.closeDocument(id);
    },
  },
  {
    id: "file.saveAndClose",
    paletteHidden: true,
    async run(arg) {
      const id = arg?.documentId;
      if (id === undefined) return;
      if (await saveDocument(id)) await runCommand("file.close", { documentId: id, force: true });
    },
  },
  {
    id: "window.closeDiscarding",
    paletteHidden: true,
    async run() {
      store().setClosePrompt(null);
      await backend.closeWindowDiscarding();
    },
  },
  {
    id: "app.quitDiscarding",
    paletteHidden: true,
    async run() {
      store().setClosePrompt(null);
      await backend.quitDiscarding();
    },
  },
  {
    id: "app.cancelClose",
    paletteHidden: true,
    run() {
      store().setClosePrompt(null);
    },
  },
  {
    id: "file.save",
    shortcut: { key: "s", mod: true },
    isEnabled: (state) => selectActiveDocument(state)?.modified ?? false,
    async run() {
      const active = selectActiveDocument(store());
      if (active) await saveDocument(active.id);
    },
  },
  {
    id: "file.saveAs",
    shortcut: { key: "s", mod: true, shift: true },
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const outcome = await backend.saveDocumentAs(active.id);
      if (outcome) store().applySave(active.id, outcome);
    },
  },
  {
    id: "edit.undo",
    shortcut: { key: "z", mod: true },
    isEnabled: canUndo,
    run: () => stepActive("undo"),
  },
  {
    id: "edit.redo",
    shortcut: { key: "z", mod: true, shift: true },
    isEnabled: canRedo,
    run: () => stepActive("redo"),
  },
  {
    id: "pages.rotateClockwise",
    isEnabled: hasDocument,
    run: () =>
      editActive(
        (_, pages) => ({ kind: "rotate", pages, quarterTurns: 1 }),
        (pages) => pages,
      ),
  },
  {
    id: "pages.rotateCounterclockwise",
    isEnabled: hasDocument,
    run: () =>
      editActive(
        (_, pages) => ({ kind: "rotate", pages, quarterTurns: -1 }),
        (pages) => pages,
      ),
  },
  {
    id: "pages.delete",
    // A document keeps at least one page.
    isEnabled: (state) =>
      targetPages(state).length > 0 &&
      targetPages(state).length < (selectActiveDocument(state)?.pageCount ?? 0),
    run: () => editActive((_, pages) => ({ kind: "delete", pages })),
  },
  {
    id: "pages.duplicate",
    isEnabled: hasDocument,
    // Each copy lands right after its page; the copies are selected.
    run: () =>
      editActive(
        (_, pages) => ({ kind: "duplicate", pages }),
        (pages) => pages.map((page, i) => page + i + 1),
      ),
  },
  {
    id: "pages.insertBlank",
    isEnabled: hasDocument,
    // After the last target page, the size of the current page (A4 until it is known).
    run: () =>
      editActive(
        (active, pages) => ({
          kind: "insertBlank",
          at: (pages[pages.length - 1] ?? active.page) + 1,
          width: active.pageSize?.width ?? 595,
          height: active.pageSize?.height ?? 842,
        }),
        (_, edit) => (edit.kind === "insertBlank" ? [edit.at] : []),
      ),
  },
  {
    id: "pages.moveEarlier",
    isEnabled: (state) =>
      moveTarget(targetPages(state), selectActiveDocument(state)?.pageCount ?? 0, -1) !== null,
    run: () =>
      editActive(
        (active, pages) => ({
          kind: "move",
          pages,
          to: moveTarget(pages, active.pageCount, -1) ?? 0,
        }),
        (pages, edit) => landed(edit, pages.length),
      ),
  },
  {
    id: "pages.moveLater",
    isEnabled: (state) =>
      moveTarget(targetPages(state), selectActiveDocument(state)?.pageCount ?? 0, 1) !== null,
    run: () =>
      editActive(
        (active, pages) => ({
          kind: "move",
          pages,
          to: moveTarget(pages, active.pageCount, 1) ?? active.pageCount,
        }),
        (pages, edit) => landed(edit, pages.length),
      ),
  },
  {
    // Drag and drop in the page organizer: `pages` land at `to` of the result.
    id: "pages.move",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const pages = arg?.pages;
      if (!active || !pages?.length || arg?.to === undefined) return;
      const to = arg.to;
      store().applyEdit(active.id, await backend.editPages(active.id, { kind: "move", pages, to }));
      store().setSelectedPages(
        active.id,
        pages.map((_, i) => to + i),
      );
    },
  },
  {
    // Merging is inserting at the end; in the organizer, after the selection.
    id: "pages.insertFromFile",
    isEnabled: hasDocument,
    async run() {
      const state = store();
      const active = selectActiveDocument(state);
      if (!active) return;
      const inOrganizer = state.activeTool === "pages" && active.selectedPages.length > 0;
      const last = active.selectedPages[active.selectedPages.length - 1] ?? 0;
      const at = inOrganizer ? last + 1 : active.pageCount;
      const outcome = await backend.insertPagesFromFiles(active.id, at);
      if (!outcome) return;
      const insert = { documentId: active.id, at, pageCount: active.pageCount };
      if (outcome.kind === "inserted") {
        applyInsert(insert, outcome.edit);
      } else {
        store().pushUnlock({ ...outcome, insert });
      }
    },
  },
  {
    id: "pages.extract",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const name = await backend.extractPages(active.id, targetPages());
      if (name) store().setNotice({ key: "extracted", values: { name } });
    },
  },
  {
    // Asks how to split, then `pages.splitInto` writes the files.
    id: "pages.split",
    isEnabled: (state) => (selectActiveDocument(state)?.pageCount ?? 0) > 1,
    run() {
      store().setSplitPrompt(true);
    },
  },
  {
    id: "pages.splitInto",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      store().setSplitPrompt(false);
      const active = selectActiveDocument(store());
      const split = arg?.split;
      if (!active || !split) return;
      if (split.kind === "pages" && (!Number.isInteger(split.size) || split.size < 1)) return;
      if (split.kind === "bytes" && !(split.limit >= 1)) return;
      const count = await backend.splitDocument(active.id, split);
      if (count !== null) store().setNotice({ key: "split", values: { count: String(count) } });
    },
  },
  {
    // Opens a page from the organizer in the page view.
    id: "pages.open",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      if (arg?.page === undefined) return;
      store().setActiveTool("select");
      await runCommand("view.goToPage", { page: arg.page });
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
      if (pending.insert) {
        const outcome = await backend.unlockInsert(pending.token, arg.password);
        if (outcome.kind === "inserted") {
          store().resolveUnlock(null);
          applyInsert(pending.insert, outcome.edit);
        } else {
          store().resolveUnlock({ ...outcome, insert: pending.insert });
        }
        return;
      }
      if (pending.recovery) {
        const restored = await backend.unlockRecovery(pending.token, arg.password);
        const { outcome } = restored;
        if (outcome.kind === "opened") {
          store().resolveUnlock(null);
          if (restored.replaced !== null) store().removeDocument(restored.replaced);
          store().addDocument(outcome.document);
        } else {
          store().resolveUnlock({ ...outcome, recovery: true });
        }
        return;
      }
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
    id: "markup.highlight",
    isEnabled: hasSelection,
    run: () => markSelection("highlight"),
  },
  {
    id: "markup.underline",
    isEnabled: hasSelection,
    run: () => markSelection("underline"),
  },
  {
    id: "markup.strikeOut",
    isEnabled: hasSelection,
    run: () => markSelection("strikeOut"),
  },
  {
    id: "markup.squiggly",
    isEnabled: hasSelection,
    run: () => markSelection("squiggly"),
  },
  {
    id: "annotation.delete",
    shortcut: { key: "Delete", alternate: "Backspace", notInFields: true },
    isEnabled: hasAnnotationSelected,
    async run() {
      const active = selectActiveDocument(store());
      const selected = active?.selectedAnnotation;
      if (!active || !selected) return;
      const outcome = await backend.removeAnnotations(active.id, selected.page, [selected.index]);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "redact.openTool",
    isEnabled: hasDocument,
    run: () => store().setActiveTool("protect"),
  },
  {
    id: "redact.addArea",
    isEnabled: hasDocument,
    paletteHidden: true,
    run(arg) {
      const active = selectActiveDocument(store());
      const { page, rect } = arg ?? {};
      if (!active || page === undefined || !rect) return;
      if (rect.width < MIN_MARK_SIDE || rect.height < MIN_MARK_SIDE) return;
      store().updateRedaction(
        active.id,
        (redaction) =>
          addMark(redaction, { page, rects: [rect], text: null, source: "area" }).state,
      );
    },
  },
  {
    id: "redact.markSelection",
    isEnabled: hasSelection,
    run() {
      const active = selectActiveDocument(store());
      const viewer = active ? getViewer(active.id) : undefined;
      if (!active || !viewer) return;
      const pages = viewer.selectionMarks();
      const text = viewer.selectedText().trim();
      store().updateRedaction(active.id, (redaction) => {
        let next = redaction;
        for (const { page, rects } of pages) {
          next = addMark(next, {
            page,
            rects,
            text: text.length > 0 ? text : null,
            source: "selection",
          }).state;
        }
        return next;
      });
      viewer.clearSelection();
      store().setActiveTool("protect");
    },
  },
  {
    id: "redact.find",
    isEnabled: hasDocument,
    paletteHidden: true,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active) return;
      const query = arg?.query ?? active.redaction.query;
      if (!query.trim()) {
        store().updateRedaction(active.id, (redaction) => ({
          ...redaction,
          query,
          preset: null,
          find: { ...redaction.find, status: "idle", hits: [], error: null },
        }));
        return;
      }
      const regex = active.redaction.regex;
      await runRedactFind(
        active,
        { kind: "text", query, options: { ...store().searchOptions, regex } },
        { query, preset: null },
      );
    },
  },
  {
    id: "redact.findPreset",
    isEnabled: hasDocument,
    paletteHidden: true,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || !arg?.preset) return;
      await runRedactFind(
        active,
        { kind: "preset", preset: arg.preset },
        { query: "", preset: arg.preset },
      );
    },
  },
  {
    id: "redact.toggleRegex",
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateRedaction(active.id, { regex: !active.redaction.regex });
    },
  },
  {
    id: "redact.markHit",
    isEnabled: hasDocument,
    paletteHidden: true,
    run(arg) {
      const active = selectActiveDocument(store());
      if (active && arg?.index !== undefined) markHits(active, [arg.index]);
    },
  },
  {
    id: "redact.markAllHits",
    isEnabled: (state) => (selectActiveDocument(state)?.redaction.find.hits.length ?? 0) > 0,
    run() {
      const active = selectActiveDocument(store());
      if (active)
        markHits(
          active,
          active.redaction.find.hits.map((_, i) => i),
        );
    },
  },
  {
    id: "redact.removeMark",
    isEnabled: hasMarks,
    paletteHidden: true,
    run(arg) {
      const active = selectActiveDocument(store());
      const id = arg?.markId;
      if (active && id !== undefined) {
        store().updateRedaction(active.id, (redaction) => removeMark(redaction, id));
      }
    },
  },
  {
    id: "redact.clearMarks",
    isEnabled: hasMarks,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateRedaction(active.id, (redaction) => ({
          ...redaction,
          marks: [],
          undone: [],
        }));
      }
    },
  },
  {
    id: "redact.undoMark",
    isEnabled: (state) => (selectActiveDocument(state)?.redaction.marks.length ?? 0) > 0,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateRedaction(active.id, undoMark);
    },
  },
  {
    id: "redact.redoMark",
    isEnabled: (state) => (selectActiveDocument(state)?.redaction.undone.length ?? 0) > 0,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateRedaction(active.id, redoMark);
    },
  },
  {
    // Asks first: applying removes content from the file.
    id: "redact.apply",
    isEnabled: hasMarks,
    run: () => store().setRedactPrompt(true),
  },
  {
    id: "redact.confirm",
    isEnabled: hasMarks,
    paletteHidden: true,
    async run() {
      const active = selectActiveDocument(store());
      store().setRedactPrompt(false);
      if (!active || active.redaction.marks.length === 0) return;
      const outcome = await backend.applyRedactions(active.id, marksForApply(active.redaction));
      // The edit starts the document over (the marks are spent); then the summary.
      store().applyEdit(active.id, outcome.edit);
      store().updateRedaction(active.id, { applied: outcome.report, sanitized: null });
    },
  },
  {
    id: "redact.cancel",
    paletteHidden: true,
    run: () => store().setRedactPrompt(false),
  },
  {
    id: "sanitize.setOption",
    paletteHidden: true,
    run(arg) {
      if (!arg?.option || arg.value === undefined) return;
      store().setSanitizeOptions({ ...store().sanitizeOptions, [arg.option]: arg.value });
    },
  },
  {
    id: "sanitize.run",
    isEnabled: (state) => hasDocument(state) && Object.values(state.sanitizeOptions).some(Boolean),
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const outcome = await backend.sanitizeDocument(active.id, store().sanitizeOptions);
      store().applyEdit(active.id, outcome.edit);
      store().updateRedaction(active.id, { sanitized: outcome.report, applied: null });
    },
  },
  {
    id: "recovery.restore",
    paletteHidden: true,
    isEnabled: (state) => state.recoveries.length > 0,
    async run(arg) {
      if (arg?.token === undefined) return;
      const restored = await backend.restoreRecovery(arg.token);
      settleRecovery(arg.token);
      acceptRestore(restored);
    },
  },
  {
    id: "recovery.discard",
    paletteHidden: true,
    isEnabled: (state) => state.recoveries.length > 0,
    async run(arg) {
      if (arg?.token === undefined) return;
      await backend.discardRecovery(arg.token);
      settleRecovery(arg.token);
    },
  },
  {
    // Offered again at the next start.
    id: "recovery.later",
    paletteHidden: true,
    isEnabled: (state) => state.recoveries.length > 0,
    run: () => store().setRecoveries([]),
  },
  { id: "comment.note", isEnabled: hasDocument, run: () => chooseCommentTool("note") },
  { id: "comment.textBox", isEnabled: hasDocument, run: () => chooseCommentTool("textBox") },
  { id: "comment.pen", isEnabled: hasDocument, run: () => chooseCommentTool("pen") },
  { id: "comment.eraser", isEnabled: hasDocument, run: () => chooseCommentTool("eraser") },
  {
    // A press with the note tool: the note is added once its text is saved.
    id: "note.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      getViewer(active.id)?.clearSelection();
      store().setPendingNote({
        documentId: active.id,
        kind: "note",
        page: arg.page,
        x: arg.x,
        y: arg.y,
        width: NOTE_SIZE,
      });
    },
  },
  {
    // A press with the text box tool: the box is added once its text is saved.
    id: "textBox.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      getViewer(active.id)?.clearSelection();
      store().setPendingNote({
        documentId: active.id,
        kind: "textBox",
        page: arg.page,
        x: arg.x,
        y: arg.y,
        width: arg.width ?? TEXT_BOX_WIDTH,
      });
    },
  },
  {
    // The Edit tool: a press places new page text there (ADR 0021).
    id: "text.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      getViewer(active.id)?.clearSelection();
      store().setPendingNote({
        documentId: active.id,
        kind: "pageText",
        page: arg.page,
        x: arg.x,
        y: arg.y,
        width: arg.width ?? TEXT_BOX_WIDTH,
      });
    },
  },
  {
    // The Edit tool pressed a paragraph of the page's own text (ADR 0023).
    id: "text.editBlock",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || !arg.block) return;
      getViewer(active.id)?.clearSelection();
      const { block } = arg;
      store().setPendingNote({
        documentId: active.id,
        kind: "block",
        page: arg.page,
        x: block.x,
        y: block.y,
        width: block.width,
        height: block.height,
        block,
      });
    },
  },
  {
    id: "text.bold",
    isEnabled: isEditingBlock,
    shortcut: { key: "b", mod: true },
    run() {
      formatBlock("bold");
    },
  },
  {
    id: "text.italic",
    isEnabled: isEditingBlock,
    shortcut: { key: "i", mod: true },
    run() {
      formatBlock("italic");
    },
  },
  {
    id: "text.setStyle",
    paletteHidden: true,
    run(arg) {
      if (arg?.textStyle) store().setTextStyle(arg.textStyle);
    },
  },
  {
    id: "note.save",
    paletteHidden: true,
    isEnabled: isWritingNote,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active) return;
      const text = arg?.text ?? "";
      const pending = store().pendingNote;
      if (pending && pending.documentId === active.id) {
        store().setPendingNote(null);
        if (pending.kind === "block" && pending.block) {
          // Unchanged text and looks are no edit; emptied text removes the paragraph (undoable).
          const { block } = pending;
          const marks = arg?.blockMarks;
          const restyled = marks !== undefined && !sameMarks(text, marks, marksOfRuns(block.runs));
          if (text === block.text && !restyled && !arg?.blockFamily) return;
          const done = await backend.editTextBlock(active.id, {
            page: pending.page,
            index: block.index,
            fingerprint: block.fingerprint,
            text,
            ...(arg?.blockFamily ? { family: arg.blockFamily } : {}),
            ...(marks ? { marks } : {}),
          });
          store().applyEdit(active.id, done);
          if (done.substituted) {
            // Font names come from the document: isolated, as <bdi> would.
            const isolate = (name: string) => `\u2068${name}\u2069`;
            const { from, font } = done.substituted;
            store().setNotice({
              key: from ? "fontSubstituted" : "fontSubstitutedUnnamed",
              values: { from: isolate(from), to: isolate(font) },
            });
          }
          return;
        }
        // An empty new note or text box is none.
        if (text.trim() === "") return;
        const { page, x, y, width } = pending;
        const outcome =
          pending.kind === "pageText"
            ? await backend.addText(active.id, page, x, y, width, text, store().textStyle)
            : pending.kind === "textBox"
              ? await backend.addTextBox(active.id, page, x, y, width, text)
              : await backend.addNote(active.id, page, x, y, text);
        store().applyEdit(active.id, outcome);
        return;
      }
      const note = active.selectedAnnotation;
      if (note?.kind !== "note" && note?.kind !== "textBox") return;
      getViewer(active.id)?.clearSelection();
      if ((note.contents ?? "") === text) return;
      // A text box emptied of its text goes.
      const outcome =
        note.kind === "textBox" && text.trim() === ""
          ? await backend.removeAnnotations(active.id, note.page, [note.index])
          : await backend.setAnnotationContents(active.id, note.page, note.index, text);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "note.cancel",
    paletteHidden: true,
    isEnabled: isWritingNote,
    run() {
      store().setPendingNote(null);
      const active = selectActiveDocument(store());
      if (active) getViewer(active.id)?.clearSelection();
    },
  },
  {
    id: "ink.add",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const page = arg?.page;
      if (!active || page === undefined || !arg?.strokes?.length) return;
      try {
        store().applyEdit(active.id, await backend.addInk(active.id, page, arg.strokes));
      } catch (error) {
        // Not added: the stroke on screen must not stay as if it were.
        getViewer(active.id)?.clearInkPreview(page);
        throw error;
      }
    },
  },
  {
    id: "ink.erase",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || !arg.indices?.length) return;
      store().applyEdit(
        active.id,
        await backend.removeAnnotations(active.id, arg.page, arg.indices),
      );
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
    isEnabled: (state) => hasSelection(state) || hasAnnotationSelected(state),
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
  { id: "view.showComments", run: () => showPanel("comments") },
  {
    // From the comments panel: the comment on its page, selected (ADR 0041).
    id: "comment.show",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      const annotation = arg?.annotation;
      if (!active || !annotation) return;
      // The organizer shows no page view: back to the pages.
      if (store().activeTool === "pages") store().setActiveTool("select");
      store().updateDocument(active.id, { page: annotation.page });
      getViewer(active.id)?.showAnnotation(annotation);
    },
  },
  {
    id: "comment.reply",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const text = arg?.text ?? "";
      if (!active || arg?.page === undefined || arg.index === undefined || text.trim() === "") {
        return;
      }
      const outcome = await backend.replyToComment(active.id, arg.page, arg.index, text);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "comment.resolve",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.index === undefined) return;
      const outcome = await backend.setCommentResolved(
        active.id,
        arg.page,
        arg.index,
        arg.resolved ?? true,
      );
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "comments.filter",
    paletteHidden: true,
    run(arg) {
      if (arg?.filter) store().setCommentFilter(arg.filter);
    },
  },
  {
    id: "comments.sort",
    paletteHidden: true,
    run(arg) {
      if (arg?.sort) store().setCommentSort(arg.sort);
    },
  },
  {
    // The name written on new comments; the shell reads the stored setting.
    id: "comments.setAuthor",
    paletteHidden: true,
    run(arg) {
      store().setCommentAuthor(arg?.text ?? "");
    },
  },
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
  const keyMatches =
    key === shortcut.key || key === shortcut.alternate || (shortcut.key === "=" && key === "+");
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
    // A Mac's delete key sends Backspace.
    Delete: mac ? "⌫" : "Del",
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
