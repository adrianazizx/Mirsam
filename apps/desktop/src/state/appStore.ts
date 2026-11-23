import { create } from "zustand";
import type {
  CompressReport,
  DetectedField,
  DocumentInfo,
  EditOutcome,
  FieldKind,
  FormField,
  NamedDestination,
  OutlineItem,
  PageSize,
  ParagraphBlock,
  RecentFile,
  RecoveryInfo,
  ReplacePlan,
  SanitizeOptions,
  SaveOutcome,
  SearchHit,
  SearchOptions,
  SecureOutcome,
  SignatureInfo,
  SignOutcome,
  TextStyle,
  ViewRect,
} from "../backend";
import type { Locale, UiDigits } from "../i18n";
import type { SelectedAnnotation } from "../viewer/controller";
import type { PageOrder, ViewMode } from "../viewer/layout";
import type { DocumentColorId } from "./colors";
import {
  type CommentFilter,
  type CommentSort,
  DEFAULT_COMMENT_FILTER,
  MAX_AUTHOR,
} from "./comments";
import type { EditMode, LinkDraft, LinkTip, SelectedObject } from "./objects";
import { DEFAULT_SANITIZE, IDLE_REDACTION, type RedactionState } from "./redaction";
import { clampZoom } from "./zoom";

/** The output dialogs (ADR 0050). */
export const OUTPUT_DIALOGS = [
  "headerFooter",
  "pageNumbers",
  "watermark",
  "exportImages",
  "exportText",
  "compress",
] as const;
export type OutputDialog = (typeof OUTPUT_DIALOGS)[number];

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

/**
 * The interface's size (Settings → Display): text and the controls around it
 * grow by this factor, the pages keep their own zoom (WCAG 1.4.4).
 */
export const UI_SCALES = [1, 1.25, 1.5, 1.75, 2] as const;
export type UiScale = (typeof UI_SCALES)[number];
export type LeftPanel = "thumbnails" | "outline" | "search" | "comments" | "signatures";

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

/** Find and replace's preview of a document (ADR 0052). */
export interface ReplaceState {
  /** "loading" while the preview is built, "replacing" while it is applied. */
  status: "idle" | "loading" | "ready" | "replacing" | "error";
  plan: ReplacePlan | null;
  /** Occurrences (indices into the plan) the user unticked: left as they are. */
  excluded: number[];
  /** Key under `errors.` when `status` is "error". */
  error: string | null;
  /** Increases with every preview; an answer for an older one is dropped. */
  serial: number;
}

