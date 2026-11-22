import { describe, expect, it } from "vitest";
import { parsePageRanges } from "./pageRanges";

describe("page ranges (ADR 0050)", () => {
  it("reads single pages, ranges, and open ends, one-based in, zero-based out", () => {
    expect(parsePageRanges("1-3, 5, 8-", 10)).toEqual([0, 1, 2, 4, 7, 8, 9]);
    expect(parsePageRanges("-2", 10)).toEqual([0, 1]);
    expect(parsePageRanges(" 4 ", 10)).toEqual([3]);
    expect(parsePageRanges("3,1,3", 10)).toEqual([0, 2]);
  });

  it("reads what an Arabic keyboard types: Arabic-Indic digits, the Arabic comma, dashes", () => {
    expect(parsePageRanges("١–٣، ٥", 10)).toEqual([0, 1, 2, 4]);
    expect(parsePageRanges("۲-۳", 10)).toEqual([1, 2]);
  });

  it("refuses what is out of range or unreadable", () => {
    for (const text of ["", "0", "11", "5-2", "a", "1-2-3", ",", "-"]) {
      expect(parsePageRanges(text, 10), text).toBeNull();
    }
  });
});
