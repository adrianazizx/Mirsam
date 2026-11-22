import { create } from "zustand";
import type {
  DocumentInfo,
  EditOutcome,
  OutlineItem,
  PageSize,
  ParagraphBlock,
  RecentFile,
  RecoveryInfo,
  SaveOutcome,
  SearchHit,
  SearchOptions,
  TextStyle,
} from "../backend";
import type { Locale } from "../i18n";
import type { SelectedAnnotation } from "../viewer/controller";
import type { PageOrder, ViewMode } from "../viewer/layout";
import {
  type CommentFilter,
  type CommentSort,
  DEFAULT_COMMENT_FILTER,
  MAX_AUTHOR,
} from "./comments";
import { clampZoom } from "./zoom";

export const TOOLS = [
  "select",
  "edit",
  "comment",
  "pages",
  "forms",
  "sign",
  "protect",
  "ocr",
] as const;
export type ToolId = (typeof TOOLS)[number];

export type ThemePreference = "system" | "light" | "dark";
/** How pages themselves are tinted, independent of the UI theme (spec section 4). */
export type ReadingTheme = "normal" | "sepia" | "night";
export type LeftPanel = "thumbnails" | "outline" | "search" | "comments";

export interface SearchState {
  query: string;
  status: "idle" | "searching" | "done" | "error";
  hits: SearchHit[];
  truncated: boolean;
  /** The hit shown in the page view, or -1. */
  active: number;
  /** Key under `errors.` when `status` is "error". */
  error: string | null;
  /** Increases with every search; a result for an older one is dropped. */
  serial: number;
}

export const IDLE_SEARCH: SearchState = {
  query: "",
  status: "idle",
  hits: [],
  truncated: false,
  active: -1,
  error: null,
  serial: 0,
};

export interface OpenDocument extends DocumentInfo {
  /** Zero-based index of the current page (the viewer reports it as the user scrolls). */
  page: number;
  /** How far down the current page the view starts (0–1); restores a remembered position. */
  pageOffset: number;
  zoomMode: "fit-width" | "custom";
  /** Effective zoom factor (1 = 100%). In fit-width mode the viewer keeps this current. */
  zoom: number;
  /** Size of the current page, once known. */
  pageSize: PageSize | null;
  viewMode: ViewMode;
  /** Which side the earlier page of a two-page spread sits on. Independent of the UI language. */
  pageOrder: PageOrder;
  /** Page labels ("iv", "A-3"), or null when the document has none (or they are not loaded). */
  labels: (string | null)[] | null;
  /** The outline, or null until loaded. */
  outline: OutlineItem[] | null;
  search: SearchState;
  /** Whether text is selected in this document's page view. */
  hasSelection: boolean;
  /** The annotation selected (clicked) in the page view: markup, ink, or a note. */
  selectedAnnotation: SelectedAnnotation | null;
  /** Bumped by every edit, undo, and redo that changes pages: views showing pages start over. */
  revision: number;
  /**
   * Bumped when only annotations changed (ADR 0017): views keep their layout
   * and paint the pages again.
   */
  paint: number;
  canUndo: boolean;
  canRedo: boolean;
  /** Differs from the file as opened or last saved. */
  modified: boolean;
  /** Pages selected in the page organizer (zero-based, ascending). */
  selectedPages: number[];
}

/** The Comment tool's modes (ADR 0018). */
export type CommentTool = "note" | "textBox" | "pen" | "eraser";

/**
 * A note or text box being written before it is added (ADRs 0018, 0019):
 * where it goes (its top-left corner in displayed-page points) and how wide
 * (a note's icon, or the text box).
 */
export interface PendingNote {
  documentId: number;
  /** A sticky note, a text box, text written into the page (ADR 0021), or a paragraph of page text edited in place (ADR 0023). */
  kind: "note" | "textBox" | "pageText" | "block";
  page: number;
  x: number;
  y: number;
  width: number;
  /** A `block`'s box height, and the paragraph being edited. */
  height?: number;
  block?: ParagraphBlock;
}

/** A close that waits for the user: something holds unsaved changes (ADR 0015). */
export type ClosePrompt =
  | { kind: "tab"; documentId: number; name: string }
  | { kind: "window" }
  | { kind: "quit" };

