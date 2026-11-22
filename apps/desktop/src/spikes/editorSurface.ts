/**
 * Milestone 3 spike (throwaway): the in-place editor's surface (spec §7, ADR 0022).
 *
 * Runs inside the real webview with `SAFHA_SELFTEST_SPIKE=editor-surface`
 * (scripts/selftest.sh). Two candidates for showing a paragraph while it is
 * edited:
 *
 * - **A, our own surface:** the engine lays the paragraph out and returns the
 *   glyphs' outlines (from the very subsets written to the PDF) and the caret
 *   stops; the webview draws those on a canvas, and a hidden text field takes
 *   keyboard and IME input. Caret and glyphs agree with the saved text by
 *   construction; what is measured is typing latency (spec §11: ≤ 32 ms).
 * - **B, a `contenteditable` overlay:** WebKit lays out and draws the text
 *   itself, with the same fonts, one span per engine run so it even uses the
 *   engine's font choice. What is measured is how far its caret and line
 *   breaks stray from the engine's layout, which is what gets saved.
 */
import { invoke } from "@tauri-apps/api/core";
import { nextFrame } from "../selftest";

interface Line {
  top: number;
  baseline: number;
  height: number;
  start: number;
  end: number;
  rtl: boolean;
}
interface Run {
  line: number;
  font: string;
  start: number;
  end: number;
  rtl: boolean;
}
interface Glyph {
  font: number;
  id: number;
  x: number;
  y: number;
  size: number;
}
interface Caret {
  offset: number;
  line: number;
  x: number;
  split: boolean;
}
interface Outline {
  font: number;
  id: number;
  unitsPerEm: number;
  path: string;
}
interface Layout {
  height: number;
  lines: Line[];
  runs: Run[];
  glyphs: Glyph[];
  carets: Caret[];
  outlines: Outline[];
  layoutMicros: number;
  subsetMicros: number;
  outlineMicros: number;
}

const CSS_FAMILY: Record<string, string> = {
  IBMPlexSans: "IBM Plex Sans",
  IBMPlexSansArabic: "IBM Plex Sans Arabic",
};

const CASES: { name: string; text: string }[] = [
  {
    name: "english",
    text: "The tenant shall pay the rent on the first day of each month, without any deduction or delay.",
  },
  { name: "arabic", text: "يدفع المستأجر الإيجار في اليوم الأول من كل شهر دون أي خصم أو تأخير." },
  {
    name: "mixed",
    text: "اشتريت 3 أجهزة iPhone 15 Pro بسعر 4,500 ريال من متجر Apple في الرياض يوم 12 مارس.",
  },
  { name: "vocalized", text: "بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ، وَصَلَّى اللَّهُ عَلَى نَبِيِّنَا مُحَمَّدٍ." },
  { name: "lam-alef", text: "لا إله إلا الله، ولا قوة إلا بالله، والسلام عليكم ورحمة الله." },
];
const WIDTH = 300;

function engineLayout(
  text: string,
  size: number,
  justify: boolean,
  known: [number, number][] = [],
) {
  return invoke<Layout>("spike_editor_layout", {
    request: { text, family: "sans", size, width: WIDTH, justify, known },
  });
}

function stats(values: number[]) {
  if (values.length === 0) return { count: 0, mean: 0, p95: 0, max: 0 };
  const sorted = [...values].sort((a, b) => a - b);
  const pick = (q: number) =>
    sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? 0;
  const mean = values.reduce((sum, v) => sum + v, 0) / values.length;
  const round = (v: number) => Math.round(v * 100) / 100;
  return {
    count: values.length,
    mean: round(mean),
    median: round(pick(0.5)),
    p95: round(pick(0.95)),
    max: round(sorted.at(-1) ?? 0),
  };
}

/** The text node and offset holding UTF-16 `offset` of `root`'s text. */
function position(root: Node, offset: number): [Node, number] | null {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let left = offset;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const length = node.textContent?.length ?? 0;
    if (left <= length) return [node, left];
    left -= length;
  }
  return null;
}

