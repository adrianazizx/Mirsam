/**
 * Caret and selection geometry over the engine's layout of a block being
 * edited (ADR 0024). Everything here is in the layout's own coordinates:
 * points from the box's left edge and top, y down. Offsets are UTF-16, as the
 * hidden text field counts them.
 */
import type { BlockLayout, TextBlockInfo } from "../backend/types";

/** A press this close outside a block's box still lands on it, in points. */
const BLOCK_SLOP = 2;

/**
 * The block a press at page point (`x`, `y`) lands on: the smallest whose box
 * (with a little slop) holds it, or -1.
 */
export function blockAt(blocks: TextBlockInfo[], x: number, y: number): number {
  let found = -1;
  let area = Number.POSITIVE_INFINITY;
  blocks.forEach(({ rect }, index) => {
    const inside =
      x >= rect.x - BLOCK_SLOP &&
      x <= rect.x + rect.width + BLOCK_SLOP &&
      y >= rect.y - BLOCK_SLOP &&
      y <= rect.y + rect.height + BLOCK_SLOP;
    if (inside && rect.width * rect.height < area) {
      found = index;
      area = rect.width * rect.height;
    }
  });
  return found;
}

export type Caret = BlockLayout["carets"][number];

/** The line an offset is shown on: a soft break's offset starts the next line. */
export function lineOf(layout: BlockLayout, offset: number): number {
  const { lines } = layout;
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (!line) continue;
    const last = index === lines.length - 1;
    if (offset < line.end || (last && offset <= line.end)) return offset >= line.start ? index : 0;
  }
  return Math.max(0, lines.length - 1);
}

/** Where the caret stands for `offset`, or null when the layout has no stop for it yet. */
export function caretFor(layout: BlockLayout, offset: number): Caret | null {
  const line = lineOf(layout, offset);
  let fallback: Caret | null = null;
  for (const stop of layout.carets) {
    if (stop.offset !== offset) continue;
    // A split stop (where directions meet) has a twin: the first is primary.
    if (stop.line === line) return stop;
    fallback ??= stop;
  }
  return fallback;
}

/** The offset whose caret stop is nearest (`x`, `y`). */
export function offsetAt(layout: BlockLayout, x: number, y: number): number {
  const { lines } = layout;
  let line = lines.findIndex((candidate) => y < candidate.top + candidate.height);
  if (line < 0) line = lines.length - 1;
  let best: Caret | null = null;
  for (const stop of layout.carets) {
    if (stop.line !== line) continue;
    if (!best || Math.abs(stop.x - x) < Math.abs(best.x - x)) best = stop;
  }
  return best?.offset ?? 0;
}

/**
 * The offset one line up (`step` −1) or down (+1) from `offset`, nearest
 * `goalX` (the x the caret had before a run of vertical moves). The first
 * line's start or the last line's end when there is no line that way.
 */
export function verticalMove(
  layout: BlockLayout,
  offset: number,
  step: -1 | 1,
  goalX: number,
): number {
  const target = lineOf(layout, offset) + step;
  const line = layout.lines[target];
  if (!line) {
    if (step < 0) return layout.lines[0]?.start ?? 0;
    return layout.lines.at(-1)?.end ?? offset;
  }
  let best: Caret | null = null;
  for (const stop of layout.carets) {
    if (stop.line !== target) continue;
    if (!best || Math.abs(stop.x - goalX) < Math.abs(best.x - goalX)) best = stop;
  }
  return best?.offset ?? line.start;
}

/** The start or end of the line `offset` is on, in reading order (Home, End). */
export function lineEdge(layout: BlockLayout, offset: number, edge: "start" | "end"): number {
  const line = layout.lines[lineOf(layout, offset)];
  if (!line) return offset;
  if (edge === "start") return line.start;
  // A soft break's trailing space (or a hard break) ends the line: stop before it.
  const last = layout.lines.at(-1) === line;
  return last ? line.end : Math.max(line.start, line.end - 1);
}

/** Rectangles covering [start, end): one per cluster it touches, in layout coordinates. */
export function selectionRects(
  layout: BlockLayout,
  start: number,
  end: number,
): { x: number; y: number; width: number; height: number }[] {
  if (end <= start) return [];
  const rects: { x: number; y: number; width: number; height: number }[] = [];
  for (const cluster of layout.clusters) {
    if (cluster.end <= start || cluster.start >= end) continue;
    const line = layout.lines[cluster.line];
    if (!line) continue;
    rects.push({
      x: cluster.left,
      y: line.top,
      width: cluster.right - cluster.left,
      height: line.height,
    });
  }
  return rects;
}

/** The word around `offset` (UTF-16), for a double click. */
export function wordAround(text: string, offset: number): [number, number] {
  const segmenter = new Intl.Segmenter(undefined, { granularity: "word" });
  for (const { segment, index, isWordLike } of segmenter.segment(text)) {
    if (offset >= index && offset <= index + segment.length && isWordLike) {
      return [index, index + segment.length];
    }
  }
  return [offset, offset];
}
