import { describe, expect, it } from "vitest";
import type { TextLayer } from "../backend";
import { fillReadableText, languageOf, scriptRuns } from "./readableText";

describe("readable page text", () => {
  it("splits mixed text into runs of one script, spaces between scripts in neither", () => {
    expect(scriptRuns("رقم العقد: SA-2026-00417 (نسخة)")).toEqual([
      { text: "رقم العقد:", lang: "ar" },
      { text: " ", lang: null },
      { text: "SA-2026-00417", lang: "en" },
      { text: " ", lang: null },
      { text: "(نسخة)", lang: "ar" },
    ]);
  });

  it("keeps digits and spaces inside a run of one script", () => {
    expect(scriptRuns("12 مارس 2026")).toEqual([
      { text: "12 ", lang: null },
      { text: "مارس", lang: "ar" },
      { text: " 2026", lang: null },
    ]);
    expect(scriptRuns("pay SAR 4,500 per month")).toEqual([
      { text: "pay SAR 4,500 per month", lang: "en" },
    ]);
  });

  it("names the language most letters are in", () => {
    expect(languageOf("عقد إيجار — Lease")).toBe("ar");
    expect(languageOf("Lease Agreement — عقد")).toBe("en");
    expect(languageOf("١٢٣ — 456")).toBeNull();
  });

  it("writes one paragraph per block, with its direction and its other-script runs marked", () => {
    const host = document.createElement("div");
    const layer: TextLayer = {
      direction: "rtl",
      quality: "good",
      blocks: [
        {
          direction: "rtl",
          lines: [
            { text: "اسم المنتج", spans: [], bounds: { x: 0, y: 0, width: 1, height: 1 } },
            { text: "Mirsam Pro", spans: [], bounds: { x: 0, y: 0, width: 1, height: 1 } },
          ],
        },
      ],
    };
    fillReadableText(host, layer, () => "unused");
    expect(host.innerHTML).toBe(
      '<p dir="rtl" lang="ar">اسم المنتج <span lang="en">Mirsam Pro</span></p>',
    );
  });

  it("says when a page has no text, or text that may read wrongly", () => {
    const host = document.createElement("div");
    const note = (kind: string) => `note:${kind}`;
    fillReadableText(host, { direction: "ltr", quality: "noText", blocks: [] }, note);
    expect(host.textContent).toBe("note:noText");
    fillReadableText(
      host,
      {
        direction: "ltr",
        quality: "suspect",
        blocks: [
          {
            direction: "ltr",
            lines: [{ text: "Hcllo", spans: [], bounds: { x: 0, y: 0, width: 1, height: 1 } }],
          },
        ],
      },
      note,
    );
    expect([...host.children].map((p) => p.textContent)).toEqual(["note:suspect", "Hcllo"]);
  });
});
