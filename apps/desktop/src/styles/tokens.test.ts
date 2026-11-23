import { describe, expect, it } from "vitest";
import css from "./tokens.css?raw";

/**
 * Enforces WCAG 2.1 AA contrast on the design tokens themselves, so a palette
 * change that makes text unreadable fails CI instead of shipping.
 */

function themeBlock(selector: string): Record<string, string> {
  const start = css.indexOf(selector);
  if (start === -1) throw new Error(`no block for ${selector}`);
  const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("}", start));
  const tokens: Record<string, string> = {};
  for (const match of body.matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) {
    if (match[1] && match[2]) tokens[match[1]] = match[2];
  }
  return tokens;
}

function luminance(hex: string): number {
  const channel = (offset: number) => {
    const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrast(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (light + 0.05) / (dark + 0.05);
}

/** [foreground, background, minimum ratio]. 4.5 for text, 3 for UI graphics (WCAG 1.4.3 / 1.4.11). */
const PAIRS: [string, string, number][] = [
  ["color-text", "color-bg-app", 4.5],
  ["color-text", "color-bg-surface", 4.5],
  ["color-text", "color-bg-hover", 4.5],
  ["color-text", "color-bg-pressed", 4.5],
  ["color-text-muted", "color-bg-app", 4.5],
  ["color-text-muted", "color-bg-surface", 4.5],
  ["color-text-muted", "color-bg-hover", 4.5],
  ["color-text-on-selected", "color-bg-selected", 4.5],
  ["color-on-accent", "color-accent", 4.5],
  ["color-on-accent", "color-accent-hover", 4.5],
  ["color-on-accent", "color-accent-pressed", 4.5],
  ["color-tooltip-text", "color-tooltip-bg", 4.5],
  ["color-danger", "color-bg-surface", 4.5],
  // Signature verdicts (ADR 0048) in the signatures panel.
  ["color-success", "color-bg-surface", 4.5],
  ["color-success", "color-bg-app", 4.5],
  ["color-warning", "color-bg-surface", 4.5],
  ["color-warning", "color-bg-app", 4.5],
  ["color-search-mark-text", "color-search-mark-bg", 4.5],
  ["color-replace-new-text", "color-replace-new-bg", 4.5],
  ["color-focus-ring", "color-bg-app", 3],
  ["color-focus-ring", "color-bg-surface", 3],
  ["color-focus-ring", "color-bg-canvas", 3],
  ["color-border-strong", "color-bg-surface", 3],
  // Signatures are previewed as ink on paper in both themes (ADR 0045).
  ["color-ink", "color-page-paper", 4.5],
  // Link areas are outlined on the page in both themes (ADR 0051).
  ["color-link-outline", "color-page-paper", 3],
  // The form designer's field outlines and tab numbers, on the page (ADR 0047).
  ["color-field-outline", "color-page-paper", 3],
  ["color-field-tab-text", "color-field-tab-bg", 4.5],
];

describe.each([
  ["light", ':root[data-theme="light"]'],
  ["dark", ':root[data-theme="dark"]'],
])("%s theme tokens", (_name, selector) => {
  const tokens = themeBlock(selector);

  it.each(PAIRS)("%s on %s is at least %d:1", (foreground, background, minimum) => {
    const fg = tokens[foreground];
    const bg = tokens[background];
    expect(fg, foreground).toBeDefined();
    expect(bg, background).toBeDefined();
    expect(contrast(fg as string, bg as string)).toBeGreaterThanOrEqual(minimum);
  });
});

describe("token hygiene", () => {
  it("defines the same colour tokens in both themes", () => {
    const light = Object.keys(themeBlock(':root[data-theme="light"]')).sort();
    const dark = Object.keys(themeBlock(':root[data-theme="dark"]')).sort();
    expect(dark).toEqual(light);
  });

  it("never encodes a physical direction", () => {
    const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(withoutComments).not.toMatch(/\b(left|right)\b/);
  });
});