export interface PendingUnlock {
  token: number;
  name: string;
  /** The last password was wrong. */
  retry: boolean;
  /** A recovery file's password, not a file's (ADR 0040). */
  recovery?: boolean;
  /** A file's whose pages are being inserted (ADR 0016): where, and the page count before. */
  insert?: { documentId: number; at: number; pageCount: number };
}

/** New page text: the sans family, 12 points, black, aligned to its start. */
export const DEFAULT_TEXT_STYLE: TextStyle = {
  family: "sans",
  size: 12,
  color: [0, 0, 0],
  align: "start",
  direction: "auto",
};

export const DEFAULT_SEARCH_OPTIONS: SearchOptions = {
  caseSensitive: false,
  wholeWord: false,
  regex: false,
  arabicNormalize: true,
  taMarbutaAsHa: false,
};

export interface AppState {
  locale: Locale;
  theme: ThemePreference;
  readingTheme: ReadingTheme;
  /** The open side panel at the inline start, or null when it is closed. */
  leftPanel: LeftPanel | null;
  searchOptions: SearchOptions;
  /** How the Edit tool sets new page text (ADR 0021). */
  textStyle: TextStyle;
  setTextStyle(patch: Partial<TextStyle>): void;
  /** The name written on new comments (ADR 0041); none when empty. */
  commentAuthor: string;
  /** The comments panel's filters and order (not kept between sessions). */
  commentFilter: CommentFilter;
  commentSort: CommentSort;
  activeTool: ToolId;
  /** What a press on a page does while the Comment tool is active. */
  commentTool: CommentTool;
  /** A note being written before it is added: where its icon goes. */
  pendingNote: PendingNote | null;
  documents: OpenDocument[];
  activeDocumentId: number | null;
  /** Unsaved changes an earlier session left, waiting for restore or discard (ADR 0040). */
  recoveries: RecoveryInfo[];
  setRecoveries(recoveries: RecoveryInfo[]): void;
  /** Encrypted files waiting for a password; the first one is being asked for. */
  unlocks: PendingUnlock[];
  /** Recent files for the start screen; null until loaded. */
  recents: RecentFile[] | null;
  /** Code of the most recent user-facing error (a key under `errors.`), or null. */
  errorCode: string | null;
  galleryOpen: boolean;
  paletteOpen: boolean;

  setLocale(locale: Locale): void;
  setTheme(theme: ThemePreference): void;
  setReadingTheme(theme: ReadingTheme): void;
  setLeftPanel(panel: LeftPanel | null): void;
  setSearchOptions(options: SearchOptions): void;
  setCommentAuthor(name: string): void;
  setCommentFilter(patch: Partial<CommentFilter>): void;
  setCommentSort(sort: CommentSort): void;
  setActiveTool(tool: ToolId): void;
  setCommentTool(tool: CommentTool): void;
  setPendingNote(note: PendingNote | null): void;
  addDocument(info: DocumentInfo, options?: { activate?: boolean }): void;
  removeDocument(id: number): void;
  activateDocument(id: number): void;
  /** Moves a tab to `index` in tab order (reading order). */
  moveDocument(id: number, index: number): void;
  updateDocument(id: number, patch: Partial<Omit<OpenDocument, keyof DocumentInfo>>): void;
  /**
   * Takes the document to the revision an edit, undo, or redo left it at.
   * Page numbers may have moved, so the outline, labels, search hits, and
   * selection are dropped and reload; the current page stays where it can.
   */
  applyEdit(id: number, outcome: EditOutcome): void;
  /** After Save or Save As: the name may change, and the document is no longer modified. */
  applySave(id: number, outcome: SaveOutcome): void;
  setSelectedPages(id: number, pages: number[]): void;
  closePrompt: ClosePrompt | null;
  setClosePrompt(prompt: ClosePrompt | null): void;
  updateSearch(id: number, patch: Partial<SearchState>): void;
  pushUnlock(unlock: PendingUnlock): void;
  /** Replaces the first pending unlock (e.g. after a wrong password) or drops it (null). */
  resolveUnlock(next: PendingUnlock | null): void;
  setRecents(recents: RecentFile[] | null): void;
  setError(code: string | null): void;
  /** A message about something done elsewhere (a file written), under `notices.`. */
  notice: { key: string; values?: Record<string, string> } | null;
  setNotice(notice: { key: string; values?: Record<string, string> } | null): void;
  /** The split dialog is open (asks how many pages per file). */
  splitPrompt: boolean;
  setSplitPrompt(open: boolean): void;
  setGalleryOpen(open: boolean): void;
  setPaletteOpen(open: boolean): void;
  /** Applies settings read from the shell's store at startup. */
  hydrate(settings: Record<string, string>): void;
  /** Applies a setting another window stored (not written back). */
  applySetting(key: string, value: string): void;
}

