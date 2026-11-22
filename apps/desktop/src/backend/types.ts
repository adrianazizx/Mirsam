/** Contract between the UI and the Rust shell. Mirrors apps/desktop/src-tauri/src/{commands,viewer}.rs. */

export interface AppInfo {
  version: string;
  devtools: boolean;
}

export interface DocumentInfo {
  id: number;
  /** File name only; the UI never sees filesystem paths. */
  name: string;
  pageCount: number;
  /** Where the file was left the last time it was open, if ever. */
  viewState?: SavedViewState | null;
  /**
   * The document's own reading preferences (its catalog's /ViewerPreferences
   * /Direction and /PageLayout), the defaults for page order and layout.
   */
  readingDirection?: "ltr" | "rtl" | null;
  pageLayout?: "single" | "continuous" | "spread" | "book" | null;
}

/** A document's remembered position (safha_core::store::ViewState). */
export interface SavedViewState {
  page: number;
  /** "fit-width" or "custom"; anything else is treated as fit-width. */
  zoomMode: string;
  zoom: number;
  /** How far down the page the view's top edge was, 0 to 1. */
  pageOffset: number;
}

/** What opening a file led to. */
export type OpenOutcome =
  | { kind: "opened"; document: DocumentInfo }
  /** Encrypted: continue with `unlockDocument(token, password)`. `retry` means the last password was wrong. */
  | { kind: "needsPassword"; token: number; name: string; retry: boolean };

export interface Startup {
  /** Restored tabs and the launch document, in tab order. */
  documents: OpenOutcome[];
  /** The document to show first. */
  active: number | null;
}

export interface RecentFile {
  /** Names the file in `openRecent`/`forgetRecent`; valid until the next `recentFiles` call. */
  token: number;
  name: string;
  /** Unix seconds. */
  lastOpened: number;
  /** False when the file has been moved or deleted since. */
  exists: boolean;
}

export interface OutlineItem {
  /** Logical-order text; may be in either script. */
  title: string;
  /** Zero-based target page, when the item points into this document. */
  page: number | null;
  /** Target height on the page in PDF points from the bottom (y up), when given. */
  top: number | null;
  children: OutlineItem[];
}

export type Direction = "ltr" | "rtl";

/** A rectangle on the displayed page: points from its top-left corner, y down. */
export interface ViewRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface TextSpan {
  /** UTF-16 indices into the line's text. */
  start: number;
  end: number;
  /** Absent for inferred word spaces, which have no glyph. */
  rect: ViewRect | null;
}

export interface TextLine {
  /** Logical (reading) order. */
  text: string;
  /** Logical order. */
  spans: TextSpan[];
  bounds: ViewRect;
}

export interface TextBlock {
  direction: Direction;
  /** Top to bottom. */
  lines: TextLine[];
}

/** A page's text in reading order with character boxes (safha_core::text). */
export interface TextLayer {
  direction: Direction;
  /** "suspect": the page's character mapping looks broken (copy will be wrong; OCR would help). */
  quality: "good" | "suspect" | "noText";
  /** Reading order. */
  blocks: TextBlock[];
}

export interface SearchOptions {
  caseSensitive: boolean;
  wholeWord: boolean;
  regex: boolean;
  /** Ignore tashkeel and tatweel; unify alef forms, ى/ي, digit styles (spec 6.2). */
  arabicNormalize: boolean;
  /** Also treat ة and ه as equal. */
  taMarbutaAsHa: boolean;
}

export interface SearchHit {
  page: number;
  block: number;
  line: number;
  /** UTF-16 indices into the line's text. */
  start: number;
  end: number;
  direction: Direction;
  rects: ViewRect[];
  context: { before: string; matched: string; after: string };
}

export interface SearchResults {
  hits: SearchHit[];
  /** More matches exist than were returned. */
  truncated: boolean;
}

/**
 * Render-queue lane. Each lane has its own viewport generation, so the
 * thumbnail panel and the page view never cancel each other's requests.
 */
export type Lane = "main" | "thumbnails";

/** Page dimensions in PDF points (1/72 inch). Exact f32 values from the engine. */
export interface PageSize {
  width: number;
  height: number;
}

/**
 * A rectangle of a page at a scale. `scale` is device pixels per PDF point and
 * must be an f32 value (`Math.fround`); the whole page is
 * `ceil(width * scale) x ceil(height * scale)` pixels in f32 arithmetic, and
 * the rectangle must lie inside it (see `viewer/geometry.ts`).
 */
