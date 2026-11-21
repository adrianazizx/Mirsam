import { invoke } from "@tauri-apps/api/core";
import { decodeBitmap } from "./bitmap";
import type { AppInfo, Backend, DocumentInfo, PageSize, SelftestConfig } from "./types";

/** The real backend: typed wrappers over the Rust shell's commands. */
export const tauriBackend: Backend = {
  appInfo: () => invoke<AppInfo>("app_info"),
  openDocumentDialog: () => invoke<DocumentInfo | null>("open_document_dialog"),
  takeStartupDocument: () => invoke<DocumentInfo | null>("take_startup_document"),
  pageSize: (documentId, page) => invoke<PageSize>("page_size", { documentId, page }),
  renderPage: async (documentId, page, scale) =>
    decodeBitmap(await invoke<ArrayBuffer>("render_page", { documentId, page, scale })),
  closeDocument: async (documentId) => {
    await invoke<boolean>("close_document", { documentId });
  },
  // The self-test commands exist only in `devtools` builds of the shell.
  selftestConfig: async () => {
    const { devtools } = await invoke<AppInfo>("app_info");
    return devtools ? invoke<SelftestConfig | null>("selftest_config") : null;
  },
  selftestReport: (results, canvasPng) =>
    invoke<void>("selftest_report", { report: { results, canvasPng } }),
};
