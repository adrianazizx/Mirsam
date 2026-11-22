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
}

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
export type Urgency = "placeholder" | "visible" | "prefetch";

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

export interface RenderedPage {
  width: number;
  height: number;
  /** Time the engine spent rasterizing, in microseconds. */
  renderMicros: number;
  /** Served from the engine's tile cache rather than rendered for this request. */
  fromCache: boolean;
  /** Straight RGBA, row-major, exactly width * height * 4 bytes. */
  rgba: Uint8ClampedArray<ArrayBuffer>;
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
  openDocumentDialog(): Promise<DocumentInfo | null>;
  /** The document the app was launched with, on the first call only. */
  takeStartupDocument(): Promise<DocumentInfo | null>;
  /** Every page's size, read without loading pages. */
  pageSizes(documentId: number): Promise<PageSize[]>;
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
  ): Promise<RenderedPage>;
  /** Advances the viewport generation; queued renders not in `keep` are cancelled. */
  updateViewport(documentId: number, generation: number, keep: KeptRequest[]): Promise<void>;
  closeDocument(documentId: number): Promise<void>;
  selftestConfig(): Promise<SelftestConfig | null>;
  selftestReport(results: unknown, canvasPng: string | null): Promise<void>;
  /** Devtools builds only; null elsewhere. */
  renderStats(): Promise<RenderStats | null>;
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
