import { describe, expect, it } from "vitest";
import type { PageSize } from "../backend";
import { CSS_PX_PER_POINT } from "../state/zoom";
import { renderScale } from "./geometry";
import {
  anchorAt,
  computeLayout,
  currentPageIn,
  fitWidthZoom,
  groupRows,
  type LayoutParams,
  resolveAnchor,
  rowsInBand,
} from "./layout";

// 300 x 400 pt pages at zoom 0.75 and dpr 1 are exactly 300 x 400 CSS px.
const PAGE: PageSize = { width: 300, height: 400 };
const WIDE: PageSize = { width: 600, height: 300 };

function params(overrides: Partial<LayoutParams> = {}): LayoutParams {
  return {
    sizes: [PAGE, PAGE, PAGE, PAGE, PAGE],
    scale: renderScale(0.75, 1),
    devicePixelRatio: 1,
    mode: "continuous",
    order: "ltr",
    currentPage: 0,
    rowGap: 10,
    padding: 20,
    spreadGap: 4,
    viewportWidth: 1000,
    ...overrides,
  };
}

describe("row grouping", () => {
  it("groups pages per mode", () => {
    expect(groupRows(5, "continuous", 0)).toEqual([[0], [1], [2], [3], [4]]);
    expect(groupRows(5, "single", 3)).toEqual([[3]]);
    expect(groupRows(5, "spread", 0)).toEqual([[0, 1], [2, 3], [4]]);
    expect(groupRows(5, "book", 0)).toEqual([[0], [1, 2], [3, 4]]);
    expect(groupRows(0, "book", 0)).toEqual([]);
  });
});

describe("layout", () => {
  it("stacks continuous pages, centred, with gaps and padding", () => {
    const layout = computeLayout(params());
    expect(layout.boxes[0]).toEqual({ page: 0, x: 350, y: 20, width: 300, height: 400 });
    expect(layout.boxes[1]?.y).toBe(20 + 400 + 10);
    expect(layout.height).toBe(20 + 5 * 400 + 4 * 10 + 20);
    expect(layout.width).toBe(1000);
  });

  it("grows wider than the viewport when zoomed in, so it scrolls sideways", () => {
    const layout = computeLayout(params({ viewportWidth: 200 }));
    expect(layout.width).toBe(300 + 2 * 20);
    expect(layout.boxes[0]?.x).toBe(20);
  });

  it("puts the earlier page of a spread on the left in LTR and on the right in RTL", () => {
    const ltr = computeLayout(params({ mode: "spread" }));
    const rtl = computeLayout(params({ mode: "spread", order: "rtl" }));
    expect((ltr.boxes[0]?.x ?? 0) < (ltr.boxes[1]?.x ?? 0)).toBe(true);
    expect((rtl.boxes[0]?.x ?? 0) > (rtl.boxes[1]?.x ?? 0)).toBe(true);
    // The pair is centred: 300 + 4 + 300 in a 1000 px viewport.
    expect(ltr.boxes[0]?.x).toBe(198);
    expect(rtl.boxes[1]?.x).toBe(198);
    expect(ltr.boxes[1]?.y).toBe(ltr.boxes[0]?.y);
  });

  it("places a book's cover where a recto goes, and a trailing page where a verso goes", () => {
    const ltr = computeLayout(params({ mode: "book", sizes: [PAGE, PAGE, PAGE, PAGE] }));
    const rtl = computeLayout(
      params({ mode: "book", order: "rtl", sizes: [PAGE, PAGE, PAGE, PAGE] }),
    );
    const spine = 500;
    // Cover: right of the spine in LTR, left of it in RTL.
    expect(ltr.boxes[0]?.x).toBe(spine + 2);
    expect(rtl.boxes[0]?.x).toBe(spine - 2 - 300);
    // Page 4 (index 3) is alone on the last row: a verso, left in LTR, right in RTL.
    expect(ltr.boxes[3]?.x).toBe(spine - 2 - 300);
    expect(rtl.boxes[3]?.x).toBe(spine + 2);
  });

  it("lays out only the current page in single mode", () => {
    const layout = computeLayout(params({ mode: "single", currentPage: 3 }));
    expect(layout.rows).toHaveLength(1);
    expect(layout.boxes[3]?.y).toBe(20);
    expect(layout.boxes[0]).toBeUndefined();
    expect(layout.rowOf[3]).toBe(0);
    expect(layout.rowOf[0]).toBe(-1);
  });

  it("centres pages of different heights within their spread row", () => {
    const layout = computeLayout(params({ mode: "spread", sizes: [PAGE, WIDE] }));
    expect(layout.rows[0]?.bottom).toBe(20 + 400);
    expect(layout.boxes[1]?.y).toBe(20 + (400 - 300) / 2);
  });

  it("aligns boxes to device pixels on HiDPI displays", () => {
    const layout = computeLayout(
      params({ devicePixelRatio: 2, scale: renderScale(0.7531, 2), viewportWidth: 999 }),
    );
    for (const box of layout.boxes) {
      expect(((box?.x ?? 0) * 2) % 1).toBe(0);
      expect(((box?.y ?? 0) * 2) % 1).toBe(0);
    }
  });
});

