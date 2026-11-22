import { invoke } from "@tauri-apps/api/core";
import { decodeBitmap } from "./bitmap";
import type {
  AppInfo,
  Backend,
  OpenOutcome,
  OutlineItem,
  PageSize,
  RecentFile,
  RenderStats,
  SearchResults,
  SelftestConfig,
  Startup,
  TextLayer,
} from "./types";

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
  renderTile: async (documentId, generation, page, tile, priority, lane = "main") =>
    decodeBitmap(
      await invoke<ArrayBuffer>("render_tile", {
        documentId,
        generation,
        page,
        tile,
        priority,
        lane,
      }),
    ),
  updateViewport: (documentId, generation, keep, lane = "main") =>
    invoke<void>("update_viewport", { documentId, generation, keep, lane }),
  closeDocument: async (documentId) => {
    await invoke<boolean>("close_document", { documentId });
  },
  // The commands below exist only in `devtools` builds of the shell.
  selftestConfig: async () =>
    (await hasDevtools()) ? invoke<SelftestConfig | null>("selftest_config") : null,
  selftestReport: (results, canvasPng) =>
    invoke<void>("selftest_report", { report: { results, canvasPng } }),
  renderStats: async () => ((await hasDevtools()) ? invoke<RenderStats>("render_stats") : null),
  shellRssKb: async () => ((await hasDevtools()) ? invoke<number | null>("spike_rss_kb") : null),
  selftestLog: async (message) => {
    if (await hasDevtools()) await invoke<void>("selftest_log", { message });
  },
  webviewRssKb: async () =>
    (await hasDevtools()) ? invoke<number | null>("webview_rss_kb") : null,
};
