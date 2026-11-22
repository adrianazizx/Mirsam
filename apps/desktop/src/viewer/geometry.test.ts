import { describe, expect, it } from "vitest";
import { CSS_PX_PER_POINT } from "../state/zoom";
import { pagePixels, renderScale, TILE_SIZE, tileId, tilesInRegion } from "./geometry";

describe("render geometry", () => {
  it("computes page pixels in f32 arithmetic, like safha-core", () => {
    // 595.276 is not an f32; the engine sees Math.fround(595.276) = 595.2760009765625.
    const width = Math.fround(595.276);
    expect(pagePixels(width, Math.fround(2))).toBe(1191);
    // A product that is an exact integer in f32 must not round up to the next pixel.
    expect(pagePixels(612, 0.5)).toBe(306);
    // f64 and f32 disagree here: 0.1 * 30 is 3.0000000000000004 in f64 but 3 in f32 math.
    expect(pagePixels(30, 0.1)).toBe(Math.ceil(Math.fround(Math.fround(30) * Math.fround(0.1))));
  });

  it("derives a stable, f32-exact scale from zoom and device pixel ratio", () => {
    const scale = renderScale(1, 2);
    expect(scale).toBe(Math.fround(scale));
    expect(scale).toBeCloseTo(2 * CSS_PX_PER_POINT, 3);
    // The same zoom always gives the same cache key.
    expect(renderScale(0.734_512, 2)).toBe(renderScale(0.734_512, 2));
    expect(renderScale(0.734_512, 2)).toBe(Math.fround(1.959));
  });

  it("covers a region with grid tiles clamped to the page", () => {
    const tiles = tilesInRegion(1200, 700, { x0: 100, y0: 600, x1: 1300, y1: 900 }, 2);
    // Columns 0..2 (the last 176 px wide), rows 1 only (512..700).
    expect(tiles.map(({ x, y, width, height }) => [x, y, width, height])).toEqual([
      [0, 512, 512, 188],
      [512, 512, 512, 188],
      [1024, 512, 176, 188],
    ]);
    expect(tiles.every((tile) => tile.scale === 2)).toBe(true);
  });

  it("returns nothing for a region outside the page and exact tiles on grid lines", () => {
    expect(tilesInRegion(800, 800, { x0: 900, y0: 0, x1: 1000, y1: 100 }, 1)).toEqual([]);
    const exact = tilesInRegion(1024, 1024, { x0: 0, y0: 0, x1: TILE_SIZE, y1: TILE_SIZE }, 1);
    expect(exact).toHaveLength(1);
  });

  it("identifies tiles by page and rectangle", () => {
    const tile = { scale: 2, x: 512, y: 0, width: 512, height: 512 };
    expect(tileId(3, tile)).toBe(tileId(3, { ...tile }));
    expect(tileId(3, tile)).not.toBe(tileId(4, tile));
    expect(tileId(3, tile)).not.toBe(tileId(3, { ...tile, scale: 3 }));
  });
});
