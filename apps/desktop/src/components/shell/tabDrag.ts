/**
 * Where a dragged tab lands: the number of other tabs that come before the
 * pointer in reading order. Tabs run left to right in an English UI and
 * right to left in an Arabic one; the DOM order is reading order either way.
 */
export function dropIndex(
  centres: number[],
  draggedIndex: number,
  pointerX: number,
  direction: "ltr" | "rtl",
): number {
  let index = 0;
  centres.forEach((centre, i) => {
    if (i === draggedIndex) return;
    const before = direction === "rtl" ? centre > pointerX : centre < pointerX;
    if (before) index += 1;
  });
  return index;
}

/** Pointer travel before a press on a tab becomes a drag (CSS px). */
export const DRAG_THRESHOLD = 6;
