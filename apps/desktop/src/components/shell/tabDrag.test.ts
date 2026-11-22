import { describe, expect, it } from "vitest";
import { dropIndex } from "./tabDrag";

describe("tab drag", () => {
  // Three tabs 100 px wide: centres at 50, 150, 250.
  const ltr = [50, 150, 250];
  it("drops in reading order left to right", () => {
    expect(dropIndex(ltr, 0, 200, "ltr")).toBe(1); // first tab past the second
    expect(dropIndex(ltr, 0, 300, "ltr")).toBe(2);
    expect(dropIndex(ltr, 2, 10, "ltr")).toBe(0);
  });
  it("drops in reading order right to left", () => {
    // In an Arabic UI the first tab is the rightmost.
    const rtl = [250, 150, 50];
    expect(dropIndex(rtl, 0, 100, "rtl")).toBe(1); // first tab dragged left past the second
    expect(dropIndex(rtl, 0, 0, "rtl")).toBe(2);
    expect(dropIndex(rtl, 2, 290, "rtl")).toBe(0);
  });
});