describe("fit width", () => {
  it("fits the typical row, not an occasional wide page", () => {
    // 300 pt = 400 CSS px at 100%; 1000 - 2 * 50 - 1 px of slack leaves 899.
    expect(fitWidthZoom([PAGE, WIDE, PAGE], "continuous", 1000, 50, 4)).toBeCloseTo(
      899 / (300 * CSS_PX_PER_POINT),
    );
    expect(fitWidthZoom([WIDE, PAGE, WIDE], "continuous", 1000, 50, 4)).toBeCloseTo(
      899 / (600 * CSS_PX_PER_POINT),
    );
    // Two 300 pt pages side by side, plus the spread gap.
    expect(fitWidthZoom([PAGE, PAGE], "spread", 1000, 50, 4)).toBeCloseTo(
      (899 - 4) / (600 * CSS_PX_PER_POINT),
    );
    expect(fitWidthZoom([], "continuous", 1000, 50, 4)).toBe(1);
  });
});

describe("queries", () => {
  const layout = computeLayout(params());

  it("finds the rows in a vertical band", () => {
    expect(rowsInBand(layout.rows, 0, 19)).toEqual([]);
    expect(rowsInBand(layout.rows, 0, 21)).toEqual([0]);
    // 425 is in the gap between rows 0 and 1; 900 is inside row 2.
    expect(rowsInBand(layout.rows, 425, 900)).toEqual([1, 2]);
    expect(rowsInBand(layout.rows, 99_999, 100_000)).toEqual([]);
  });

  it("calls the most visible page current, the earlier one on a tie", () => {
    expect(currentPageIn(layout, { x0: 0, y0: 0, x1: 1000, y1: 500 })).toBe(0);
    expect(currentPageIn(layout, { x0: 0, y0: 300, x1: 1000, y1: 800 })).toBe(1);
    const spread = computeLayout(params({ mode: "spread", order: "rtl" }));
    // Both pages of the first spread are fully visible: page 0 (on the right) wins.
    expect(currentPageIn(spread, { x0: 0, y0: 0, x1: 1000, y1: 500 })).toBe(0);
  });

  it("keeps a point on the same spot of its page across a zoom", () => {
    const point = { x: 500, y: 455 + 200 };
    const anchor = anchorAt(layout, point.x, point.y);
    expect(anchor?.page).toBe(1);
    const zoomed = computeLayout(params({ scale: renderScale(1.5, 1) }));
    const moved = anchor ? resolveAnchor(zoomed, anchor) : null;
    const box = zoomed.boxes[1];
    expect(moved?.x).toBeCloseTo((box?.x ?? 0) + (box?.width ?? 0) / 2);
    expect(moved?.y).toBeCloseTo((box?.y ?? 0) + (225 / 400) * (box?.height ?? 0));
  });
});
