import { describe, expect, it } from "vitest";
import { mockBlockLayout } from "../backend/mock";
import {
  blockAt,
  caretFor,
  lineEdge,
  lineOf,
  offsetAt,
  selectionRects,
  verticalMove,
  wordAround,
} from "./blockLayout";

// 6-point advances (nine to a 54-point line): "one two three four" breaks
// after "one two " (8) and "three " (14).
const text = "one two three four";
const ltr = mockBlockLayout(text, 54, 12, false, 0);
const rtl = mockBlockLayout("واحد اثنان ثلاثة", 60, 12, true, 0);

describe("block layout geometry", () => {
  it("puts a soft break's offset at the start of the next line", () => {
    expect(ltr.lines.map((line) => [line.start, line.end])).toEqual([
      [0, 8],
      [8, 14],
      [14, 18],
    ]);
    expect(lineOf(ltr, 0)).toBe(0);
    expect(lineOf(ltr, 7)).toBe(0);
    expect(lineOf(ltr, 8)).toBe(1);
    expect(lineOf(ltr, 18)).toBe(2);
    expect(caretFor(ltr, 8)).toMatchObject({ line: 1, x: 0 });
    expect(caretFor(ltr, 18)).toMatchObject({ line: 2, x: 24 });
  });

  it("finds the nearest caret stop to a point, line by line", () => {
    expect(offsetAt(ltr, 13, 5)).toBe(2);
    expect(offsetAt(ltr, 1, 16)).toBe(8);
    // Below the last line: the last line.
    expect(offsetAt(ltr, 100, 999)).toBe(18);
    // Right to left: the start of the line is at its right edge.
    expect(offsetAt(rtl, 60, 5)).toBe(0);
    expect(offsetAt(rtl, 55, 5)).toBe(1);
  });

  it("moves up and down by the engine's lines, keeping the x it started at", () => {
    // From "t|wo" (x 30) down to "thr|ee"'s nearest stop.
    const down = verticalMove(ltr, 5, 1, 30);
    expect(down).toBe(13);
    expect(verticalMove(ltr, down, 1, 30)).toBe(18);
    expect(verticalMove(ltr, 2, -1, 12)).toBe(0);
    expect(verticalMove(ltr, 18, 1, 24)).toBe(18);
  });

  it("finds a line's start and end without its trailing space", () => {
    expect(lineEdge(ltr, 10, "start")).toBe(8);
    expect(lineEdge(ltr, 10, "end")).toBe(13);
    expect(lineEdge(ltr, 15, "end")).toBe(18);
  });

  it("covers a selection with one rectangle per cluster, mirrored for right to left", () => {
    const rounded = (rects: ReturnType<typeof selectionRects>) =>
      rects.map((rect) => [rect.x, rect.y, rect.width, Math.round(rect.height * 10) / 10]);
    expect(rounded(selectionRects(ltr, 0, 3))).toEqual([
      [0, 0, 6, 14.4],
      [6, 0, 6, 14.4],
      [12, 0, 6, 14.4],
    ]);
    expect(selectionRects(ltr, 4, 4)).toEqual([]);
    expect(rounded(selectionRects(rtl, 0, 1))).toEqual([[54, 0, 6, 14.4]]);
  });

  it("selects the word under a double click, in either script", () => {
    expect(wordAround(text, 5)).toEqual([4, 7]);
    expect(wordAround("واحد اثنان", 7)).toEqual([5, 10]);
  });

  it("finds the smallest block under a press, with a little slop", () => {
    const blocks = [
      { rect: { x: 0, y: 0, width: 300, height: 300 }, refusal: null },
      { rect: { x: 10, y: 10, width: 50, height: 20 }, refusal: null },
    ];
    expect(blockAt(blocks, 20, 20)).toBe(1);
    expect(blockAt(blocks, 200, 200)).toBe(0);
    expect(blockAt(blocks, 61, 31)).toBe(1);
    expect(blockAt(blocks, 400, 400)).toBe(-1);
  });
});
