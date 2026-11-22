import { describe, expect, it } from "vitest";
import { fitWidthZoom, stepZoom } from "./zoom";

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
});