/**
 * Settings persist in the shell's SQLite store (spec section 9, "Storage").
 * `localStorage` keeps a copy so the first frame already has the right
 * language and theme, before the store answers.
 */
const CACHE_KEY = "safha.preferences";

type Persisted = Pick<
  AppState,
  "locale" | "theme" | "readingTheme" | "leftPanel" | "searchOptions" | "commentAuthor"
>;

const DEFAULTS: Persisted = {
  locale: "en",
  theme: "system",
  readingTheme: "normal",
  leftPanel: null,
  searchOptions: DEFAULT_SEARCH_OPTIONS,
  commentAuthor: "",
};

/** Parses stored strings, ignoring anything unknown or malformed. */
export function parseSettings(raw: Record<string, string | undefined>): Partial<Persisted> {
  const out: Partial<Persisted> = {};
  if (raw.locale === "en" || raw.locale === "ar") out.locale = raw.locale;
  if (raw.theme === "system" || raw.theme === "light" || raw.theme === "dark") {
    out.theme = raw.theme;
  }
  if (
    raw.readingTheme === "normal" ||
    raw.readingTheme === "sepia" ||
    raw.readingTheme === "night"
  ) {
    out.readingTheme = raw.readingTheme;
  }
  if (
    raw.leftPanel === "thumbnails" ||
    raw.leftPanel === "outline" ||
    raw.leftPanel === "search" ||
    raw.leftPanel === "comments"
  ) {
    out.leftPanel = raw.leftPanel;
  } else if (raw.leftPanel === "none") {
    out.leftPanel = null;
  }
  if (typeof raw.commentAuthor === "string") {
    out.commentAuthor = raw.commentAuthor.trim().slice(0, MAX_AUTHOR);
  }
  if (raw["search.options"]) {
    try {
      const parsed = JSON.parse(raw["search.options"]) as Partial<SearchOptions>;
      const options = { ...DEFAULT_SEARCH_OPTIONS };
      for (const key of Object.keys(DEFAULT_SEARCH_OPTIONS) as (keyof SearchOptions)[]) {
        if (typeof parsed[key] === "boolean") options[key] = parsed[key];
      }
      out.searchOptions = options;
    } catch {
      // A malformed value keeps the defaults.
    }
  }
  return out;
}

function serialize(key: keyof Persisted, state: Persisted): [string, string] {
  switch (key) {
    case "leftPanel":
      return ["leftPanel", state.leftPanel ?? "none"];
    case "searchOptions":
      return ["search.options", JSON.stringify(state.searchOptions)];
    default:
      return [key, String(state[key])];
  }
}

function loadCached(): Persisted {
  try {
    const raw = localStorage.getItem(CACHE_KEY);
    if (!raw) return DEFAULTS;
    return { ...DEFAULTS, ...parseSettings(JSON.parse(raw) as Record<string, string>) };
  } catch {
    return DEFAULTS;
  }
}

/** Where settings are written; the backend's store in the app (set by `App`). */
let persistSetting: (key: string, value: string) => void = () => {};
export function setSettingsWriter(writer: (key: string, value: string) => void) {
  persistSetting = writer;
}

function writeCache(state: Persisted) {
  try {
    const cached: Record<string, string> = {};
    for (const key of Object.keys(DEFAULTS) as (keyof Persisted)[]) {
      const [name, value] = serialize(key, state);
      cached[name] = value;
    }
    localStorage.setItem(CACHE_KEY, JSON.stringify(cached));
  } catch {
    // Storage can be unavailable (private mode, quota); the store still has it.
  }
}

function persist(key: keyof Persisted, state: Persisted) {
  const [name, value] = serialize(key, state);
  persistSetting(name, value);
  writeCache(state);
}

/** Settings the user changed since launch; `hydrate` leaves them alone. */
const changedSinceLaunch = new Set<keyof Persisted>();

/** For tests: forget which settings were changed since launch. */
export function resetSettingsChanges(): void {
  changedSinceLaunch.clear();
}