export const IDLE_REPLACE: ReplaceState = {
  status: "idle",
  plan: null,
  excluded: [],
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
  /** The replace preview, while one is open (ADR 0052). */
  replace: ReplaceState;
  /** Areas marked for redaction, and the search for more (ADR 0042). */
  redaction: RedactionState;
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

/** Whether the Forms tool fills fields in or makes them (ADR 0047). */
export type FormsMode = "fill" | "design";

/** What a drag on a page draws in the form designer (ADR 0047). */
export type DesignTool = Exclude<FieldKind, "signature">;

/** The form designer's state for the active document (ADR 0047). */
export interface FormDesign {
  documentId: number;
  /** The form as last read: what the viewer outlines and the inspector edits. */
  fields: FormField[];
  /** The selected field and which of its widgets. */
  selected: { field: number; widget: number } | null;
  /** Fields found on a flat page, waiting for review; `include` as ticked. */
  detected: { page: number; fields: (DetectedField & { include: boolean })[] } | null;
}

/**
 * What a press on a page places with the Sign tool (ADR 0045); null selects
 * instead. "certificate": a visible certificate signature (ADR 0048).
 */
export type FillTool = "text" | "date" | "check" | "cross" | "dot" | "signature" | "certificate";

/**
 * A note or text box being written before it is added (ADRs 0018, 0019):
 * where it goes (its top-left corner in displayed-page points) and how wide
 * (a note's icon, or the text box).
 */
export interface PendingNote {
  documentId: number;
  /**
   * A sticky note, a text box, text written into the page (ADR 0021), a
   * paragraph of page text edited in place (ADR 0023), or Fill & Sign text (ADR 0045).
   */
  kind: "note" | "textBox" | "pageText" | "block" | "fillText";
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
  /** Digits of the Arabic UI's own numbers (page counts, zoom). */
  uiDigits: UiDigits;
  theme: ThemePreference;
  readingTheme: ReadingTheme;
  uiScale: UiScale;
  /** The open side panel at the inline start, or null when it is closed. */
  leftPanel: LeftPanel | null;
  searchOptions: SearchOptions;
  /** The search panel shows its replace row (ADR 0052). */
  replaceOpen: boolean;
  /** What matches are replaced with; a regular expression's may name its groups (`$1`). */
  replacement: string;
  setReplaceOpen(open: boolean): void;
  setReplacement(text: string): void;
  /** How the Edit tool sets new page text (ADR 0021). */
  textStyle: TextStyle;
  setTextStyle(patch: Partial<TextStyle>): void;
  /** What a press on a page does with the Edit tool: text, page objects, or links (ADR 0051). */
  editMode: EditMode;
  setEditMode(mode: EditMode): void;
  /** The page object selected in the Edit tool's object mode (ADR 0051). */
  selectedObject: SelectedObject | null;
  setSelectedObject(selected: SelectedObject | null): void;
  /** The crop being set for the selected image (displayed page), while cropping. */
  objectCrop: ViewRect | null;
  setObjectCrop(crop: ViewRect | null): void;
  /** A link being added or edited (ADR 0051). */
  linkDraft: LinkDraft | null;
  setLinkDraft(link: LinkDraft | null): void;
  /** The link under the pointer, or one pressed to follow. */
  linkTip: LinkTip | null;
  setLinkTip(tip: LinkTip | null): void;
  /** A document's named destinations, once read, for links to them. */
  destinations: { documentId: number; list: NamedDestination[] } | null;
  setDestinations(destinations: { documentId: number; list: NamedDestination[] } | null): void;
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
  /** What a press places with the Sign tool; null: it selects (ADR 0045). */
  fillTool: FillTool | null;
  /** The Forms tool fills fields in, or makes them (ADR 0047). */
  formsMode: FormsMode;
  /** What a drag draws in the form designer; null: it selects and moves fields. */
  designTool: DesignTool | null;
  formDesign: FormDesign | null;
  setFormsMode(mode: FormsMode): void;
  setDesignTool(tool: DesignTool | null): void;
  /** Changes the designer's state: a patch, or a function of it (null: none). */
  updateFormDesign(
    change: Partial<FormDesign> | ((current: FormDesign | null) => FormDesign | null),
  ): void;
  /** The kept signature `fillTool: "signature"` places. */
  fillSignature: number | null;
  /** The colour new Fill & Sign text, dates and marks are placed in (ADR 0045). */
  fillColor: DocumentColorId;
  setFillColor(color: DocumentColorId): void;
  setFill(tool: FillTool | null, signature?: number | null): void;
  /** The kept signatures and initials, newest first; null until loaded. */
  signatures: SignatureInfo[] | null;
  setSignatures(signatures: SignatureInfo[] | null): void;
  /** The dialog that makes a new signature (or initials) is open. */
  signatureDialog: { initials: boolean } | null;
  setSignatureDialog(dialog: { initials: boolean } | null): void;
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
  setUiDigits(digits: UiDigits): void;
  setTheme(theme: ThemePreference): void;
  setReadingTheme(theme: ReadingTheme): void;
  setUiScale(scale: UiScale): void;
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
  /** After a save with another protection (ADR 0043): its new protection, and undo starts over. */
  applySecured(id: number, outcome: SecureOutcome): void;
  /** After signing with a certificate (ADR 0048): the signed file is shown, and undo starts over. */
  applySigned(id: number, outcome: SignOutcome): void;
  /** The protection dialog is open. */
  securityPrompt: boolean;
  setSecurityPrompt(open: boolean): void;
  setSelectedPages(id: number, pages: number[]): void;
  closePrompt: ClosePrompt | null;
  setClosePrompt(prompt: ClosePrompt | null): void;
  updateSearch(id: number, patch: Partial<SearchState>): void;
  updateReplace(id: number, patch: Partial<ReplaceState>): void;
  /** Changes a document's redaction state: a patch, or a function of the current state. */
  updateRedaction(
    id: number,
    change: Partial<RedactionState> | ((current: RedactionState) => RedactionState),
  ): void;
  /** What sanitizing is set to take out (ADR 0042). */
  sanitizeOptions: SanitizeOptions;
  setSanitizeOptions(options: SanitizeOptions): void;
  /** The apply-redactions confirmation is open. */
  redactPrompt: boolean;
  setRedactPrompt(open: boolean): void;
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
  /** The open M6 output dialog (ADR 0050), if any. */
  outputDialog: OutputDialog | null;
  setOutputDialog(dialog: OutputDialog | null): void;
  /** The file name of the watermark image last picked (the image stays in the shell). */
  watermarkImage: string | null;
  setWatermarkImage(name: string | null): void;
  /** The last compression's sizes, for the document it was made from. */
  compressed: { documentId: number; report: CompressReport } | null;
  setCompressed(compressed: { documentId: number; report: CompressReport } | null): void;
  /** The settings dialog is open (OCR models, ADR 0034). */
  settingsOpen: boolean;
  setSettingsOpen(open: boolean): void;
  /** The keyboard shortcuts list (ADR 0070's second pass). */
  shortcutsOpen: boolean;
  setShortcutsOpen(open: boolean): void;
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
const CACHE_KEY = "mirsam.preferences";

type Persisted = Pick<
  AppState,
  | "locale"
  | "uiDigits"
  | "theme"
  | "readingTheme"
  | "uiScale"
  | "leftPanel"
  | "searchOptions"
  | "commentAuthor"
>;

const DEFAULTS: Persisted = {
  locale: "en",
  uiDigits: "arabicIndic",
  theme: "system",
  readingTheme: "normal",
  uiScale: 1,
  leftPanel: null,
  searchOptions: DEFAULT_SEARCH_OPTIONS,
  commentAuthor: "",
};

/** Parses stored strings, ignoring anything unknown or malformed. */
export function parseSettings(raw: Record<string, string | undefined>): Partial<Persisted> {
  const out: Partial<Persisted> = {};
  if (raw.locale === "en" || raw.locale === "ar") out.locale = raw.locale;
  if (raw.uiDigits === "arabicIndic" || raw.uiDigits === "western") out.uiDigits = raw.uiDigits;
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
  const scale = UI_SCALES.find((candidate) => String(candidate) === raw.uiScale);
  if (scale !== undefined) out.uiScale = scale;
  if (
    raw.leftPanel === "thumbnails" ||
    raw.leftPanel === "outline" ||
    raw.leftPanel === "search" ||
    raw.leftPanel === "comments" ||
    raw.leftPanel === "signatures"
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
    editMode: "text",
    setEditMode(editMode) {
      set({ editMode, selectedObject: null, objectCrop: null, linkDraft: null, pendingNote: null });
    },
    selectedObject: null,
    // A new selection ends a crop in progress.
    setSelectedObject: (selectedObject) => set({ selectedObject, objectCrop: null }),
    objectCrop: null,
    setObjectCrop: (objectCrop) => set({ objectCrop }),
    linkDraft: null,
    setLinkDraft: (linkDraft) => set({ linkDraft }),
    linkTip: null,
    setLinkTip: (linkTip) => set({ linkTip }),
    destinations: null,
    setDestinations: (destinations) => set({ destinations }),
    fillTool: null,
    fillSignature: null,
    fillColor: "black",
    formsMode: "fill",
    designTool: null,
    formDesign: null,
    setFormsMode(formsMode) {
      set({ formsMode, designTool: null, pendingNote: null });
    },
    setDesignTool(designTool) {
      set({ designTool });
    },
    updateFormDesign(change) {
      set((state) => ({
        formDesign:
          typeof change === "function"
            ? change(state.formDesign)
            : state.formDesign
              ? { ...state.formDesign, ...change }
              : null,
      }));
    },
    setFillColor(fillColor) {
      set({ fillColor });
    },
    setFill(fillTool, signature) {
      set((state) => ({
        fillTool,
        fillSignature: signature === undefined ? state.fillSignature : signature,
        pendingNote: null,
      }));
    },
    signatures: null,
    setSignatures(signatures) {
      set((state) => ({
        signatures,
        // A forgotten signature is no longer placed.
        ...(state.fillSignature !== null &&
        !(signatures ?? []).some((kept) => kept.id === state.fillSignature)
          ? {
              fillSignature: null,
              fillTool: state.fillTool === "signature" ? null : state.fillTool,
            }
          : {}),
      }));
    },
    signatureDialog: null,
    setSignatureDialog(signatureDialog) {
      set({ signatureDialog });
    },
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
    settingsOpen: false,
    setSettingsOpen(settingsOpen) {
      set({ settingsOpen });
    },
    shortcutsOpen: false,
    setShortcutsOpen(shortcutsOpen) {
      set({ shortcutsOpen });
    },
    outputDialog: null,
    setOutputDialog(dialog) {
      // A new dialog starts without the last compression's report.
      set({ outputDialog: dialog, compressed: null });
    },
    watermarkImage: null,
    setWatermarkImage(name) {
      set({ watermarkImage: name });
    },
    compressed: null,
    setCompressed(compressed) {
      set({ compressed });
    },
    galleryOpen: false,
    paletteOpen: false,

    setLocale: (locale) => update("locale", locale),
    setUiDigits: (uiDigits) => update("uiDigits", uiDigits),
    setTheme: (theme) => update("theme", theme),
    setReadingTheme: (readingTheme) => update("readingTheme", readingTheme),
    setUiScale: (uiScale) => update("uiScale", uiScale),
    setLeftPanel: (leftPanel) => update("leftPanel", leftPanel),
    setSearchOptions: (searchOptions) => update("searchOptions", searchOptions),
    replaceOpen: false,
    replacement: "",
    setReplaceOpen(replaceOpen) {
      set({ replaceOpen });
    },
    setReplacement(replacement) {
      set({ replacement });
    },
    setActiveTool(activeTool) {
      set({
        activeTool,
        pendingNote: null,
        selectedObject: null,
        objectCrop: null,
        linkDraft: null,
        linkTip: null,
      });
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
        // The document's own preferences (mirsam_core::catalog), else continuous,
        // left to right. Many Arabic books ask for right-to-left spreads.
        viewMode: info.pageLayout ?? "continuous",
        pageOrder: info.readingDirection === "rtl" ? "rtl" : "ltr",
        labels: null,
        outline: null,
        search: IDLE_SEARCH,
        replace: IDLE_REPLACE,
        redaction: IDLE_REDACTION,
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
          ...(state.selectedObject?.documentId === id
            ? { selectedObject: null, objectCrop: null }
            : {}),
          ...(state.linkDraft?.documentId === id ? { linkDraft: null } : {}),
          ...(state.linkTip?.documentId === id ? { linkTip: null } : {}),
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
        // Objects and links were read from the old revision: the viewer selects
        // again what an edit kept (ADR 0051).
        ...(state.selectedObject?.documentId === id
          ? { selectedObject: null, objectCrop: null }
          : {}),
        ...(state.linkDraft?.documentId === id ? { linkDraft: null } : {}),
        ...(state.linkTip?.documentId === id ? { linkTip: null } : {}),
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
                  ...(outcome.textChanged
                    ? { search: IDLE_SEARCH, replace: IDLE_REPLACE, hasSelection: false }
                    : {}),
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
                  replace: IDLE_REPLACE,
                  // Page numbers moved: marks on the old pages mean nothing now.
                  redaction: IDLE_REDACTION,
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
    applySecured(id, outcome) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id === id
            ? {
                ...open,
                name: outcome.name,
                modified: outcome.modified,
                security: outcome.security,
                canUndo: false,
                canRedo: false,
              }
            : open,
        ),
      }));
    },
    applySigned(id, outcome) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id === id
            ? {
                ...open,
                name: outcome.name,
                modified: outcome.modified,
                signed: true,
                canUndo: false,
                canRedo: false,
                selectedAnnotation: null,
                // The same pages, with a new widget on one.
                paint: open.paint + 1,
              }
            : open,
        ),
      }));
    },
    securityPrompt: false,
    setSecurityPrompt(securityPrompt) {
      set({ securityPrompt });
    },
    updateRedaction(id, change) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id !== id
            ? open
            : {
                ...open,
                redaction:
                  typeof change === "function"
                    ? change(open.redaction)
                    : { ...open.redaction, ...change },
              },
        ),
      }));
    },
    sanitizeOptions: DEFAULT_SANITIZE,
    setSanitizeOptions(sanitizeOptions) {
      set({ sanitizeOptions });
    },
    redactPrompt: false,
    setRedactPrompt(open) {
      set({ redactPrompt: open });
    },
    updateSearch(id, patch) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id === id ? { ...open, search: { ...open.search, ...patch } } : open,
        ),
      }));
    },
    updateReplace(id, patch) {
      set((state) => ({
        documents: state.documents.map((open) =>
          open.id === id ? { ...open, replace: { ...open.replace, ...patch } } : open,
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
