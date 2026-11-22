import type { AnnotationKind, DocumentComment } from "../backend";

/** The comments panel's filters (ADR 0041). */
export type CommentStatus = "all" | "open" | "resolved";
export type CommentKindFilter = "all" | "markup" | "notes" | "textBoxes" | "ink";
export type CommentSort = "page" | "newest" | "author";

export interface CommentFilter {
  status: CommentStatus;
  kind: CommentKindFilter;
  /** One author's comments ("" for those without a name), or everyone's (null). */
  author: string | null;
}

export const DEFAULT_COMMENT_FILTER: CommentFilter = { status: "all", kind: "all", author: null };

/** The longest name the app writes on comments (the engine allows 256). */
export const MAX_AUTHOR = 100;

const KIND_GROUP: Record<AnnotationKind, Exclude<CommentKindFilter, "all">> = {
  highlight: "markup",
  underline: "markup",
  strikeOut: "markup",
  squiggly: "markup",
  ink: "ink",
  note: "notes",
  textBox: "textBoxes",
};

export function filterComments(list: DocumentComment[], filter: CommentFilter): DocumentComment[] {
  return list.filter(
    (comment) =>
      (filter.status === "all" || (filter.status === "resolved") === comment.resolved) &&
      (filter.kind === "all" || KIND_GROUP[comment.kind] === filter.kind) &&
      (filter.author === null || (comment.author ?? "") === filter.author),
  );
}

/** A thread's latest activity: the comment or its newest reply. */
export function latestActivity(comment: DocumentComment): number | null {
  const times = [comment.modified, ...comment.replies.map((reply) => reply.modified)].filter(
    (time): time is number => time !== null,
  );
  return times.length === 0 ? null : Math.max(...times);
}

/** Top to bottom on its page, pages in order. */
function byPlace(a: DocumentComment, b: DocumentComment): number {
  return a.page - b.page || a.rect.y - b.rect.y || a.index - b.index;
}

/**
 * Comments in page order (top to bottom), by latest activity (undated ones
 * last), or by author in the UI language's collation (unnamed ones last).
 */
export function sortComments(
  list: DocumentComment[],
  sort: CommentSort,
  collator: Intl.Collator,
): DocumentComment[] {
  const sorted = [...list];
  if (sort === "page") return sorted.sort(byPlace);
  if (sort === "newest") {
    const time = (comment: DocumentComment) => latestActivity(comment) ?? Number.NEGATIVE_INFINITY;
    return sorted.sort((a, b) => time(b) - time(a) || byPlace(a, b));
  }
  return sorted.sort((a, b) => {
    if ((a.author === null) !== (b.author === null)) return a.author === null ? 1 : -1;
    return collator.compare(a.author ?? "", b.author ?? "") || byPlace(a, b);
  });
}

/** The comments' distinct authors in collation order; "" (no name) last, when any. */
export function authorsOf(list: DocumentComment[], collator: Intl.Collator): string[] {
  const names = new Set(list.map((comment) => comment.author ?? ""));
  const named = [...names].filter((name) => name !== "").sort(collator.compare);
  return names.has("") ? [...named, ""] : named;
}
