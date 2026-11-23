import type { TextLayer } from "../backend";

/**
 * A page's text for screen readers. The page itself is pictures (tiles), so
 * its text, in reading order, goes into the page as hidden paragraphs: one per
 * block, with the block's direction, and each run of another script marked
 * with its language so the screen reader switches voice ("an Arabic sentence
 * with an English product name in it").
 *
 * Languages are guessed from the script: Arabic script is read as Arabic,
 * Hebrew as Hebrew, Latin as English, unless the PDF's own `/Lang` names a
 * language written in that script (French for Latin, Persian or Urdu for
 * Arabic script), which then wins. The tagged structure's per-span `/Lang`
 * is not read.
 */

export type ReadableNote = "noText" | "suspect";

type Script = "ar" | "he" | "en";

function scriptOf(code: number): Script | null {
  if (
    (code >= 0x0600 && code <= 0x06ff) ||
    (code >= 0x0750 && code <= 0x077f) ||
    (code >= 0x08a0 && code <= 0x08ff) ||
    (code >= 0xfb50 && code <= 0xfdff) ||
    (code >= 0xfe70 && code <= 0xfeff)
  ) {
    // Arabic-Indic digits are numbers, not words: they follow the text around them.
    if ((code >= 0x0660 && code <= 0x0669) || (code >= 0x06f0 && code <= 0x06f9)) return null;
    return "ar";
  }
  if (code >= 0x0590 && code <= 0x05ff) return "he";
  if ((code >= 0x41 && code <= 0x5a) || (code >= 0x61 && code <= 0x7a)) return "en";
  if (code >= 0xc0 && code <= 0x24f && code !== 0xd7 && code !== 0xf7) return "en";
  return null;
}

export interface ScriptRun {
  text: string;
  /** Null: digits, spaces and punctuation only. */
  lang: Script | null;
}

/**
 * Splits text into runs of one script. A word takes the script of its letters,
 * with its digits and punctuation ("SA-2026-00417", "(نسخة)"). Between words,
 * spaces and lone numbers or signs take the script around them when both sides
 * agree, and otherwise none: they are read in the paragraph's language, as
 * bidi resolves neutrals to the paragraph's direction.
 */
export function scriptRuns(text: string): ScriptRun[] {
  const chars = [...text];
  const scripts = chars.map((char) => scriptOf(char.codePointAt(0) ?? 0));
  // A word (no spaces) whose letters are of one script lends it to its other characters.
  let start = 0;
  for (let i = 0; i <= chars.length; i += 1) {
    if (i < chars.length && !/\s/u.test(chars[i] ?? "")) continue;
    const found = new Set(scripts.slice(start, i).filter((script) => script !== null));
    if (found.size === 1) {
      const [script] = found;
      for (let j = start; j < i; j += 1) scripts[j] = script ?? null;
    }
    start = i + 1;
  }
  const next: (Script | null)[] = new Array(chars.length).fill(null);
  for (let i = chars.length - 2; i >= 0; i -= 1) next[i] = scripts[i + 1] ?? next[i + 1] ?? null;
  const runs: ScriptRun[] = [];
  let previous: Script | null = null;
  chars.forEach((char, i) => {
    const own = scripts[i] ?? null;
    const lang = own ?? (previous !== null && previous === next[i] ? previous : null);
    if (own) previous = own;
    const last = runs.at(-1);
    if (last && last.lang === lang) last.text += char;
    else runs.push({ text: char, lang });
  });
  return runs;
}

/** The language most of `text`'s letters are in, if it has letters. */
export function languageOf(text: string): Script | null {
  const counts = new Map<Script, number>();
  for (const run of scriptRuns(text)) {
    if (run.lang) counts.set(run.lang, (counts.get(run.lang) ?? 0) + run.text.length);
  }
  let best: Script | null = null;
  for (const [lang, count] of counts)
    if (best === null || count > (counts.get(best) ?? 0)) best = lang;
  return best;
}

/** Languages written in Arabic or Hebrew script, by primary subtag. */
const ARABIC_SCRIPT = new Set(["ar", "fa", "ur", "ps", "ckb", "ku", "sd", "ug"]);
const HEBREW_SCRIPT = new Set(["he", "iw", "yi"]);
/** Languages not usually written in Latin script, whose `/Lang` says nothing of Latin text. */
const OTHER_SCRIPT = new Set([
  ..."am be bg bn el gu hi hy ja ka km kn ko lo mk ml mr my ne ru si ta te th uk zh".split(" "),
]);

/** The language tag for text in `script`, given the document's own `/Lang`. */
export function languageTag(script: Script, documentLanguage?: string | null): string {
  const primary = documentLanguage?.split("-")[0]?.toLowerCase();
  if (!documentLanguage || !primary) return script;
  if (script === "ar") return ARABIC_SCRIPT.has(primary) ? documentLanguage : script;
  if (script === "he") return HEBREW_SCRIPT.has(primary) ? documentLanguage : script;
  const latin =
    !ARABIC_SCRIPT.has(primary) && !HEBREW_SCRIPT.has(primary) && !OTHER_SCRIPT.has(primary);
  return latin ? documentLanguage : script;
}

/**
 * Fills `host` with the page's text as paragraphs, or a note when it has none.
 * `documentLanguage` is the PDF's `/Lang`, when it states one.
 */
export function fillReadableText(
  host: HTMLElement,
  layer: TextLayer,
  note: (kind: ReadableNote) => string,
  documentLanguage?: string | null,
): void {
  const paragraphs: HTMLElement[] = [];
  const blocks = layer.blocks.filter((block) => block.lines.some((line) => line.text.trim()));
  if (layer.quality === "noText" || blocks.length === 0) {
    paragraphs.push(paragraph(note("noText")));
  } else if (layer.quality === "suspect") {
    paragraphs.push(paragraph(note("suspect")));
  }
  for (const block of blocks) {
    const text = block.lines
      .map((line) => line.text.trim())
      .filter(Boolean)
      .join(" ");
    const element = document.createElement("p");
    element.dir = block.direction;
    const lang = languageOf(text);
    if (lang) element.lang = languageTag(lang, documentLanguage);
    for (const run of scriptRuns(text)) {
      if (run.lang === null || run.lang === lang) {
        element.append(run.text);
      } else {
        const span = document.createElement("span");
        span.lang = languageTag(run.lang, documentLanguage);
        span.textContent = run.text;
        element.append(span);
      }
    }
    paragraphs.push(element);
  }
  host.replaceChildren(...paragraphs);
}

function paragraph(text: string): HTMLElement {
  const element = document.createElement("p");
  element.textContent = text;
  return element;
}
