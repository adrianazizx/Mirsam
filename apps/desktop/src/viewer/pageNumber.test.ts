import { describe, expect, it } from "vitest";
import { parsePageNumber } from "./pageNumber";

describe("page number entry", () => {
  it("accepts Western, Arabic-Indic, and Extended Arabic-Indic digits", () => {
    expect(parsePageNumber("12")).toBe(12);
    expect(parsePageNumber("١٢")).toBe(12);
    expect(parsePageNumber("۱۲")).toBe(12);
    expect(parsePageNumber(" 7 ")).toBe(7);
  });

  it("rejects anything that is not a positive whole number", () => {
    for (const text of ["", " ", "0", "-3", "3.5", "12a", "1 2", "١٢x"]) {
      expect(parsePageNumber(text), text).toBeNull();
    }
  });
});