/** B: a contenteditable paragraph laid out by WebKit with the engine's fonts. */
function overlay(text: string, layout: Layout, size: number, justify: boolean): HTMLDivElement {
  const div = document.createElement("div");
  div.contentEditable = "true";
  div.dir = layout.lines[0]?.rtl ? "rtl" : "ltr";
  Object.assign(div.style, {
    position: "fixed",
    top: "0",
    left: "0",
    width: `${WIDTH}px`,
    fontSize: `${size}px`,
    lineHeight: "normal",
    whiteSpace: "pre-wrap",
    textAlign: justify ? "justify" : "start",
    zIndex: "10000",
    background: "white",
    color: "black",
    padding: "0",
    margin: "0",
    fontKerning: "normal",
  });
  // One span per stretch of the engine's font choice, in logical order.
  const pieces = [...layout.runs].sort((a, b) => a.start - b.start);
  const merged: Run[] = [];
  for (const run of pieces) {
    const last = merged.at(-1);
    if (last && last.font === run.font && last.end === run.start) last.end = run.end;
    else merged.push({ ...run });
  }
  for (const run of merged) {
    const span = document.createElement("span");
    span.style.fontFamily = `"${CSS_FAMILY[run.font] ?? run.font}"`;
    span.textContent = text.slice(run.start, run.end);
    div.append(span);
  }
  document.body.append(div);
  return div;
}

/** B's caret against the engine's: deviation per stop and line agreement. */
async function compare(text: string, size: number, justify: boolean) {
  const layout = await engineLayout(text, size, justify);
  const div = overlay(text, layout, size, justify);
  await nextFrame();
  const box = div.getBoundingClientRect();
  const deviations: number[] = [];
  const browserTops = new Map<number, number>();
  const lineOf: [number, number][] = [];
  for (const caret of layout.carets) {
    if (caret.split) continue;
    const at = position(div, caret.offset);
    if (!at) continue;
    const range = document.createRange();
    range.setStart(at[0], at[1]);
    range.collapse(true);
    const rect = range.getClientRects()[0] ?? range.getBoundingClientRect();
    if (!rect || (rect.width === 0 && rect.height === 0 && rect.left === 0)) continue;
    const top = Math.round(rect.top - box.top);
    if (!browserTops.has(top)) browserTops.set(top, browserTops.size);
    lineOf.push([caret.line, browserTops.get(top) ?? -1]);
    // Same line only: a different break makes the x meaningless.
    deviations.push(Math.abs(rect.left - box.left - caret.x));
  }
  // Lines in the order their tops appear.
  const order = [...browserTops.keys()].sort((a, b) => a - b);
  const rank = new Map(order.map((top, i) => [browserTops.get(top) ?? -1, i]));
  const sameLine = lineOf.filter(([engine, browser]) => rank.get(browser) === engine).length;
  const onSameLine = deviations.filter((_, i) => rank.get(lineOf[i]?.[1] ?? -1) === lineOf[i]?.[0]);
  div.remove();
  return {
    engineLines: layout.lines.length,
    browserLines: browserTops.size,
    stops: lineOf.length,
    stopsOnSameLine: sameLine,
    caretDeviationPx: stats(onSameLine),
    over1px: onSameLine.filter((d) => d > 1).length,
    over2px: onSameLine.filter((d) => d > 2).length,
  };
}

/** A: draws the engine's glyph outlines; the outline cache survives keystrokes. */
class Surface {
  readonly canvas = document.createElement("canvas");
  private readonly paths = new Map<string, { path: Path2D; unitsPerEm: number }>();
  constructor(private readonly scale: number) {
    Object.assign(this.canvas.style, {
      position: "fixed",
      top: "0",
      left: "320px",
      zIndex: "10000",
      background: "white",
    });
    document.body.append(this.canvas);
  }
  known(): [number, number][] {
    return [...this.paths.keys()].map((key) => key.split(":").map(Number) as [number, number]);
  }
  draw(layout: Layout, caretOffset: number) {
    for (const outline of layout.outlines) {
      this.paths.set(`${outline.font}:${outline.id}`, {
        path: new Path2D(outline.path),
        unitsPerEm: outline.unitsPerEm,
      });
    }
    const width = Math.ceil(WIDTH * this.scale);
    const height = Math.ceil((layout.height + 4) * this.scale);
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
      this.canvas.style.width = `${WIDTH}px`;
      this.canvas.style.height = `${layout.height + 4}px`;
    }
    const ctx = this.canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = "black";
    for (const glyph of layout.glyphs) {
      const outline = this.paths.get(`${glyph.font}:${glyph.id}`);
      if (!outline) continue;
      const k = (glyph.size / outline.unitsPerEm) * this.scale;
      ctx.setTransform(k, 0, 0, -k, glyph.x * this.scale, glyph.y * this.scale);
      ctx.fill(outline.path);
    }
    const caret = layout.carets.find((stop) => stop.offset === caretOffset);
    const line = caret ? layout.lines[caret.line] : undefined;
    if (caret && line) {
      ctx.setTransform(this.scale, 0, 0, this.scale, 0, 0);
      ctx.fillRect(caret.x - 0.5, line.top, 1, line.height);
    }
  }
}

