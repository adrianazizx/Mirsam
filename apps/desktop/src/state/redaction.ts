import type {
  RedactHit,
  RedactMarkArg,
  RedactPreset,
  RedactReport,
  SanitizeOptions,
  SanitizeReport,
  ViewRect,
} from "../backend";

/**
 * Redaction state for one document (ADR 0042): the areas marked so far,
 * which wait, visibly, until they are applied; the last search for things to
 * mark; and what the last redaction or sanitizing removed. Marks live only
 * here until applied, so removing one (or undoing it) is free; applying them
 * is the engine's one undo step.
 */

export type MarkSource = "area" | "search" | "selection";

export interface RedactMark {
  id: number;
  page: number;
  /** Boxes on the displayed page, in points. */
  rects: ViewRect[];
  /** The text under it, when it came from a search or a selection. */
  text: string | null;
  source: MarkSource;
}

export interface RedactFind {
  status: "idle" | "searching" | "done" | "error";
  hits: RedactHit[];
  truncated: boolean;
  /** Key under `errors.` when `status` is "error". */
  error: string | null;
  /** Increases with every search; a result for an older one is dropped. */
  serial: number;
}

export interface RedactionState {
  marks: RedactMark[];
  /** Marks taken back by "undo mark", most recent last, for "redo mark". */
  undone: RedactMark[];
  nextId: number;
  /** The text in the search field, and the preset last used (null for typed text). */
  query: string;
  regex: boolean;
  preset: RedactPreset | null;
  find: RedactFind;
  /** What the last apply removed, for the summary under the buttons. */
  applied: RedactReport | null;
  sanitized: SanitizeReport | null;
}

export const IDLE_FIND: RedactFind = {
  status: "idle",
  hits: [],
  truncated: false,
  error: null,
  serial: 0,
};

export const IDLE_REDACTION: RedactionState = {
  marks: [],
  undone: [],
  nextId: 1,
  query: "",
  regex: false,
  preset: null,
  find: IDLE_FIND,
  applied: null,
  sanitized: null,
};

export const DEFAULT_SANITIZE: SanitizeOptions = {
  metadata: true,
  scripts: true,
  attachments: true,
  comments: true,
  hiddenContent: true,
};

/** Rectangles smaller than this (points) on a side are a stray click, not a mark. */
export const MIN_MARK_SIDE = 3;

function same(a: ViewRect[], b: ViewRect[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (r, i) =>
        r.x === b[i]?.x && r.y === b[i]?.y && r.width === b[i]?.width && r.height === b[i]?.height,
    )
  );
}

/** Adds a mark; marking the same boxes twice changes nothing. Marking drops the redo list. */
export function addMark(
  state: RedactionState,
  mark: Omit<RedactMark, "id">,
): { state: RedactionState; added: boolean } {
  const rects = mark.rects.filter((r) => r.width > 0 && r.height > 0);
  if (rects.length === 0) return { state, added: false };
  if (state.marks.some((m) => m.page === mark.page && same(m.rects, rects))) {
    return { state, added: false };
  }
  return {
    state: {
      ...state,
      marks: [...state.marks, { ...mark, rects, id: state.nextId }],
      undone: [],
      nextId: state.nextId + 1,
    },
    added: true,
  };
}

export function removeMark(state: RedactionState, id: number): RedactionState {
  return { ...state, marks: state.marks.filter((m) => m.id !== id) };
}

/** Takes the most recent mark back (kept for redo). */
export function undoMark(state: RedactionState): RedactionState {
  const last = state.marks.at(-1);
  if (!last) return state;
  return { ...state, marks: state.marks.slice(0, -1), undone: [...state.undone, last] };
}

export function redoMark(state: RedactionState): RedactionState {
  const last = state.undone.at(-1);
  if (!last) return state;
  return { ...state, marks: [...state.marks, last], undone: state.undone.slice(0, -1) };
}

/** Whether a hit is already marked (its boxes are). */
export function isMarked(state: RedactionState, hit: RedactHit): boolean {
  return state.marks.some((m) => m.page === hit.page && same(m.rects, hit.rects));
}

/** The marks as the backend takes them. */
export function marksForApply(state: RedactionState): RedactMarkArg[] {
  return state.marks.map((m) => ({ page: m.page, rects: m.rects, text: m.text }));
}

/** How many pages the marks are on. */
export function markedPages(state: RedactionState): number {
  return new Set(state.marks.map((m) => m.page)).size;
}