function initialZoom(info: DocumentInfo): Pick<OpenDocument, "zoomMode" | "zoom"> {
  const saved = info.viewState;
  if (saved?.zoomMode === "custom" && Number.isFinite(saved.zoom)) {
    return { zoomMode: "custom", zoom: clampZoom(saved.zoom) };
  }
  return { zoomMode: "fit-width", zoom: 1 };
}

export const useAppStore = create<AppState>((set, get) => {
  const update = <K extends keyof Persisted>(key: K, value: Persisted[K]) => {
    changedSinceLaunch.add(key);
    set({ [key]: value } as Partial<AppState>);
    persist(key, get());
  };

  return {
    ...loadCached(),
    activeTool: "select",
    commentTool: "note",
    pendingNote: null,
    commentFilter: DEFAULT_COMMENT_FILTER,
    commentSort: "page",
    textStyle: DEFAULT_TEXT_STYLE,
    setTextStyle(patch) {
      set((state) => ({ textStyle: { ...state.textStyle, ...patch } }));
    },
    setCommentAuthor: (name) => update("commentAuthor", name.trim().slice(0, MAX_AUTHOR)),
    setCommentFilter(patch) {
      set((state) => ({ commentFilter: { ...state.commentFilter, ...patch } }));
    },
    setCommentSort(commentSort) {
      set({ commentSort });
    },
    recoveries: [],
    setRecoveries(recoveries) {
      set({ recoveries });
    },
    documents: [],
    activeDocumentId: null,
    unlocks: [],
    recents: null,
    errorCode: null,
    notice: null,
    setNotice(notice) {
      set({ notice });
    },
    splitPrompt: false,
    setSplitPrompt(open) {
      set({ splitPrompt: open });
    },
    galleryOpen: false,
    paletteOpen: false,

    setLocale: (locale) => update("locale", locale),
    setTheme: (theme) => update("theme", theme),
    setReadingTheme: (readingTheme) => update("readingTheme", readingTheme),
    setLeftPanel: (leftPanel) => update("leftPanel", leftPanel),
    setSearchOptions: (searchOptions) => update("searchOptions", searchOptions),
    setActiveTool(activeTool) {
      set({ activeTool, pendingNote: null });
    },
    setCommentTool(commentTool) {
      set({ commentTool, pendingNote: null });
    },
    setPendingNote(pendingNote) {
      set({ pendingNote });
    },
    addDocument(info, options) {
      if (get().documents.some((open) => open.id === info.id)) {
        if (options?.activate !== false) set({ activeDocumentId: info.id, errorCode: null });
        return;
      }
      const lastPage = Math.max(0, info.pageCount - 1);
      const saved = info.viewState;
      const document: OpenDocument = {
        ...info,
        page: saved ? Math.min(Math.max(0, Math.trunc(saved.page)), lastPage) : 0,
        pageOffset: saved ? Math.min(Math.max(0, saved.pageOffset), 1) : 0,
        ...initialZoom(info),
        pageSize: null,
        // The document's own preferences (safha_core::catalog), else continuous,
        // left to right. Many Arabic books ask for right-to-left spreads.
        viewMode: info.pageLayout ?? "continuous",
        pageOrder: info.readingDirection === "rtl" ? "rtl" : "ltr",
        labels: null,
        outline: null,
        search: IDLE_SEARCH,
        hasSelection: false,
        selectedAnnotation: null,
        revision: 0,
        paint: 0,
        canUndo: info.canUndo ?? false,
        canRedo: false,
        modified: info.modified ?? false,
        selectedPages: [],
      };
      set((state) => ({
        documents: [...state.documents, document],
        activeDocumentId: options?.activate === false ? state.activeDocumentId : info.id,
        errorCode: null,
      }));
    },
    removeDocument(id) {
      set((state) => {
        const index = state.documents.findIndex((open) => open.id === id);
        const documents = state.documents.filter((open) => open.id !== id);
        const neighbour = documents[Math.min(index, documents.length - 1)];
        return {
          documents,
          activeDocumentId:
            state.activeDocumentId === id ? (neighbour?.id ?? null) : state.activeDocumentId,
        };
      });
    },
    activateDocument(id) {
      if (get().documents.some((open) => open.id === id)) set({ activeDocumentId: id });
    },
    moveDocument(id, index) {
      set((state) => {
        const from = state.documents.findIndex((open) => open.id === id);
        const moving = state.documents[from];
        if (!moving) return {};
        const rest = state.documents.filter((open) => open.id !== id);
        const to = Math.min(Math.max(0, Math.trunc(index)), rest.length);
        if (to === from) return {};
        return { documents: [...rest.slice(0, to), moving, ...rest.slice(to)] };
      });
    },
    updateDocument(id, patch) {
      set((state) => ({
        documents: state.documents.map((open) => {
          if (open.id !== id) return open;
          const next = { ...open, ...patch };
          next.page = Math.min(Math.max(0, next.page), Math.max(0, open.pageCount - 1));
          next.zoom = clampZoom(next.zoom);
          return next;
        }),
      }));
    },
    applyEdit(id, outcome) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id !== id
            ? open
            : !outcome.pagesChanged
              ? {
                  // The same pages: everything on them stays, they are painted again.
                  ...open,
                  paint: open.paint + 1,
                  selectedAnnotation: null,
                  // Text written into them: what search found, and selected, is stale.
                  ...(outcome.textChanged ? { search: IDLE_SEARCH, hasSelection: false } : {}),
                  canUndo: outcome.canUndo,
                  canRedo: outcome.canRedo,
                  modified: outcome.modified,
                }
              : {
                  ...open,
                  pageCount: outcome.pageCount,
                  page: Math.min(open.page, Math.max(0, outcome.pageCount - 1)),
                  pageSize: null,
                  labels: null,
                  outline: null,
                  search: IDLE_SEARCH,
                  hasSelection: false,
                  selectedAnnotation: null,
                  revision: open.revision + 1,
                  // Page numbers moved; the command that edited says what is selected next.
                  selectedPages: [],
                  canUndo: outcome.canUndo,
                  canRedo: outcome.canRedo,
                  modified: outcome.modified,
                },
        ),
      }));
    },
    closePrompt: null,
    setClosePrompt(prompt) {
      set({ closePrompt: prompt });
    },
    setSelectedPages(id, pages) {
      const sorted = [...new Set(pages)].sort((a, b) => a - b);
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id === id
            ? {
                ...open,
                selectedPages: sorted.filter((page) => page >= 0 && page < open.pageCount),
              }
            : open,
        ),
      }));
    },
    applySave(id, outcome) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id === id ? { ...open, name: outcome.name, modified: outcome.modified } : open,
        ),
      }));
    },
    updateSearch(id, patch) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id === id ? { ...open, search: { ...open.search, ...patch } } : open,
        ),
      }));
    },
    pushUnlock(unlock) {
      set((state) => ({
        unlocks: [...state.unlocks.filter((pending) => pending.token !== unlock.token), unlock],
      }));
    },
    resolveUnlock(next) {
      set((state) => ({
        unlocks: next ? [next, ...state.unlocks.slice(1)] : state.unlocks.slice(1),
      }));
    },
    setRecents(recents) {
      set({ recents });
    },
    setError(errorCode) {
      set({ errorCode });
    },
    setGalleryOpen(galleryOpen) {
      set({ galleryOpen });
    },
    setPaletteOpen(paletteOpen) {
      set({ paletteOpen });
    },
    applySetting(key, value) {
      const parsed = parseSettings({ [key]: value });
      if (Object.keys(parsed).length === 0) return;
      set(parsed);
      writeCache(get());
    },
    hydrate(settings) {
      // The store is the source of truth: a setting it does not have is the
      // default, whatever the first-paint copy said (a cleared store, or a
      // self-test's throwaway one, must not inherit an earlier session's).
      // A setting changed since launch, before the store answered, stays.
      const stored: Partial<Persisted> = { ...DEFAULTS, ...parseSettings(settings) };
      for (const key of changedSinceLaunch) delete stored[key];
      set(stored);
      // Refresh the first-paint copy from the source of truth.
      writeCache(get());
    },
  };
});

export function selectActiveDocument(state: AppState): OpenDocument | null {
  return state.documents.find((open) => open.id === state.activeDocumentId) ?? null;
}

/** "iv" for a labelled page, otherwise its number; `page` is zero-based. */
export function pageLabelOf(document: Pick<OpenDocument, "labels">, page: number): string | null {
  return document.labels?.[page] ?? null;
}