/** A: keystroke → engine layout → drawn, timed to the next frame. */
async function typeOnSurface(start: string, typed: string, size: number) {
  const surface = new Surface(window.devicePixelRatio || 1);
  let text = start;
  const latencies: number[] = [];
  const engine: number[] = [];
  const payload: number[] = [];
  for (const ch of typed) {
    const began = performance.now();
    text += ch;
    const layout = await engineLayout(text, size, false, surface.known());
    surface.draw(layout, text.length);
    await nextFrame();
    latencies.push(performance.now() - began);
    engine.push((layout.layoutMicros + layout.subsetMicros + layout.outlineMicros) / 1000);
    payload.push(JSON.stringify(layout).length);
  }
  surface.canvas.remove();
  return {
    keystrokes: typed.length,
    chars: text.length,
    latencyMs: stats(latencies),
    engineMs: stats(engine),
    payloadBytes: stats(payload),
  };
}

/** B: the browser's own typing, timed to the next frame. */
async function typeInOverlay(start: string, typed: string, size: number) {
  const layout = await engineLayout(start, size, false);
  const div = overlay(start, layout, size, false);
  div.focus();
  const end = position(div, start.length);
  if (end) getSelection()?.collapse(end[0], end[1]);
  const latencies: number[] = [];
  for (const ch of typed) {
    const began = performance.now();
    document.execCommand("insertText", false, ch);
    await nextFrame();
    latencies.push(performance.now() - began);
  }
  div.remove();
  return { keystrokes: typed.length, latencyMs: stats(latencies) };
}

/** A's input plumbing: a hidden text field reports what was typed and deleted. */
async function inputPlumbing() {
  const field = document.createElement("textarea");
  Object.assign(field.style, {
    position: "fixed",
    top: "0",
    left: "0",
    opacity: "0",
    width: "1px",
    height: "1px",
  });
  document.body.append(field);
  const seen: string[] = [];
  field.addEventListener("beforeinput", (event) => {
    seen.push(`${event.inputType}:${event.data ?? ""}`);
  });
  field.focus();
  document.execCommand("insertText", false, "ب");
  document.execCommand("insertText", false, "َ");
  document.execCommand("delete", false);
  await nextFrame();
  field.remove();
  return { events: seen, focused: true };
}

export async function runEditorSurfaceSpike(): Promise<Record<string, unknown>> {
  await document.fonts.ready;
  await Promise.all([
    document.fonts.load('12px "IBM Plex Sans"', "Aa"),
    document.fonts.load('12px "IBM Plex Sans Arabic"', "بت"),
  ]);
  const fidelity: Record<string, unknown> = {};
  for (const { name, text } of CASES) {
    for (const size of [12, 24]) {
      fidelity[`${name}@${size}`] = await compare(text, size, false);
    }
    fidelity[`${name}@12-justified`] = await compare(text, 12, true);
  }
  const arabicTyped = " وسيدفع المبلغ";
  const englishTyped = " and pay later";
  const long = CASES[1]?.text.repeat(6) ?? "";
  const typing = {
    surfaceShortArabic: await typeOnSurface(CASES[1]?.text ?? "", arabicTyped, 12),
    surfaceShortEnglish: await typeOnSurface(CASES[0]?.text ?? "", englishTyped, 12),
    surfaceLongArabic: await typeOnSurface(long, arabicTyped, 12),
    overlayShortArabic: await typeInOverlay(CASES[1]?.text ?? "", arabicTyped, 12),
    overlayLongArabic: await typeInOverlay(long, arabicTyped, 12),
  };
  return {
    spike: "editor-surface",
    userAgent: navigator.userAgent,
    devicePixelRatio: window.devicePixelRatio,
    fidelity,
    typing,
    input: await inputPlumbing(),
  };
}
