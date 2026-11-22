/**
 * Colours offered for what is written into a document: new page text (ADR
 * 0021) and Fill & Sign text and marks (ADR 0045). They are the document's
 * colours (written into the PDF), not the UI's, so they are data here rather
 * than design tokens.
 */
export const DOCUMENT_COLORS = [
  { id: "black", rgb: [0, 0, 0] },
  { id: "blue", rgb: [0.05, 0.28, 0.63] },
  { id: "red", rgb: [0.72, 0.11, 0.11] },
  { id: "green", rgb: [0.1, 0.45, 0.22] },
] as const satisfies readonly { id: string; rgb: readonly [number, number, number] }[];

export type DocumentColorId = (typeof DOCUMENT_COLORS)[number]["id"];

/** A colour's RGB components (0–1), black for an unknown id. */
export function rgbOf(id: string): [number, number, number] {
  const found = DOCUMENT_COLORS.find((color) => color.id === id);
  return found ? [...found.rgb] : [0, 0, 0];
}

/** A colour as CSS, for a swatch. */
export function cssOf(rgb: readonly [number, number, number]): string {
  return `rgb(${rgb.map((c) => Math.round(c * 255)).join(" ")})`;
}
