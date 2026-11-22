/**
 * Loose matching for the command palette: case, Arabic tashkeel and tatweel,
 * and alef forms are ignored, and each word typed must appear in order (not
 * necessarily adjacent), so "zi" finds "Zoom in" and "تكب" finds "تكبير".
 */
export function foldForMatch(text: string): string {
  return text
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯؐ-ًؚ-ٰٟـ]/g, "")
    .replace(/[أإآٱ]/g, "ا")
    .replace(/ى/g, "ي");
}

function isSubsequence(needle: string, haystack: string): boolean {
  let at = 0;
  for (const char of needle) {
    at = haystack.indexOf(char, at);
    if (at < 0) return false;
    at += char.length;
  }
  return true;
}

export function paletteMatch(text: string, query: string): boolean {
  const haystack = foldForMatch(text);
  return foldForMatch(query)
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => haystack.includes(word) || isSubsequence(word, haystack));
}
