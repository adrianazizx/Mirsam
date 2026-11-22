import type { Direction, TextLayer, TextLine, ViewRect } from "../backend";

/**
 * Text selection over the engine's text layers (docs/SPEC.md section 6.1):
 * "selection moves visually but copies logically".
 *
 * A selection runs between two carets. A caret is a line (in reading order)
 * and a horizontal position on it, in displayed-page points. Lines between the
 * two carets are selected whole. On the first line, everything visually
 * *after* the caret in the block's direction is selected (to its left in a
 * right-to-left block); on the last line, everything visually before. On a
 * single line, everything between the two positions. So dragging across a
 * mixed Arabic/English line highlights exactly the glyphs the pointer passed
 * over, and the copied text is those glyphs' characters in logical order.
 *
 * Everything here is pure: the viewer feeds it text layers and pointer
 * positions and paints the rectangles it returns.
 */

/** A line of a page's text layer. Blocks and lines are in reading order. */
export interface LineRef {
  page: number;
  block: number;
  line: number;
}

export interface Caret extends LineRef {
  /** Displayed-page points. ±Infinity stand for the line's ends. */
  x: number;
}

export interface Selection {
  anchor: Caret;
  focus: Caret;
}

/** Selected glyph boxes of one page, merged into runs, in displayed-page points. */
export interface PageHighlight {
  page: number;
  rects: ViewRect[];
}

export type LayerLookup = (page: number) => TextLayer | undefined;

function lineAt(layers: LayerLookup, ref: LineRef): TextLine | undefined {
  return layers(ref.page)?.blocks[ref.block]?.lines[ref.line];
}

function directionAt(layers: LayerLookup, ref: LineRef): Direction {
  return layers(ref.page)?.blocks[ref.block]?.direction ?? "ltr";
}

/** Reading order of two lines: negative when `a` comes first. */
export function compareLines(a: LineRef, b: LineRef): number {
  return a.page - b.page || a.block - b.block || a.line - b.line;
}

/**
 * The caret for a point on a page: the nearest line, preferring lines the
 * point is vertically inside, and the point's x. Null on a page without text.
 */
export function caretAt(layer: TextLayer, page: number, x: number, y: number): Caret | null {
  let best: { caret: Caret; score: number } | null = null;
  layer.blocks.forEach((block, b) => {
    block.lines.forEach((line, l) => {
      const box = line.bounds;
      const dy = y < box.y ? box.y - y : y > box.y + box.height ? y - box.y - box.height : 0;
      const dx = x < box.x ? box.x - x : x > box.x + box.width ? x - box.x - box.width : 0;
      const score = dy * 4 + dx;
      if (!best || score < best.score) {
        best = { caret: { page, block: b, line: l, x }, score };
      }
    });
  });
  return (best as { caret: Caret } | null)?.caret ?? null;
}

/** True when the point lies on a line's text (for the text cursor). */
export function isOverText(layer: TextLayer, x: number, y: number): boolean {
  return layer.blocks.some((block) =>
    block.lines.some(
      (line) =>
        x >= line.bounds.x &&
        x <= line.bounds.x + line.bounds.width &&
        y >= line.bounds.y &&
        y <= line.bounds.y + line.bounds.height,
    ),
  );
}

/** Carets spanning the word under a point (double-click). */
export function wordAt(layer: TextLayer, page: number, x: number, y: number): Selection | null {
  const caret = caretAt(layer, page, x, y);
  if (!caret) return null;
  const line = layer.blocks[caret.block]?.lines[caret.line];
  if (!line) return null;
  // The span under the point, or the nearest one.
  let index = -1;
  let nearest = Number.POSITIVE_INFINITY;
  line.spans.forEach((span, i) => {
    if (!span.rect) return;
    const centre = span.rect.x + span.rect.width / 2;
    const inside = x >= span.rect.x && x <= span.rect.x + span.rect.width;
    const distance = inside ? 0 : Math.abs(centre - x);
    if (distance < nearest) {
      nearest = distance;
      index = i;
    }
  });
  if (index < 0) return null;
  const isWordSpan = (i: number) => {
    const span = line.spans[i];
    return span !== undefined && /\S/u.test(line.text.slice(span.start, span.end));
  };
  if (!isWordSpan(index)) return lineSelection(caret);
  let first = index;
  let last = index;
  while (first > 0 && isWordSpan(first - 1)) first -= 1;
  while (last < line.spans.length - 1 && isWordSpan(last + 1)) last += 1;
  let x0 = Number.POSITIVE_INFINITY;
  let x1 = Number.NEGATIVE_INFINITY;
  for (let i = first; i <= last; i += 1) {
    const rect = line.spans[i]?.rect;
    if (!rect) continue;
    x0 = Math.min(x0, rect.x);
    x1 = Math.max(x1, rect.x + rect.width);
  }
  return { anchor: { ...caret, x: x0 }, focus: { ...caret, x: x1 } };
}

/** Carets spanning a whole line (triple-click). */
export function lineSelection(ref: LineRef): Selection {
  return {
    anchor: { ...ref, x: Number.NEGATIVE_INFINITY },
    focus: { ...ref, x: Number.POSITIVE_INFINITY },
  };
}

/** Every line of a page, from its first to its last (select all on a page). */
export function pageSelection(layer: TextLayer, page: number): Selection | null {
  const lastBlock = layer.blocks.length - 1;
  const last = layer.blocks[lastBlock];
  if (!last || last.lines.length === 0 || layer.blocks[0]?.lines.length === 0) return null;
  const startOf = (direction: Direction) =>
    direction === "rtl" ? Number.POSITIVE_INFINITY : Number.NEGATIVE_INFINITY;
  const firstDirection = layer.blocks[0]?.direction ?? "ltr";
  return {
    anchor: { page, block: 0, line: 0, x: startOf(firstDirection) },
    focus: {
      page,
      block: lastBlock,
      line: last.lines.length - 1,
      x: -startOf(last.direction),
    },
  };
}

