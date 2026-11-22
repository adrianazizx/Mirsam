import { describe, expect, it } from "vitest";
import type { Direction, TextLayer, TextLine } from "../backend";
import {
  caretAt,
  highlights,
  isCollapsed,
  pageSelection,
  type Selection,
  selectedText,
  wordAt,
} from "./selection";

const CHAR = 10;
const HEIGHT = 12;

/**
 * A line from its logical text and the visual slot (0 = leftmost) of each
 * character: each character is a 10 pt box, spaces have no box (inferred).
 */
function line(text: string, slots: number[], y: number): TextLine {
  const chars = Array.from(text);
  let offset = 0;
  const spans = chars.map((char, i) => {
    const start = offset;
    offset += char.length;
    const slot = slots[i] ?? 0;
    return {
      start,
      end: offset,
      rect: char === " " ? null : { x: slot * CHAR, y, width: CHAR, height: HEIGHT },
    };
  });
  const xs = slots.map((slot) => slot * CHAR);
  const x = Math.min(...xs);
  return {
    text,
    spans,
    bounds: { x, y, width: Math.max(...xs) + CHAR - x, height: HEIGHT },
  };
}

/** Visual slots for a line read left to right. */
const ltr = (text: string) => Array.from(text).map((_, i) => i);
/** Visual slots for a line read right to left, ending at slot `width - 1`. */
const rtl = (text: string, width = Array.from(text).length) =>
  Array.from(text).map((_, i) => width - 1 - i);

function layer(direction: Direction, lines: TextLine[]): TextLayer {
  return { direction, quality: "good", blocks: [{ direction, lines }] };
}

const at = (x: number) => x * CHAR + CHAR / 2;
/** Just inside a slot's left or right edge: past its centre only from the other side. */
const left = (slot: number) => slot * CHAR + 1;
const right = (slot: number) => slot * CHAR + CHAR - 1;

describe("selection", () => {
  it("copies the glyphs dragged over on a left-to-right line", () => {
    const page = layer("ltr", [line("abc DEF", ltr("abc DEF"), 0)]);
    const selection: Selection = {
      anchor: { page: 0, block: 0, line: 0, x: left(1) },
      focus: { page: 0, block: 0, line: 0, x: right(5) },
    };
    expect(selectedText(selection, () => page)).toBe("bc DE");
  });

  it("selects an Arabic line from its right edge in reading order", () => {
    const text = "سلام عليكم";
    const page = layer("rtl", [line(text, rtl(text), 0)]);
    // Dragging from the right edge across the first word.
    const selection: Selection = {
      anchor: { page: 0, block: 0, line: 0, x: 100 },
      focus: { page: 0, block: 0, line: 0, x: left(6) },
    };
    expect(selectedText(selection, () => page)).toBe("سلام");
  });

  it("extends across lines in the block's direction", () => {
    const first = "سطر أول";
    const second = "سطر ثان";
    const page = layer("rtl", [line(first, rtl(first), 0), line(second, rtl(second), 20)]);
    // From the middle of line one (its second word, to the left) to the
    // middle of line two (its first word, from the right).
    const selection: Selection = {
      anchor: { page: 0, block: 0, line: 0, x: at(3) },
      focus: { page: 0, block: 0, line: 1, x: at(3) },
    };
    expect(selectedText(selection, () => page)).toBe("أول\nسطر");
    // Dragging upwards selects the same text.
    expect(selectedText({ anchor: selection.focus, focus: selection.anchor }, () => page)).toBe(
      "أول\nسطر",
    );
  });

  it("selects visually and copies logically inside mixed-direction lines", () => {
    // Logical "رقم INV-1 هنا": displayed right to left, with the Latin run
    // left to right in the middle.
    const text = "رقم INV-1 هنا";
    // Slots (0 = leftmost): هنا at 0-2, space 3, INV-1 at 4-8, space 9, رقم at 10-12.
    const slots = [12, 11, 10, 9, 4, 5, 6, 7, 8, 3, 2, 1, 0];
    const page = layer("rtl", [line(text, slots, 0)]);
    const over = (from: number, to: number): Selection => ({
      anchor: { page: 0, block: 0, line: 0, x: from <= to ? left(from) : right(from) },
      focus: { page: 0, block: 0, line: 0, x: from <= to ? right(to) : left(to) },
    });
    expect(selectedText(over(4, 8), () => page)).toBe("INV-1");
    // From the right edge to the middle of the Latin run: the visually
    // passed glyphs, in reading order.
    expect(selectedText(over(12, 7), () => page)).toBe("رقم -1");
  });

  it("selects the word under a double-click and the whole page", () => {
    const text = "The quick fox";
    const page = layer("ltr", [line(text, ltr(text), 0), line("jumps", ltr("jumps"), 20)]);
    const word = wordAt(page, 0, at(5), 6);
    expect(word && selectedText(word, () => page)).toBe("quick");
    const all = pageSelection(page, 0);
    expect(all && selectedText(all, () => page)).toBe("The quick fox\njumps");
  });

  it("hit-tests the nearest line and reports a click as collapsed", () => {
    const page = layer("ltr", [line("one", ltr("one"), 0), line("two", ltr("two"), 30)]);
    expect(caretAt(page, 0, 5, 33)?.line).toBe(1);
    expect(caretAt(page, 0, 5, 14)?.line).toBe(0);
    const click = caretAt(page, 0, 15, 5);
    expect(click && isCollapsed({ anchor: click, focus: click }, () => page)).toBe(true);
  });

  it("merges a line's selected glyphs into runs across word spaces", () => {
    const text = "ab cd";
    const page = layer("ltr", [line(text, ltr(text), 0)]);
    const all = pageSelection(page, 0);
    const result = all ? highlights(all, () => page) : [];
    expect(result).toEqual([{ page: 0, rects: [{ x: 0, y: 0, width: 50, height: HEIGHT }] }]);
  });

  it("joins pages with a blank line", () => {
    const one = layer("ltr", [line("end", ltr("end"), 0)]);
    const two = layer("ltr", [line("next", ltr("next"), 0)]);
    const selection: Selection = {
      anchor: { page: 0, block: 0, line: 0, x: Number.NEGATIVE_INFINITY },
      focus: { page: 1, block: 0, line: 0, x: Number.POSITIVE_INFINITY },
    };
    expect(selectedText(selection, (page) => (page === 0 ? one : two))).toBe("end\n\nnext");
  });
});
