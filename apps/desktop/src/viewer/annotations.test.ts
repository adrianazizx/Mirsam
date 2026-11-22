import { describe, expect, it } from "vitest";
import type { PageAnnotation } from "../backend";
import {
  annotationAt,
  distanceToSegment,
  erased,
  inkRuns,
  inkWidth,
  nearStroke,
} from "./annotations";

const annotation = (patch: Partial<PageAnnotation>): PageAnnotation => ({
  index: 0,
  kind: "highlight",
  rects: [],
  contents: null,
  strokes: [],
  ...patch,
});

describe("annotation geometry", () => {
  it("measures the distance to a segment, ends included", () => {
    expect(distanceToSegment({ x: 5, y: 3 }, [0, 0], [10, 0])).toBe(3);
    expect(distanceToSegment({ x: 13, y: 4 }, [0, 0], [10, 0])).toBe(5);
    expect(distanceToSegment({ x: 2, y: 2 }, [1, 1], [1, 1])).toBeCloseTo(Math.SQRT2);
  });

  it("hits a stroke near its line, not across its box", () => {
    // A diagonal: its box covers (100, 0), the stroke does not.
    const stroke: [number, number][] = [
      [0, 0],
      [100, 100],
    ];
    expect(nearStroke({ x: 52, y: 50 }, stroke, 5)).toBe(true);
    expect(nearStroke({ x: 100, y: 0 }, stroke, 5)).toBe(false);
    expect(nearStroke({ x: 1, y: 1 }, [[0, 0]], 5)).toBe(true);
  });

  it("finds the annotation on top, and what an eraser touches", () => {
    const list = [
      annotation({ index: 0, kind: "highlight", rects: [{ x: 0, y: 0, width: 50, height: 10 }] }),
      annotation({ index: 2, kind: "note", rects: [{ x: 40, y: 0, width: 20, height: 20 }] }),
      annotation({
        index: 3,
        kind: "ink",
        rects: [{ x: 0, y: 0, width: 100, height: 100 }],
        strokes: [
          [
            [0, 100],
            [100, 100],
          ],
        ],
      }),
    ];
    expect(annotationAt(list, { x: 45, y: 5 })?.index).toBe(2);
    expect(annotationAt(list, { x: 10, y: 5 })?.index).toBe(0);
    expect(annotationAt(list, { x: 50, y: 98 })?.index).toBe(3);
    // Inside the ink's box but far from its stroke: nothing.
    expect(annotationAt(list, { x: 50, y: 50 })).toBeNull();
    expect(erased(list, { x: 50, y: 101 })).toEqual([3]);
    expect(erased(list, { x: 45, y: 5 })).toEqual([]);
  });

  it("draws a stroke in runs of one width, as the engine does", () => {
    expect(inkWidth(0.5)).toBe(2);
    expect(inkWidth(1)).toBe(3);
    expect(inkWidth(0)).toBe(1);
    const runs = inkRuns([
      { x: 0, y: 0, pressure: 0.5 },
      { x: 10, y: 0, pressure: 0.5 },
      { x: 20, y: 0, pressure: 0.5 },
      { x: 30, y: 0, pressure: 1 },
    ]);
    expect(runs).toEqual([
      { d: "M0 0L10 0L20 0", width: 2 },
      { d: "M20 0L30 0", width: 2.5 },
    ]);
    expect(inkRuns([{ x: 5, y: 5, pressure: 0.5 }])).toEqual([{ d: "M5 5L5 5", width: 2 }]);
  });
});
