import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  type AppInfo,
  type Backend,
  type EditOutcome,
  isBackendError,
  type OpenOutcome,
  type OutlineItem,
  type PageAnnotation,
  type PageSize,
  type RecentFile,
  type RenderedPage,
  type RenderStats,
  type SaveOutcome,
  type SearchResults,
  type SelftestConfig,
  type Startup,
  type TextLayer,
} from "./types";

/**
 * A tile that has not arrived after this long is given up and asked for again
 * at the next viewport update. A render takes milliseconds; this only guards
 * against a load that never settles (seen twice in ~40 experiment runs, cause
 * unknown; ADR 0011), which would otherwise leave a page on its preview.
 */
const TILE_TIMEOUT_MS = 10_000;

let tileUrl: string | null = null;
/**
 * The shell's tile protocol: `safha://localhost/tile` (`http://safha.localhost/tile`
 * on Windows). Asked of Tauri on first use: the module also loads outside Tauri.
 */
function tileBase(): string {
  tileUrl ??= convertFileSrc("tile", "safha");
  return tileUrl;
}

/**
 * Tiles are fetched as BMP images and decoded by the webview itself (ADR
 * 0011): the pixels never become a JavaScript buffer, which keeps WebKit's
 * memory flat. Failures reject with the shell's error code (`cancelled` when
 * the viewport moved on).
 */
/** The URL of one tile (exported for the memory experiment, spikes/tileMemory.ts). */
export function tileUrlFor(query: URLSearchParams): string {
  return `${tileBase()}?${query}`;
}

/**
 * Tiles are BMP images the webview loads and decodes itself (ADR 0011): the
 * pixels never become a JavaScript buffer or Blob, which keeps WebKit's memory
 * flat (+36 MB per 2 GB of tiles, against +120–190 MB for every route that
 * hands the bytes to JavaScript). An image element cannot read the response,
 * so a cancelled or failed tile rejects as `tile_unavailable` and the tile
 * client tells the two apart by generation.
 */
async function loadTile(query: URLSearchParams): Promise<RenderedPage> {
  const image = new Image();
  // The scheme answers with Access-Control-Allow-Origin: *, so canvases the
  // tile is drawn into stay readable (the self-test reads them back).
  image.crossOrigin = "anonymous";
  image.decoding = "async";
  image.src = tileUrlFor(query);
  let timer: number | undefined;
  const timeout = new Promise<"timeout">((resolve) => {
    timer = window.setTimeout(() => resolve("timeout"), TILE_TIMEOUT_MS);
  });
  try {
    if ((await Promise.race([image.decode(), timeout])) === "timeout") {
      image.src = "";
      // Treated like a cancellation: asked for again at the next viewport update.
      throw { code: "cancelled", detail: "the tile did not arrive in time" };
    }
  } catch (error) {
    if (isBackendError(error)) throw error;
    throw { code: "tile_unavailable", detail: "the tile was cancelled or failed" };
  } finally {
    window.clearTimeout(timer);
  }
  return {
    width: image.naturalWidth,
    height: image.naturalHeight,
    renderMicros: null,
    fromCache: null,
    image,
  };
}

let devtools: Promise<boolean> | null = null;
/** Whether the shell was built with the `devtools` feature (asked once). */
function hasDevtools(): Promise<boolean> {
  devtools ??= invoke<AppInfo>("app_info").then((info) => info.devtools);
  return devtools;
}

