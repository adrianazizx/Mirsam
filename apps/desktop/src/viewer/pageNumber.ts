/**
 * Parses a page number typed by the user: Western (0–9), Arabic-Indic (٠–٩),
 * or Extended Arabic-Indic (۰–۹, Persian/Urdu) digits, surrounding spaces
 * ignored. Returns the one-based number, or null for anything else.
 */
export function parsePageNumber(text: string): number | null {
  let digits = "";
  for (const char of text.trim()) {
    const code = char.codePointAt(0) ?? 0;
    if (code >= 0x30 && code <= 0x39) digits += char;
    else if (code >= 0x660 && code <= 0x669) digits += String(code - 0x660);
    else if (code >= 0x6f0 && code <= 0x6f9) digits += String(code - 0x6f0);
    else return null;
  }
  if (digits === "") return null;
  const value = Number.parseInt(digits, 10);
  return Number.isSafeInteger(value) && value >= 1 ? value : null;
}
