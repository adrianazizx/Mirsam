import type { PageAnnotation, StrokePoint, ViewRect } from "../backend";

/**
 * Annotation geometry for the viewer (ADRs 0017 and 0018), in displayed-page
 * points: which annotation a press lands on, what an eraser touches, and how
 * a pen stroke is drawn while it waits for the engine's rendering.
 */

/** The pen's nominal width in points; the engine's `Ink::DEFAULT_WIDTH`. */
export const INK_WIDTH = 2;
/** A press this close to a stroke (in points, beyond its half width) hits it. */
export const STROKE_TOLERANCE = 3;

/** Distance from `p` to the segment from `a` to `b`. */
export function distanceToSegment(
  p: { x: number; y: number },
  a: [number, number],
  b: [number, number],
): number {
  const [ax, ay] = a;
  const [dx, dy] = [b[0] - ax, b[1] - ay];
  const length = dx * dx + dy * dy;
  const t =
    length === 0 ? 0 : Math.max(0, Math.min(1, ((p.x - ax) * dx + (p.y - ay) * dy) / length));
  return Math.hypot(p.x - (ax + t * dx), p.y - (ay + t * dy));
}

/** Whether `p` lies within `tolerance` of a stroke. */
export function nearStroke(
  p: { x: number; y: number },
  stroke: [number, number][],
  tolerance: number,
): boolean {
  if (stroke.length === 1) {
    const [only] = stroke;
    return only !== undefined && Math.hypot(p.x - only[0], p.y - only[1]) <= tolerance;
  }
  for (let i = 1; i < stroke.length; i += 1) {
    const a = stroke[i - 1];
    const b = stroke[i];
    if (a && b && distanceToSegment(p, a, b) <= tolerance) return true;
  }
  return false;
}

function inside(p: { x: number; y: number }, rect: ViewRect): boolean {
  return (
    p.x >= rect.x && p.x <= rect.x + rect.width && p.y >= rect.y && p.y <= rect.y + rect.height
  );
}

/**
 * Whether a press at `p` lands on an annotation: on a marked run or a note's
 * icon, or near an ink stroke (its box, when its strokes could not be read).
 */
export function hits(annotation: PageAnnotation, p: { x: number; y: number }): boolean {
  if (annotation.kind === "ink" && annotation.strokes.length > 0) {
    const tolerance = INK_WIDTH + STROKE_TOLERANCE;
    return annotation.strokes.some((stroke) => nearStroke(p, stroke, tolerance));
  }
  return annotation.rects.some((rect) => inside(p, rect));
}

/** The annotation a press at `p` lands on: the one drawn last (on top) first. */
export function annotationAt(
  list: PageAnnotation[],
  p: { x: number; y: number },
): PageAnnotation | null {
  for (let i = list.length - 1; i >= 0; i -= 1) {
    const annotation = list[i];
    if (annotation && hits(annotation, p)) return annotation;
  }
  return null;
}

/** The ink annotations an eraser at `p` touches, by index. */
export function erased(list: PageAnnotation[], p: { x: number; y: number }): number[] {
  return list
    .filter((annotation) => annotation.kind === "ink" && hits(annotation, p))
    .map((annotation) => annotation.index);
}

/** The pen's width at a pressure, as the engine draws it (`Ink::width_at`). */
export function inkWidth(pressure: number): number {
  const width = INK_WIDTH * (0.5 + Math.max(0, Math.min(1, pressure)));
  return Math.round(width * 10) / 10;
}

/**
 * A stroke as SVG paths: one per run of segments of the same width, the way
 * the engine writes its appearance stream, so the preview matches it.
 */
export function inkRuns(points: StrokePoint[]): { d: string; width: number }[] {
  const first = points[0];
  if (!first) return [];
  if (points.length === 1) {
    return [{ d: `M${first.x} ${first.y}L${first.x} ${first.y}`, width: inkWidth(first.pressure) }];
  }
  const runs: { d: string; width: number }[] = [];
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1];
    const b = points[i];
    if (!a || !b) continue;
    const width = inkWidth((a.pressure + b.pressure) / 2);
    const run = runs.at(-1);
    if (run && run.width === width) run.d += `L${b.x} ${b.y}`;
    else runs.push({ d: `M${a.x} ${a.y}L${b.x} ${b.y}`, width });
  }
  return runs;
}

/** A point clamped to a page of `width` × `height` points. */
export function clampToPage(
  x: number,
  y: number,
  width: number,
  height: number,
): { x: number; y: number } {
  return { x: Math.max(0, Math.min(width, x)), y: Math.max(0, Math.min(height, y)) };
}
