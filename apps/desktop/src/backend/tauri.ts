import { invoke } from "@tauri-apps/api/core";
import { decodeBitmap } from "./bitmap";
import type {
  AppInfo,
  Backend,
  DocumentInfo,
  PageSize,
  RenderStats,
  SelftestConfig,
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
  openDocumentDialog: () => invoke<DocumentInfo | null>("open_document_dialog"),
  takeStartupDocument: () => invoke<DocumentInfo | null>("take_startup_document"),
  pageSizes: (documentId) => invoke<PageSize[]>("page_sizes", { documentId }),
  renderTile: async (documentId, generation, page, tile, priority) =>
    decodeBitmap(
      await invoke<ArrayBuffer>("render_tile", { documentId, generation, page, tile, priority }),
    ),
  updateViewport: (documentId, generation, keep) =>
    invoke<void>("update_viewport", { documentId, generation, keep }),
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
