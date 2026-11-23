/**
 * Fired on `window` when every on-screen tile of a page has been drawn at the
 * current scale (the page is sharp). Once per page and scale. The self-test
 * waits for it.
 */
export const PAGE_PAINTED_EVENT = "mirsam:page-painted";

/** Fired on `window` when a page's low-resolution preview is drawn. Used by the benchmark. */
export const PAGE_PREVIEW_EVENT = "mirsam:page-preview";

export interface PagePreviewDetail {
  documentId: number;
  page: number;
}

export interface PagePaintedDetail {
  documentId: number;
  page: number;
  /** The whole page in device pixels at `scale`. */
  width: number;
  height: number;
  scale: number;
  /** Tiles drawn to make the page sharp. */
  tiles: number;
  /** Sum of the engine's rasterization time for those tiles. */
  renderMicros: number;
  /** First tile request for this page at this scale → last tile on screen. */
  roundTripMs: number;
  /** Viewer created (document opened) → this page sharp. */
  sinceOpenMs: number;
  /** Viewer created → this page's low-resolution preview on screen, if it had one. */
  previewSinceOpenMs: number | null;
}
