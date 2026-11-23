import type { PageSize, TileRequest } from "../backend";
import { CSS_PX_PER_POINT } from "../state/zoom";

/** Side of a full-resolution tile in device pixels (ADR 0004: 512 px renders in < 1 ms). */
export const TILE_SIZE = 512;

/** Largest render scale (device px per point) the engine accepts for a whole page. */
const MAX_SCALE = 64;

/**
 * Device pixels per point for `zoom` on a display with `devicePixelRatio`,
 * rounded to four significant digits and to an f32, so the same zoom always
 * yields the same cache key and the engine sees exactly the number we used.
 */
export function renderScale(zoom: number, devicePixelRatio: number): number {
  const exact = zoom * CSS_PX_PER_POINT * devicePixelRatio;
  return Math.fround(Math.min(MAX_SCALE, Number(exact.toPrecision(4))));
}

/**
 * One side of a page in device pixels at `scale`, computed in f32 exactly as
 * mirsam-core does (`(side * scale).ceil()` on f32 values), so edge tiles never
 * overhang the engine's idea of the page.
 */
export function pagePixels(sidePt: number, scale: number): number {
  return Math.ceil(Math.fround(Math.fround(sidePt) * Math.fround(scale)));
}

export function pagePixelSize(size: PageSize, scale: number): { width: number; height: number } {
  return { width: pagePixels(size.width, scale), height: pagePixels(size.height, scale) };
}

/** An axis-aligned rectangle; `x1`/`y1` are exclusive. */
export interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export function intersect(a: Rect, b: Rect): Rect | null {
  const r = {
    x0: Math.max(a.x0, b.x0),
    y0: Math.max(a.y0, b.y0),
    x1: Math.min(a.x1, b.x1),
    y1: Math.min(a.y1, b.y1),
  };
  return r.x1 > r.x0 && r.y1 > r.y0 ? r : null;
}

/**
 * The tiles of a `pageWidth x pageHeight` pixel page that intersect `region`
 * (in the same page-pixel space), on a fixed grid of `TILE_SIZE` with edge
 * tiles clamped to the page.
 */
export function tilesInRegion(
  pageWidth: number,
  pageHeight: number,
  region: Rect,
  scale: number,
): TileRequest[] {
  const clipped = intersect(region, { x0: 0, y0: 0, x1: pageWidth, y1: pageHeight });
  if (!clipped) return [];
  const tiles: TileRequest[] = [];
  const firstColumn = Math.floor(clipped.x0 / TILE_SIZE);
  const lastColumn = Math.ceil(clipped.x1 / TILE_SIZE) - 1;
  const firstRow = Math.floor(clipped.y0 / TILE_SIZE);
  const lastRow = Math.ceil(clipped.y1 / TILE_SIZE) - 1;
  for (let row = firstRow; row <= lastRow; row += 1) {
    for (let column = firstColumn; column <= lastColumn; column += 1) {
      const x = column * TILE_SIZE;
      const y = row * TILE_SIZE;
      tiles.push({
        scale,
        x,
        y,
        width: Math.min(TILE_SIZE, pageWidth - x),
        height: Math.min(TILE_SIZE, pageHeight - y),
      });
    }
  }
  return tiles;
}

/** Stable identity of a tile request of one page (the engine's cache key, minus the document). */
export function tileId(page: number, tile: TileRequest): string {
  return `${page}:${tile.scale}:${tile.x}:${tile.y}:${tile.width}:${tile.height}`;
}