/** The real backend: typed wrappers over the Rust shell's commands. */
export const tauriBackend: Backend = {
  appInfo: () => invoke<AppInfo>("app_info"),
  openDocumentDialog: () => invoke<OpenOutcome | null>("open_document_dialog"),
  startup: () => invoke<Startup>("startup"),
  unlockDocument: (token, password) => invoke<OpenOutcome>("unlock_document", { token, password }),
  cancelUnlock: (token) => invoke<void>("cancel_unlock", { token }),
  recentFiles: (limit) => invoke<RecentFile[]>("recent_files", { limit }),
  openRecent: (token) => invoke<OpenOutcome>("open_recent", { token }),
  forgetRecent: (token) => invoke<void>("forget_recent", { token }),
  clearRecents: () => invoke<void>("clear_recents"),
  saveViewState: (documentId, view) => invoke<void>("save_view_state", { documentId, view }),
  saveSession: (documentIds, active) => invoke<void>("save_session", { documentIds, active }),
  settings: () => invoke<Record<string, string>>("settings"),
  setSetting: (key, value) => invoke<void>("set_setting", { key, value }),
  pageSizes: (documentId) => invoke<PageSize[]>("page_sizes", { documentId }),
  documentOutline: (documentId) => invoke<OutlineItem[]>("document_outline", { documentId }),
  pageLabels: (documentId) => invoke<(string | null)[] | null>("page_labels", { documentId }),
  pageText: (documentId, page) => invoke<TextLayer>("page_text", { documentId, page }),
  searchDocument: (documentId, query, options) =>
    invoke<SearchResults>("search_document", { documentId, query, options }),
  renderTile: (documentId, generation, page, tile, priority, lane = "main") =>
    loadTile(
      new URLSearchParams({
        doc: String(documentId),
        lane,
        gen: String(generation),
        page: String(page),
        // An f32 value printed in full; the shell narrows it back exactly.
        scale: String(tile.scale),
        x: String(tile.x),
        y: String(tile.y),
        w: String(tile.width),
        h: String(tile.height),
        u: priority.urgency,
        d: String(priority.distance),
      }),
    ),
  updateViewport: (documentId, generation, keep, lane = "main") =>
    invoke<void>("update_viewport", { documentId, generation, keep, lane }),
  closeDocument: async (documentId) => {
    await invoke<boolean>("close_document", { documentId });
  },
  newWindow: () => invoke<void>("new_window"),
  printDocument: (documentId) => invoke<void>("print_document", { documentId }),
  editPages: (documentId, edit) => invoke<EditOutcome>("edit_pages", { documentId, edit }),
  undoEdit: (documentId) => invoke<EditOutcome>("undo_edit", { documentId }),
  redoEdit: (documentId) => invoke<EditOutcome>("redo_edit", { documentId }),
  addMarkup: (documentId, kind, pages) =>
    invoke<EditOutcome>("add_markup", { documentId, kind, pages }),
  addInk: (documentId, page, strokes) =>
    invoke<EditOutcome>("add_ink", { documentId, page, strokes }),
  addNote: (documentId, page, x, y, contents) =>
    invoke<EditOutcome>("add_note", { documentId, page, x, y, contents }),
  addTextBox: (documentId, page, x, y, width, contents) =>
    invoke<EditOutcome>("add_text_box", { documentId, page, x, y, width, contents }),
  setAnnotationContents: (documentId, page, index, contents) =>
    invoke<EditOutcome>("set_annotation_contents", { documentId, page, index, contents }),
  removeAnnotations: (documentId, page, indices) =>
    invoke<EditOutcome>("remove_annotations", { documentId, page, indices }),
  pageAnnotations: (documentId, page) =>
    invoke<PageAnnotation[]>("page_annotations", { documentId, page }),
  // Addressed to one window: listen on this window only.
  onCloseRequested: (listener) =>
    getCurrentWebviewWindow().listen("close-requested", () => listener()),
  onQuitRequested: (listener) =>
    getCurrentWebviewWindow().listen("quit-requested", () => listener()),
  closeWindowDiscarding: () => invoke<void>("close_window_discarding"),
  quitDiscarding: () => invoke<void>("quit_discarding"),
  saveDocument: (documentId) => invoke<SaveOutcome>("save_document", { documentId }),
  insertPagesFromFiles: (documentId, at) =>
    invoke<EditOutcome | null>("insert_pages_from_files", { documentId, at }),
  extractPages: (documentId, pages) =>
    invoke<string | null>("extract_pages", { documentId, pages }),
  splitDocument: (documentId, size) =>
    invoke<number | null>("split_document", { documentId, size }),
  saveDocumentAs: (documentId) => invoke<SaveOutcome | null>("save_document_as", { documentId }),
  moveToNewWindow: (documentId) => invoke<void>("move_to_new_window", { documentId }),
  onSettingChanged: (listener) =>
    listen<{ key: string; value: string }>("settings-changed", (event) =>
      listener(event.payload.key, event.payload.value),
    ),
  // The commands below exist only in `devtools` builds of the shell.
  selftestConfig: async () =>
    (await hasDevtools()) ? invoke<SelftestConfig | null>("selftest_config") : null,
  selftestReport: (results, canvasPng) =>
    invoke<void>("selftest_report", { report: { results, canvasPng } }),
  renderStats: async () => ((await hasDevtools()) ? invoke<RenderStats>("render_stats") : null),
  windowLabels: async () => ((await hasDevtools()) ? invoke<string[]>("window_labels") : null),
  shellRssKb: async () => ((await hasDevtools()) ? invoke<number | null>("spike_rss_kb") : null),
  selftestLog: async (message) => {
    if (await hasDevtools()) await invoke<void>("selftest_log", { message });
  },
  webviewRssKb: async () =>
    (await hasDevtools()) ? invoke<number | null>("webview_rss_kb") : null,
};
