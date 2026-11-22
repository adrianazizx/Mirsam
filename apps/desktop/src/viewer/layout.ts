import type { PageSize } from "../backend";
import { CSS_PX_PER_POINT, clampZoom } from "../state/zoom";
import { pagePixels } from "./geometry";

/**
 * Page layout for every view mode, in CSS pixels of the scroll content.
 * Pure functions: no DOM, so they are unit-tested directly.
 *
 * - `continuous`: one page per row, every page, scrolling vertically.
 * - `single`: only the current page.
 * - `spread`: two pages per row (1–2, 3–4, …).
 * - `book`: two pages per row with the cover alone (1, 2–3, 4–5, …).
 *
 * `order` places the earlier page of a spread on the left (`ltr`) or on the
 * right (`rtl`, Arabic and Hebrew books). The page area never mirrors with the
 * UI language; only this explicit setting changes it.
 */
export type ViewMode = "continuous" | "single" | "spread" | "book";
export type PageOrder = "ltr" | "rtl";

export interface PageBox {
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Row {
  top: number;
  bottom: number;
  /** Pages in reading order. */
  pages: number[];
}

export interface Layout {
  /** Box of each page, or undefined for pages not laid out (single mode). */
  boxes: (PageBox | undefined)[];
  /** Rows sorted top to bottom. */
  rows: Row[];
  /** Row index of each page, or -1. */
  rowOf: Int32Array;
  width: number;
  height: number;
}

export interface LayoutParams {
  sizes: PageSize[];
  /** Render scale, device px per point (see `geometry.renderScale`). */
  scale: number;
  devicePixelRatio: number;
  mode: ViewMode;
  order: PageOrder;
  /** The page shown in `single` mode. */
  currentPage: number;
  /** Gap between rows, around the content, and between the pages of a spread (CSS px). */
  rowGap: number;
  padding: number;
  spreadGap: number;
  /** Width of the scroll viewport; narrower content is centred in it. */
  viewportWidth: number;
}

/** Pages grouped into rows for a mode, in reading order. */
export function groupRows(pageCount: number, mode: ViewMode, currentPage: number): number[][] {
  if (pageCount <= 0) return [];
  if (mode === "single") return [[Math.min(Math.max(0, currentPage), pageCount - 1)]];
  const rows: number[][] = [];
  if (mode === "continuous") {
    for (let page = 0; page < pageCount; page += 1) rows.push([page]);
    return rows;
  }
  let page = 0;
  if (mode === "book") {
    rows.push([0]);
    page = 1;
  }
  for (; page < pageCount; page += 2) {
    rows.push(page + 1 < pageCount ? [page, page + 1] : [page]);
  }
  return rows;
}

const twoUp = (mode: ViewMode) => mode === "spread" || mode === "book";

/**
 * For a lone page in a two-up row: does it sit in the reading-second slot?
 * A book's cover is a recto (the second slot of its spread); a trailing odd
 * page is a verso (the first slot).
 */
function loneInSecondSlot(mode: ViewMode, rowIndex: number): boolean {
  return mode === "book" && rowIndex === 0;
}

export function computeLayout(params: LayoutParams): Layout {
  const { sizes, scale, devicePixelRatio: dpr, mode, order, rowGap, padding, spreadGap } = params;
  const align = (value: number) => Math.round(value * dpr) / dpr;
  const cssSize = (page: number) => {
    const size = sizes[page] ?? { width: 0, height: 0 };
    return {
      width: pagePixels(size.width, scale) / dpr,
      height: pagePixels(size.height, scale) / dpr,
    };
  };

  const grouped = groupRows(sizes.length, mode, params.currentPage);
  const boxes: (PageBox | undefined)[] = new Array(sizes.length);
  const rowOf = new Int32Array(sizes.length).fill(-1);
  const rows: Row[] = [];

  // Width of each row's content, to size the scroll area before placing rows.
  const rowWidths = grouped.map((pages) => {
    const widths = pages.map((page) => cssSize(page).width);
    if (!twoUp(mode)) return widths[0] ?? 0;
    const [a = 0, b = a] = widths;
    return a + spreadGap + b;
  });
  const contentWidth = Math.max(params.viewportWidth, Math.max(0, ...rowWidths) + 2 * padding);

  let top = padding;
  grouped.forEach((pages, rowIndex) => {
    const dims = pages.map(cssSize);
    const rowHeight = Math.max(0, ...dims.map((dim) => dim.height));
    const rowLeft = (contentWidth - (rowWidths[rowIndex] ?? 0)) / 2;
    const rowTop = align(top);

    // Slots left to right. In a two-up row, slot widths come from the pages;
    // an empty slot takes the lone page's width so the spine stays centred.
    let slots: (number | null)[];
    if (!twoUp(mode)) {
      slots = [pages[0] ?? null];
    } else if (pages.length === 2) {
      slots =
        order === "ltr"
          ? [pages[0] ?? null, pages[1] ?? null]
          : [pages[1] ?? null, pages[0] ?? null];
    } else {
      const second = loneInSecondSlot(mode, rowIndex);
      // Reading-second slot is on the right in LTR and on the left in RTL.
      const onRight = second === (order === "ltr");
      slots = onRight ? [null, pages[0] ?? null] : [pages[0] ?? null, null];
    }
    const loneWidth = dims[0]?.width ?? 0;
    let x = rowLeft;
    for (const page of slots) {
      const dim = page === null ? { width: loneWidth, height: 0 } : cssSize(page);
      if (page !== null) {
        boxes[page] = {
          page,
          x: align(x),
          y: align(rowTop + (rowHeight - dim.height) / 2),
          width: dim.width,
          height: dim.height,
        };
        rowOf[page] = rows.length;
      }
      x += dim.width + spreadGap;
    }
    rows.push({ top: rowTop, bottom: rowTop + rowHeight, pages });
    top = rowTop + rowHeight + rowGap;
  });

  const height = rows.length > 0 ? (rows.at(-1)?.bottom ?? 0) + padding : 0;
  return { boxes, rows, rowOf, width: contentWidth, height };
}

/**
 * The zoom at which a typical row fills `availableWidth` (CSS px): the median
 * row width over the whole document (the lower one for an even count). An
 * occasional landscape page in a portrait document therefore does not shrink
 * every other page; it scrolls sideways instead. Computed over every row, so
 * the zoom stays put while scrolling.
 */
export function fitWidthZoom(
  sizes: PageSize[],
  mode: ViewMode,
  availableWidth: number,
  padding: number,
  spreadGap: number,
): number {
  if (sizes.length === 0 || availableWidth <= 0) return 1;
  const rowPoints: number[] = [];
  for (const pages of groupRows(sizes.length, mode === "single" ? "continuous" : mode, 0)) {
    const widths = pages.map((page) => sizes[page]?.width ?? 0);
    const points = twoUp(mode)
      ? (widths[0] ?? 0) + (widths[1] ?? widths[0] ?? 0)
      : (widths[0] ?? 0);
    if (points > 0) rowPoints.push(points);
  }
  rowPoints.sort((a, b) => a - b);
  const typical = rowPoints[Math.floor((rowPoints.length - 1) / 2)];
  if (typical === undefined) return 1;
  // One pixel of slack: page sizes round up to whole device pixels.
  const usable = availableWidth - 2 * padding - 1 - (twoUp(mode) ? spreadGap : 0);
  const zoom = usable / (typical * CSS_PX_PER_POINT);
  return Number.isFinite(zoom) && zoom > 0 ? clampZoom(zoom) : 1;
}

/** Index of the first row whose bottom is below `y` (binary search). */
export function firstRowBelow(rows: Row[], y: number): number {
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if ((rows[mid]?.bottom ?? 0) <= y) low = mid + 1;
    else high = mid;
  }
  return low;
}