export interface TileRequest {
  scale: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Render-queue classes, most urgent first (safha_core::render::Urgency). */
export type Urgency = "placeholder" | "visible" | "thumbnail" | "prefetch";

export interface Priority {
  urgency: Urgency;
  /** Distance from the viewport centre in CSS pixels; nearer renders first. */
  distance: number;
}

/** A queued request that is still wanted after the viewport moved. */
export interface KeptRequest {
  page: number;
  tile: TileRequest;
  priority: Priority;
}

/**
 * A rendered rectangle, decoded and ready to draw (see `backend/rendered.ts`).
 * Whoever receives one owns it and must `releaseRendered` it once drawn or
 * dropped: an `ImageBitmap` holds its pixels until closed.
 */
export interface RenderedPage {
  width: number;
  height: number;
  /**
   * Time the engine spent rasterizing, in microseconds, and whether the tile
   * came from its cache — null where the transport does not say (the shell's
   * image transport; its render queue counts both, see `renderStats`).
   */
  renderMicros: number | null;
  fromCache: boolean | null;
  /**
   * From the shell: an image element the webview loaded and decoded from a
   * BMP, so the pixels never became a JavaScript buffer (ADR 0011). From the
   * in-browser mock: `ImageData`.
   */
  image: HTMLImageElement | ImageBitmap | ImageData;
}

/**
 * A tile the image transport did not deliver: the engine cancelled it (the
 * viewport moved on) or failed it. An image element cannot read the response,
 * so the two look alike; the tile client tells them apart by generation.
 */
export function isTileUnavailable(value: unknown): boolean {
  return isBackendError(value) && value.code === "tile_unavailable";
}

/** Render-queue counters (devtools builds only). */
export interface RenderStats {
  submitted: number;
  cacheHits: number;
  coalesced: number;
  rendered: number;
  failed: number;
  cancelled: number;
  renderMicros: number;
  pageLoads: number;
  cacheBytes: number;
  cacheCapacityBytes: number;
  cacheEntries: number;
  cacheEvictions: number;
  pending: number;
}

export interface SelftestConfig {
  locale: string;
  theme: string;
  /** Name of a spike or benchmark to run instead of the plain self-test. */
  spike: string | null;
}

/** Error payload of a rejected command. `code` maps to a localized message. */
export interface BackendError {
  code: string;
  detail: string;
}

export function isBackendError(value: unknown): value is BackendError {
  return (
    typeof value === "object" && value !== null && typeof (value as BackendError).code === "string"
  );
}

/** A render that was dropped because the viewport moved on. Expected; never shown to the user. */
export function isCancelled(value: unknown): boolean {
  return isBackendError(value) && value.code === "cancelled";
}

export interface Backend {
  appInfo(): Promise<AppInfo>;
  /** Shows the native picker. Resolves to null when the user cancels. */
  openDocumentDialog(): Promise<OpenOutcome | null>;
  /**
   * What to show at launch: the previous session's tabs and the launch
   * document. After a UI reload, the documents still open in the shell.
   */
  startup(): Promise<Startup>;
  unlockDocument(token: number, password: string): Promise<OpenOutcome>;
  cancelUnlock(token: number): Promise<void>;
  recentFiles(limit: number): Promise<RecentFile[]>;
  /** Rejects with code `file_missing` when the file is gone. */
  openRecent(token: number): Promise<OpenOutcome>;
  forgetRecent(token: number): Promise<void>;
  clearRecents(): Promise<void>;
  saveViewState(documentId: number, view: SavedViewState): Promise<void>;
  /** The open tabs in order, restored at the next launch. */
  saveSession(documentIds: number[], active: number | null): Promise<void>;
  /** Stored UI settings (`locale`, `theme`, `readingTheme`, `leftPanel`, `search.options`). */
  settings(): Promise<Record<string, string>>;
  setSetting(key: string, value: string): Promise<void>;
  /** Every page's size, read without loading pages. */
  pageSizes(documentId: number): Promise<PageSize[]>;
  documentOutline(documentId: number): Promise<OutlineItem[]>;
  /** Every page's label, or null when the document defines none. */
  pageLabels(documentId: number): Promise<(string | null)[] | null>;
  pageText(documentId: number, page: number): Promise<TextLayer>;
  /**
   * Searches the whole document. The first search builds the index. Rejects
   * with code `invalid_query` (empty, bad regex) or `cancelled` (closed).
   */
  searchDocument(documentId: number, query: string, options: SearchOptions): Promise<SearchResults>;
  /**
   * Renders one rectangle of a page through the engine's render queue.
   * Rejects with code `cancelled` (see `isCancelled`) when a later
   * `updateViewport` dropped it before it started.
   */
  renderTile(
    documentId: number,
    generation: number,
    page: number,
    tile: TileRequest,
    priority: Priority,
    lane?: Lane,
  ): Promise<RenderedPage>;
  /** Advances the lane's viewport generation; queued renders not in `keep` are cancelled. */
  updateViewport(
    documentId: number,
    generation: number,
    keep: KeptRequest[],
    lane?: Lane,
  ): Promise<void>;
  closeDocument(documentId: number): Promise<void>;
  /** Opens an empty window. */
  newWindow(): Promise<void>;
  /**
   * Prints a document through the platform's own PDF viewer and print panel
   * (ADR 0012; unverified on screen so far).
   */
  printDocument(documentId: number): Promise<void>;
  /** Gives the document's tab to a new window; the caller drops its own tab. */
  moveToNewWindow(documentId: number): Promise<void>;
  /**
   * Calls `listener` whenever any window stores a setting, so every window
   * follows a change made in one. Returns a function that stops listening.
   */
  onSettingChanged(listener: (key: string, value: string) => void): Promise<() => void>;
  selftestConfig(): Promise<SelftestConfig | null>;
  selftestReport(results: unknown, canvasPng: string | null): Promise<void>;
  /** Devtools builds only; null elsewhere. */
  renderStats(): Promise<RenderStats | null>;
  /** Labels of the open windows; devtools builds only. */
  windowLabels(): Promise<string[] | null>;
  /** Resident memory of the shell process in kilobytes; devtools builds only. */
  shellRssKb(): Promise<number | null>;
  /**
   * Resident memory of the webview's own processes (WebKit content and GPU
   * processes on macOS), found heuristically; devtools builds only.
   */
  webviewRssKb(): Promise<number | null>;
  /** Writes a progress line to the shell's log during a self-test; devtools builds only. */
  selftestLog(message: string): Promise<void>;
}
