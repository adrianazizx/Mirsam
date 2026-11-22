import { describe, expect, it } from "vitest";
import {
  addMark,
  IDLE_REDACTION,
  isMarked,
  markedPages,
  marksForApply,
  redoMark,
  removeMark,
  undoMark,
} from "./redaction";

const box = (x: number, y = 10) => ({ x, y, width: 40, height: 12 });

describe("redaction marks", () => {
  it("adds marks, numbering them, and ignores the same boxes twice", () => {
    const first = addMark(IDLE_REDACTION, { page: 0, rects: [box(10)], text: "a", source: "area" });
    expect(first.added).toBe(true);
    const again = addMark(first.state, { page: 0, rects: [box(10)], text: "a", source: "search" });
    expect(again.added).toBe(false);
    expect(again.state.marks).toHaveLength(1);
    const other = addMark(first.state, { page: 1, rects: [box(10)], text: null, source: "area" });
    expect(other.state.marks.map((m) => m.id)).toEqual([1, 2]);
  });

  it("drops empty boxes and refuses a mark that has none", () => {
    const empty = { x: 0, y: 0, width: 0, height: 5 };
    expect(
      addMark(IDLE_REDACTION, { page: 0, rects: [empty], text: null, source: "area" }).added,
    ).toBe(false);
    const kept = addMark(IDLE_REDACTION, {
      page: 0,
      rects: [empty, box(5)],
      text: null,
      source: "area",
    });
    expect(kept.state.marks[0]?.rects).toEqual([box(5)]);
  });

  it("undoes and redoes marks in order, and a new mark ends the redo list", () => {
    let state = IDLE_REDACTION;
    for (const x of [10, 60, 110]) {
      state = addMark(state, { page: 0, rects: [box(x)], text: null, source: "area" }).state;
    }
    state = undoMark(undoMark(state));
    expect(state.marks.map((m) => m.rects[0]?.x)).toEqual([10]);
    state = redoMark(state);
    expect(state.marks.map((m) => m.rects[0]?.x)).toEqual([10, 60]);
    state = addMark(state, { page: 0, rects: [box(200)], text: null, source: "area" }).state;
    expect(state.undone).toEqual([]);
    expect(redoMark(state)).toBe(state);
    expect(undoMark(IDLE_REDACTION)).toBe(IDLE_REDACTION);
  });

  it("removes a mark by id and says which hits are marked", () => {
    const hit = { page: 2, text: "x", before: "", after: "", rects: [box(5)] };
    let state = addMark(IDLE_REDACTION, {
      page: 2,
      rects: hit.rects,
      text: "x",
      source: "search",
    }).state;
    expect(isMarked(state, hit)).toBe(true);
    expect(isMarked(state, { ...hit, page: 3 })).toBe(false);
    state = removeMark(state, 1);
    expect(state.marks).toEqual([]);
    expect(isMarked(state, hit)).toBe(false);
  });

  it("hands the backend page, boxes, and text, and counts pages", () => {
    let state = addMark(IDLE_REDACTION, {
      page: 0,
      rects: [box(1)],
      text: "t",
      source: "search",
    }).state;
    state = addMark(state, { page: 0, rects: [box(100)], text: null, source: "area" }).state;
    state = addMark(state, { page: 4, rects: [box(1)], text: null, source: "area" }).state;
    expect(markedPages(state)).toBe(2);
    expect(marksForApply(state)[0]).toEqual({ page: 0, rects: [box(1)], text: "t" });
  });
});