/** Rows intersecting the vertical band [y0, y1). */
export function rowsInBand(rows: Row[], y0: number, y1: number): number[] {
  const out: number[] = [];
  for (let index = firstRowBelow(rows, y0); index < rows.length; index += 1) {
    const row = rows[index];
    if (!row || row.top >= y1) break;
    out.push(index);
  }
  return out;
}

/**
 * The page a reader would call "current": the one with the largest visible
 * area, ties going to the earlier page.
 */
export function currentPageIn(
  layout: Layout,
  view: { x0: number; y0: number; x1: number; y1: number },
): number | null {
  let best: number | null = null;
  let bestArea = 0;
  // Rows run top to bottom and pages within a row in reading order, so pages
  // arrive in ascending order and a strict comparison keeps the earlier one on
  // a tie (half a square pixel of slack absorbs rounding).
  for (const rowIndex of rowsInBand(layout.rows, view.y0, view.y1)) {
    for (const page of layout.rows[rowIndex]?.pages ?? []) {
      const box = layout.boxes[page];
      if (!box) continue;
      const w = Math.min(view.x1, box.x + box.width) - Math.max(view.x0, box.x);
      const h = Math.min(view.y1, box.y + box.height) - Math.max(view.y0, box.y);
      const area = w > 0 && h > 0 ? w * h : 0;
      if (area > bestArea + 0.5) {
        best = page;
        bestArea = area;
      }
    }
  }
  return best;
}

/** A content point expressed relative to a page, so it survives a relayout. */
export interface Anchor {
  page: number;
  /** Fractions of the page box; may fall outside [0, 1] in gaps. */
  fx: number;
  fy: number;
}

/** The anchor for content point (x, y): the page in the nearest row, nearest horizontally. */
export function anchorAt(layout: Layout, x: number, y: number): Anchor | null {
  if (layout.rows.length === 0) return null;
  let rowIndex = Math.min(firstRowBelow(layout.rows, y), layout.rows.length - 1);
  // In a gap above row `rowIndex`, the previous row may be nearer.
  const row = layout.rows[rowIndex];
  const previous = layout.rows[rowIndex - 1];
  if (row && previous && y < row.top && y - previous.bottom < row.top - y) rowIndex -= 1;
  let best: PageBox | undefined;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const page of layout.rows[rowIndex]?.pages ?? []) {
    const box = layout.boxes[page];
    if (!box) continue;
    const distance = x < box.x ? box.x - x : x > box.x + box.width ? x - box.x - box.width : 0;
    if (distance < bestDistance) {
      best = box;
      bestDistance = distance;
    }
  }
  if (!best || best.width <= 0 || best.height <= 0) return null;
  return { page: best.page, fx: (x - best.x) / best.width, fy: (y - best.y) / best.height };
}

/** Where an anchor's point lies in a (new) layout. */
export function resolveAnchor(layout: Layout, anchor: Anchor): { x: number; y: number } | null {
  const box = layout.boxes[anchor.page];
  if (!box) return null;
  return { x: box.x + anchor.fx * box.width, y: box.y + anchor.fy * box.height };
}
