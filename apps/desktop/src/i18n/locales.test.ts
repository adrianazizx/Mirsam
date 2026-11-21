import { describe, expect, it } from "vitest";
import { directionOf, formatNumber } from "./index";
import ar from "./locales/ar.json";
import en from "./locales/en.json";

/** Flattens a locale tree into { "a.b.c": "text" }. */
function flatten(node: unknown, prefix = "", out: Record<string, string> = {}) {
  if (typeof node === "string") {
    out[prefix] = node;
  } else if (typeof node === "object" && node !== null) {
    for (const [key, child] of Object.entries(node)) {
      flatten(child, prefix ? `${prefix}.${key}` : key, out);
    }
  }
  return out;
}

function placeholdersOf(text: string): string[] {
  return [...text.matchAll(/{{\s*(\w+)\s*}}/g)].map((match) => match[1] ?? "").sort();
}

const english = flatten(en);
const arabic = flatten(ar);

describe("locale files", () => {
  it("define exactly the same keys in English and Arabic", () => {
    expect(Object.keys(arabic).sort()).toEqual(Object.keys(english).sort());
  });

  it("use the same placeholders in every translated string", () => {
    for (const [key, text] of Object.entries(english)) {
      expect(placeholdersOf(arabic[key] ?? ""), key).toEqual(placeholdersOf(text));
    }
  });

  it("leave no string empty", () => {
    const empty = [...Object.entries(english), ...Object.entries(arabic)]
      .filter(([, text]) => text.trim() === "")
      .map(([key]) => key);
    expect(empty).toEqual([]);
  });
});

describe("formatting", () => {
  it("maps locales to directions", () => {
    expect(directionOf("en")).toBe("ltr");
    expect(directionOf("ar")).toBe("rtl");
  });

  it("uses Western digits in the Arabic UI chrome", () => {
    expect(formatNumber(1234, "ar")).toMatch(/^1.?234$/);
  });
});
