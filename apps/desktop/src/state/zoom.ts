/** CSS pixels per PDF point at 100% zoom (96 dpi / 72 pt per inch). */
export const CSS_PX_PER_POINT = 96 / 72;

export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 8;

/** Largest bitmap side the engine will rasterize in one request (safha-core MAX_BITMAP_SIDE). */
export const MAX_BITMAP_SIDE = 8192;

const ZOOM_STOPS = [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 3, 4, 6, 8];

export function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));
}

/** The next preset above (`direction` 1) or below (-1) the current zoom. */
export function stepZoom(zoom: number, direction: 1 | -1): number {
  const epsilon = 0.001;
  if (direction === 1) {
    return ZOOM_STOPS.find((stop) => stop > zoom + epsilon) ?? MAX_ZOOM;
  }
  return ZOOM_STOPS.findLast((stop) => stop < zoom - epsilon) ?? MIN_ZOOM;
}

/** Zoom at which a page `pageWidthPt` points wide exactly fills `availableCssPx`. */
export function fitWidthZoom(pageWidthPt: number, availableCssPx: number): number {
  if (pageWidthPt <= 0 || availableCssPx <= 0) return 1;
  return clampZoom(availableCssPx / (pageWidthPt * CSS_PX_PER_POINT));
}

/**
 * Device pixels per point for rasterizing at `zoom` on a display with
 * `devicePixelRatio`, reduced if needed so neither side exceeds the engine's
 * single-bitmap limit (whole-page rendering only; tiles lift this in M1).
 */
export function renderScale(
  zoom: number,
  devicePixelRatio: number,
  pageSize: { width: number; height: number },
): number {
  const wanted = zoom * CSS_PX_PER_POINT * devicePixelRatio;
  const longest = Math.max(pageSize.width, pageSize.height);
  const limit = (MAX_BITMAP_SIDE - 1) / longest;
  return Math.min(wanted, limit);
}
