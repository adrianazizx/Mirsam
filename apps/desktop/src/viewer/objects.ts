import type { LinkInfo, PageObject, ViewRect } from "../backend";

/**
 * Hit-testing and reshaping for the Edit tool's object and link modes
 * (ADR 0051), in displayed-page points (y down). Pure, so the viewer's
 * gestures can be tested without a page.
 */

/** A corner of a box on the displayed page. */
export type BoxCorner = "nw" | "ne" | "sw" | "se";
export const BOX_CORNERS: readonly BoxCorner[] = ["nw", "ne", "sw", "se"];

/** The smallest side a resize or a crop leaves, in points. */
export const MIN_SIDE = 4;

export function cornerPoint(rect: ViewRect, corner: BoxCorner): { x: number; y: number } {
  return {
    x: corner === "ne" || corner === "se" ? rect.x + rect.width : rect.x,
    y: corner === "sw" || corner === "se" ? rect.y + rect.height : rect.y,
  };
}

function contains(rect: ViewRect, p: { x: number; y: number }, slack: number): boolean {
  return (
    p.x >= rect.x - slack &&
    p.x <= rect.x + rect.width + slack &&
    p.y >= rect.y - slack &&
    p.y <= rect.y + rect.height + slack
  );
}

/**
 * The object a press at `p` takes: the topmost (drawn last) whose box holds
 * it, `slack` points around thin ones (a line has no area).
 */
export function objectAt(
  list: readonly PageObject[],
  p: { x: number; y: number },
  slack: number,
): PageObject | null {
  for (let i = list.length - 1; i >= 0; i -= 1) {
    const object = list[i];
    if (!object) continue;
    const thin = Math.min(object.rect.width, object.rect.height) < slack * 2;
    if (contains(object.rect, p, thin ? slack : 0)) return object;
  }
  return null;
}

/** The link a press at `p` is on: the last one listed whose area holds it. */
export function linkAt(list: readonly LinkInfo[], p: { x: number; y: number }): LinkInfo | null {
  for (let i = list.length - 1; i >= 0; i -= 1) {
    const link = list[i];
    if (link?.rects.some((rect) => contains(rect, p, 0))) return link;
  }
  return null;
}

/** The bounds of a link's areas. */
export function boundsOf(rects: readonly ViewRect[]): ViewRect {
  const x0 = Math.min(...rects.map((r) => r.x));
  const y0 = Math.min(...rects.map((r) => r.y));
  const x1 = Math.max(...rects.map((r) => r.x + r.width));
  const y1 = Math.max(...rects.map((r) => r.y + r.height));
  return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
}

/**
 * The corner handle of `rect` a press at `p` grabs: within `reach` outside
 * the corner, and inside it at most a quarter of the box (so a small box
 * still has a middle to drag it by).
 */
export function cornerAt(
  rect: ViewRect,
  p: { x: number; y: number },
  reach: number,
): BoxCorner | null {
  const inward = Math.min(reach, Math.min(rect.width, rect.height) / 4);
  const near = (offset: number, outward: number) =>
    offset * outward >= 0 ? Math.abs(offset) <= reach : Math.abs(offset) <= inward;
  return (
    BOX_CORNERS.find((corner) => {
      const at = cornerPoint(rect, corner);
      const east = corner === "ne" || corner === "se";
      const south = corner === "sw" || corner === "se";
      return near(p.x - at.x, east ? 1 : -1) && near(p.y - at.y, south ? 1 : -1);
    }) ?? null
  );
}

/**
 * `rect` with `corner` dragged to `p`, the opposite corner kept: freely, or
 * in proportion (`keepAspect`), never smaller than `MIN_SIDE`, and within
 * `bounds` when given (a crop stays inside its image).
 */
export function reshaped(
  rect: ViewRect,
  corner: BoxCorner,
  p: { x: number; y: number },
  keepAspect: boolean,
  bounds: ViewRect | null,
): ViewRect {
  const anchor = cornerPoint(
    rect,
    {
      nw: "se",
      ne: "sw",
      sw: "ne",
      se: "nw",
    }[corner] as BoxCorner,
  );
  const east = corner === "ne" || corner === "se";
  const south = corner === "sw" || corner === "se";
  let x = bounds ? Math.min(Math.max(p.x, bounds.x), bounds.x + bounds.width) : p.x;
  let y = bounds ? Math.min(Math.max(p.y, bounds.y), bounds.y + bounds.height) : p.y;
  let w = Math.max(MIN_SIDE, east ? x - anchor.x : anchor.x - x);
  let h = Math.max(MIN_SIDE, south ? y - anchor.y : anchor.y - y);
  if (keepAspect && rect.width > 0 && rect.height > 0) {
    const scale = Math.max(w / rect.width, h / rect.height);
    w = Math.max(MIN_SIDE, rect.width * scale);
    h = Math.max(MIN_SIDE, rect.height * scale);
  }
  x = east ? anchor.x : anchor.x - w;
  y = south ? anchor.y : anchor.y - h;
  return { x, y, width: w, height: h };
}

/** `rect` moved by (`dx`, `dy`). */
export function moved(rect: ViewRect, dx: number, dy: number): ViewRect {
  return { ...rect, x: rect.x + dx, y: rect.y + dy };
}

/** The rectangle between two points. */
export function spanned(a: { x: number; y: number }, b: { x: number; y: number }): ViewRect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    width: Math.abs(b.x - a.x),
    height: Math.abs(b.y - a.y),
  };
}
