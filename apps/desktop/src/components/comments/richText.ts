/**
 * The paragraph editor's rich field (ADR 0025): plain text with bold and
 * italic stretches, held in the browser's own editable element so the
 * browser keeps the caret, selection, IME and undo (ADR 0022).
 *
 * These functions move between that element and the model the engine takes:
 * the text, and marks over it in UTF-16 offsets.
 */
import type { BlockMark, StyledRun } from "../../backend/types";

/** The field's content as the engine takes it. */
export interface StyledValue {
  text: string;
  /** Bold or italic stretches; text no mark covers is neither. */
  marks: BlockMark[];
}

/** A paragraph's runs as marks: the stretches that are bold or italic. */
export function marksOfRuns(runs: StyledRun[]): BlockMark[] {
  return normalize(runs.map(({ start, end, bold, italic }) => ({ start, end, bold, italic })));
}

/** Bold and italic for each UTF-16 unit of a text `length` long. */
function flagsOf(length: number, marks: BlockMark[]): { bold: boolean; italic: boolean }[] {
  const flags = Array.from({ length }, () => ({ bold: false, italic: false }));
  for (const mark of marks) {
    for (let i = Math.max(0, mark.start); i < Math.min(length, mark.end); i += 1) {
      flags[i] = { bold: mark.bold, italic: mark.italic };
    }
  }
  return flags;
}

/** Marks merged where they touch with the same look; plain stretches dropped. */
function normalize(marks: BlockMark[]): BlockMark[] {
  const out: BlockMark[] = [];
  for (const mark of [...marks].sort((a, b) => a.start - b.start)) {
    if (mark.end <= mark.start || !(mark.bold || mark.italic)) continue;
    const last = out.at(-1);
    if (last && last.end === mark.start && last.bold === mark.bold && last.italic === mark.italic) {
      last.end = mark.end;
    } else {
      out.push({ ...mark });
    }
  }
  return out;
}

/** Whether two sets of marks give `text` the same look. */
export function sameMarks(text: string, a: BlockMark[], b: BlockMark[]): boolean {
  const x = flagsOf(text.length, a);
  const y = flagsOf(text.length, b);
  return x.every((f, i) => f.bold === y[i]?.bold && f.italic === y[i]?.italic);
}

/** Text with its line breaks as `<br>`s. */
function fragmentOf(text: string): DocumentFragment {
  const fragment = document.createDocumentFragment();
  text.split("\n").forEach((line, i) => {
    if (i > 0) fragment.append(document.createElement("br"));
    if (line) fragment.append(document.createTextNode(line));
  });
  return fragment;
}

/** Fills `root` with `value`: `<b>` and `<i>` around its marked stretches. */
export function fillField(root: HTMLElement, value: StyledValue): void {
  const { text } = value;
  const flags = flagsOf(text.length, value.marks);
  root.replaceChildren();
  let i = 0;
  while (i < text.length) {
    let j = i + 1;
    const { bold, italic } = flags[i] ?? { bold: false, italic: false };
    while (j < text.length && flags[j]?.bold === bold && flags[j]?.italic === italic) j += 1;
    let node: Node = fragmentOf(text.slice(i, j));
    if (italic) {
      const element = document.createElement("i");
      element.append(node);
      node = element;
    }
    if (bold) {
      const element = document.createElement("b");
      element.append(node);
      node = element;
    }
    root.append(node);
    i = j;
  }
}

/** Bold as `element` sets it, or as inherited when it does not say. */
function boldOf(element: HTMLElement, inherited: boolean): boolean {
  if (element.tagName === "B" || element.tagName === "STRONG") return true;
  const weight = element.style.fontWeight;
  if (weight === "bold" || weight === "bolder") return true;
  if (weight === "normal" || weight === "lighter") return false;
  const number = Number.parseInt(weight, 10);
  return Number.isFinite(number) ? number >= 600 : inherited;
}

/** Italic as `element` sets it, or as inherited when it does not say. */
function italicOf(element: HTMLElement, inherited: boolean): boolean {
  if (element.tagName === "I" || element.tagName === "EM") return true;
  const style = element.style.fontStyle;
  if (style === "italic" || style.startsWith("oblique")) return true;
  if (style === "normal") return false;
  return inherited;
}

/**
 * Reads `root` back: its text (line breaks for `<br>` and between block
 * elements) and its bold and italic stretches. The browser's own formatting
 * may un-bold inside a bold element with an inline style, so each element
 * either says or inherits. Trailing line breaks (the browser keeps one so an
 * empty last line has height) are not text.
 */
export function readField(root: HTMLElement): StyledValue {
  let text = "";
  const marks: BlockMark[] = [];
  const push = (piece: string, bold: boolean, italic: boolean) => {
    if (!piece) return;
    const start = text.length;
    text += piece;
    if (bold || italic) marks.push({ start, end: text.length, bold, italic });
  };
  const walk = (node: Node, bold: boolean, italic: boolean) => {
    for (const child of node.childNodes) {
      if (child.nodeType === Node.TEXT_NODE) {
        push((child.textContent ?? "").replace(/ /g, " "), bold, italic);
      } else if (child instanceof HTMLElement) {
        if (child.tagName === "BR") {
          push("\n", false, false);
          continue;
        }
        const b = boldOf(child, bold);
        const i = italicOf(child, italic);
        const block = child.tagName === "DIV" || child.tagName === "P";
        if (block && text.length > 0 && !text.endsWith("\n")) push("\n", false, false);
        walk(child, b, i);
      }
    }
  };
  walk(root, false, false);
  const trimmed = text.replace(/\n+$/u, "");
  return {
    text: trimmed,
    marks: normalize(
      marks
        .map((m) => ({ ...m, end: Math.min(m.end, trimmed.length) }))
        .filter((m) => m.start < trimmed.length),
    ),
  };
}
