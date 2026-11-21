/** Contract between the UI and the Rust shell. Mirrors apps/desktop/src-tauri/src/commands.rs. */

export interface AppInfo {
  version: string;
  devtools: boolean;
}

export interface DocumentInfo {
  id: number;
  /** File name only; the UI never sees filesystem paths. */
  name: string;
  pageCount: number;
}

/** Page dimensions in PDF points (1/72 inch). */
export interface PageSize {
  width: number;
  height: number;
}

export interface RenderedPage {
  width: number;
  height: number;
  /** Time the engine spent rasterizing, in microseconds. */
  renderMicros: number;
  /** Straight RGBA, row-major, exactly width * height * 4 bytes. */
  rgba: Uint8ClampedArray<ArrayBuffer>;
}

export interface SelftestConfig {
  locale: string;
  theme: string;
  /** Name of a spike benchmark to run instead of the plain self-test. */
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

export interface Backend {
  appInfo(): Promise<AppInfo>;
  /** Shows the native picker. Resolves to null when the user cancels. */
  openDocumentDialog(): Promise<DocumentInfo | null>;
  /** The document the app was launched with, on the first call only. */
  takeStartupDocument(): Promise<DocumentInfo | null>;
  pageSize(documentId: number, page: number): Promise<PageSize>;
  /** `scale` is device pixels per PDF point. */
  renderPage(documentId: number, page: number, scale: number): Promise<RenderedPage>;
  closeDocument(documentId: number): Promise<void>;
  selftestConfig(): Promise<SelftestConfig | null>;
  selftestReport(results: unknown, canvasPng: string | null): Promise<void>;
}
