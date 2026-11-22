import { describe, expect, it } from "vitest";
import { fillField, marksOfRuns, readField, sameMarks } from "./richText";

const field = () => document.createElement("div");

describe("the paragraph field's rich text", () => {
  it("fills and reads back the same text and marks, line breaks included", () => {
    const value = {
      text: "The rent is due\non the first day",
      marks: [
        { start: 12, end: 15, bold: true, italic: false },
        { start: 23, end: 28, bold: true, italic: true },
      ],
    };
    const element = field();
    fillField(element, value);
    expect(element.innerHTML).toBe("The rent is <b>due</b><br>on the <b><i>first</i></b> day");
    expect(readField(element)).toEqual(value);
  });

  it("reads Arabic by UTF-16 units, and plain text has no marks", () => {
    const element = field();
    fillField(element, {
      text: "يدفع الإيجار شهريا",
      marks: [{ start: 5, end: 12, bold: true, italic: false }],
    });
    expect(element.querySelector("b")?.textContent).toBe("الإيجار");
    expect(readField(element).marks).toEqual([{ start: 5, end: 12, bold: true, italic: false }]);
    fillField(element, { text: "plain", marks: [] });
    expect(readField(element)).toEqual({ text: "plain", marks: [] });
  });

  it("follows the browser's own formatting: inline styles, strong and em, block elements", () => {
    const element = field();
    // WebKit un-bolds inside a bold element with an inline style.
    element.innerHTML =
      '<b>due <span style="font-weight: normal;">now</span></b> and <strong>this</strong>' +
      '<div><em>next</em> <span style="font-style: italic; font-weight: 700;">line</span></div>';
    expect(readField(element)).toEqual({
      text: "due now and this\nnext line",
      marks: [
        { start: 0, end: 4, bold: true, italic: false },
        { start: 12, end: 16, bold: true, italic: false },
        { start: 17, end: 21, bold: false, italic: true },
        { start: 22, end: 26, bold: true, italic: true },
      ],
    });
  });

  it("drops the line break the browser keeps after the last line, and non-breaking spaces", () => {
    const element = field();
    element.innerHTML = "one two<br><br>";
    expect(readField(element)).toEqual({ text: "one two", marks: [] });
  });

  it("compares looks, not how the marks are cut", () => {
    const text = "abcdef";
    const whole = [{ start: 1, end: 5, bold: true, italic: false }];
    const cut = [
      { start: 1, end: 3, bold: true, italic: false },
      { start: 3, end: 5, bold: true, italic: false },
    ];
    expect(sameMarks(text, whole, cut)).toBe(true);
    expect(sameMarks(text, whole, [])).toBe(false);
    // A paragraph's runs, as marks: only its bold or italic stretches.
    const runs = [
      {
        start: 0,
        end: 2,
        font: "F",
        bold: false,
        italic: false,
        size: 12,
        color: [0, 0, 0] as [number, number, number],
      },
      {
        start: 2,
        end: 4,
        font: "F-Bold",
        bold: true,
        italic: false,
        size: 12,
        color: [0, 0, 0] as [number, number, number],
      },
    ];
    expect(marksOfRuns(runs)).toEqual([{ start: 2, end: 4, bold: true, italic: false }]);
  });
});
