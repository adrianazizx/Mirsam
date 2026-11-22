/**
 * Page ranges as people type them: "1-3, 5, 8-" (ADR 0050). One-based in the
 * text, zero-based and ascending in the result. Arabic-Indic and Persian
 * digits, the Arabic comma (،), and en or em dashes are read too, so a range
 * typed on an Arabic keyboard works. `null` for anything unreadable or out of
 * `1..pageCount`; an open end ("8-") runs to the last page.
 */
export function parsePageRanges(text: string, pageCount: number): number[] | null {
  const western = text
    .replace(/[٠-٩]/g, (d) => String(d.charCodeAt(0) - 0x0660))
    .replace(/[۰-۹]/g, (d) => String(d.charCodeAt(0) - 0x06f0))
    .replace(/[،؛;]/g, ",")
    .replace(/[‐-―−]/g, "-");
  const parts = western
    .split(",")
    .map((part) => part.replace(/\s+/g, ""))
    .filter((part) => part !== "");
  if (parts.length === 0) return null;
  const pages = new Set<number>();
  for (const part of parts) {
    const match = /^(\d+)?(-)?(\d+)?$/.exec(part);
    if (!match || (!match[1] && !match[3])) return null;
    const from = match[1] ? Number(match[1]) : 1;
    const to = match[2] ? (match[3] ? Number(match[3]) : pageCount) : from;
    if (from < 1 || to > pageCount || from > to) return null;
    for (let page = from; page <= to; page += 1) pages.add(page - 1);
  }
  return [...pages].sort((a, b) => a - b);
}
