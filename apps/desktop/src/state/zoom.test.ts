import { describe, expect, it } from "vitest";
import { CSS_PX_PER_POINT, fitWidthZoom, MAX_BITMAP_SIDE, renderScale, stepZoom } from "./zoom";

describe("zoom", () => {
  it("steps through presets in both directions and stops at the limits", () => {
    expect(stepZoom(1, 1)).toBe(1.1);
    expect(stepZoom(1, -1)).toBe(0.9);
    expect(stepZoom(0.8, 1)).toBe(0.9);
    expect(stepZoom(8, 1)).toBe(8);
    expect(stepZoom(0.1, -1)).toBe(0.1);
  });

  it("fits a page to the available width", () => {
    // A 600pt page is 800 CSS px at 100%.
    expect(fitWidthZoom(600, 800)).toBeCloseTo(1);
    expect(fitWidthZoom(600, 400)).toBeCloseTo(0.5);
    expect(fitWidthZoom(0, 400)).toBe(1);
  });

  it("renders at device resolution, capped to the engine's bitmap limit", () => {
    const a4 = { width: 595, height: 842 };
    expect(renderScale(1, 2, a4)).toBeCloseTo(2 * CSS_PX_PER_POINT);
    const capped = renderScale(8, 2, a4);
    expect(Math.ceil(a4.height * capped)).toBeLessThanOrEqual(MAX_BITMAP_SIDE);
    expect(capped).toBeLessThan(8 * 2 * CSS_PX_PER_POINT);
  });
});