/** The selection's carets in reading order. */
export function ordered(selection: Selection): [Caret, Caret] {
  return compareLines(selection.anchor, selection.focus) <= 0
    ? [selection.anchor, selection.focus]
    : [selection.focus, selection.anchor];
}

/** True when the selection covers no glyph (a click without a drag). */
export function isCollapsed(selection: Selection, layers: LayerLookup): boolean {
  return selectedLines(selection, layers).every((line) => line.spans.length === 0);
}

interface SelectedLine extends LineRef {
  text: TextLine;
  /** Indices of the selected spans, ascending (logical order). */
  spans: number[];
}

/** For each line the selection touches (in reading order), its selected spans. */
export function selectedLines(selection: Selection, layers: LayerLookup): SelectedLine[] {
  const [start, end] = ordered(selection);
  const out: SelectedLine[] = [];
  for (let page = start.page; page <= end.page; page += 1) {
    const layer = layers(page);
    if (!layer) continue;
    layer.blocks.forEach((block, b) => {
      block.lines.forEach((text, l) => {
        const ref = { page, block: b, line: l };
        if (compareLines(ref, start) < 0 || compareLines(ref, end) > 0) return;
        const first = compareLines(ref, start) === 0;
        const last = compareLines(ref, end) === 0;
        const rtl = block.direction === "rtl";
        // Visual extent selected on this line.
        let lo = Number.NEGATIVE_INFINITY;
        let hi = Number.POSITIVE_INFINITY;
        if (first && last) {
          lo = Math.min(start.x, end.x);
          hi = Math.max(start.x, end.x);
        } else if (first) {
          if (rtl) hi = start.x;
          else lo = start.x;
        } else if (last) {
          if (rtl) lo = end.x;
          else hi = end.x;
        }
        out.push({ ...ref, text, spans: spansBetween(text, lo, hi) });
      });
    });
  }
  return out;
}

/**
 * Spans whose glyph centre lies strictly between lo and hi (a click selects
 * nothing; the pointer has to pass a glyph's middle). An inferred space joins
 * when both its neighbours do.
 */
function spansBetween(line: TextLine, lo: number, hi: number): number[] {
  const inRange = line.spans.map((span) => {
    if (!span.rect) return null;
    const centre = span.rect.x + span.rect.width / 2;
    return centre > lo && centre < hi;
  });
  const selected: number[] = [];
  inRange.forEach((value, i) => {
    if (value === true) {
      selected.push(i);
      return;
    }
    if (value === null) {
      const before = inRange.slice(0, i).findLast((v) => v !== null);
      const after = inRange.slice(i + 1).find((v) => v !== null);
      if (before === true && after === true) selected.push(i);
    }
  });
  return selected;
}

/**
 * The selected text in logical order: each line's selected characters in
 * reading order; lines joined by a newline, blocks and pages by a blank line.
 * Where a visual range picks out pieces that are not contiguous in reading
 * order (possible in mixed-direction lines), the pieces are joined by a space.
 */
export function selectedText(selection: Selection, layers: LayerLookup): string {
  let out = "";
  let previous: LineRef | null = null;
  for (const line of selectedLines(selection, layers)) {
    if (line.spans.length === 0) continue;
    const pieces: string[] = [];
    let piece = "";
    let last = -2;
    for (const index of line.spans) {
      const span = line.text.spans[index];
      if (!span) continue;
      if (index !== last + 1 && piece) {
        pieces.push(piece);
        piece = "";
      }
      piece += line.text.text.slice(span.start, span.end);
      last = index;
    }
    if (piece) pieces.push(piece);
    const text = pieces
      .map((p) => p.trim())
      .filter(Boolean)
      .join(" ");
    if (!text) continue;
    if (previous) {
      const sameBlock = previous.page === line.page && previous.block === line.block;
      out += sameBlock ? "\n" : "\n\n";
    }
    out += text;
    previous = line;
  }
  return out;
}

/**
 * Highlight rectangles per page: selected glyph boxes of each line merged into
 * visual runs (boxes closer than the line height join, which also covers
 * the word spaces between them).
 */
export function highlights(selection: Selection, layers: LayerLookup): PageHighlight[] {
  const byPage = new Map<number, ViewRect[]>();
  for (const line of selectedLines(selection, layers)) {
    const boxes = line.spans
      .map((index) => line.text.spans[index]?.rect)
      .filter((rect): rect is ViewRect => Boolean(rect))
      .sort((a, b) => a.x - b.x);
    if (boxes.length === 0) continue;
    const top = line.text.bounds.y;
    const height = line.text.bounds.height;
    const join = height;
    const runs: ViewRect[] = [];
    for (const box of boxes) {
      const run = runs.at(-1);
      if (run && box.x <= run.x + run.width + join) {
        run.width = Math.max(run.width, box.x + box.width - run.x);
      } else {
        runs.push({ x: box.x, y: top, width: box.width, height });
      }
    }
    const list = byPage.get(line.page) ?? [];
    list.push(...runs);
    byPage.set(line.page, list);
  }
  return [...byPage].map(([page, rects]) => ({ page, rects }));
}

/** The pages a selection covers, first to last. */
export function selectionPages(selection: Selection): [number, number] {
  const [start, end] = ordered(selection);
  return [start.page, end.page];
}

/** The direction of the line a caret is on. */
export function caretDirection(caret: LineRef, layers: LayerLookup): Direction {
  return directionAt(layers, caret);
}

export { lineAt };
