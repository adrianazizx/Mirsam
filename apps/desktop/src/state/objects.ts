import type { LinkInfo, LinkTarget, PageObject, ViewRect } from "../backend";

/**
 * What a press on a page does with the Edit tool (ADR 0021, ADR 0051): place
 * or edit text, select and change page objects, or add and edit links.
 */
export type EditMode = "text" | "objects" | "links";

/** The page object selected with the Edit tool's object mode. */
export interface SelectedObject {
  documentId: number;
  page: number;
  object: PageObject;
}

/**
 * A link being added (`index` null: over `rects`, drawn or selected) or
 * edited (its index in the page's annotations), and where it should go.
 */
export interface LinkDraft {
  documentId: number;
  page: number;
  index: number | null;
  rects: ViewRect[];
  target: LinkTarget;
}

/** A link the pointer is over, or one pressed to follow (`pinned`, ADR 0051). */
export interface LinkTip {
  documentId: number;
  page: number;
  link: LinkInfo;
  pinned: boolean;
}

/** A web address as the engine will write it: `https://` for a bare domain, `mailto:` for an e-mail address. */
export function normaliseUrl(input: string): string {
  const text = input.trim();
  if (/^(https?:\/\/|mailto:)/i.test(text) || text.includes(":")) return text;
  if (text.includes("@") && !text.includes("/")) return `mailto:${text}`;
  return `https://${text}`;
}

/**
 * Whether a link may be opened by this app (the shell checks again): web and
 * mail only, nothing a launcher reads specially; letters outside ASCII are
 * percent-encoded by the shell.
 */
export function isOpenable(target: LinkTarget): target is { kind: "url"; url: string } {
  return (
    target.kind === "url" &&
    /^(https?:\/\/|mailto:)/i.test(target.url) &&
    !/[\s\p{Cc}"<>\\^`{|}]/u.test(target.url)
  );
}

/** The displayed-page map that takes `from` onto `to` (a move or a resize). */
export function mapBetween(
  from: ViewRect,
  to: ViewRect,
): [number, number, number, number, number, number] {
  const sx = from.width > 0 ? to.width / from.width : 1;
  const sy = from.height > 0 ? to.height / from.height : 1;
  return [sx, 0, 0, sy, to.x - from.x * sx, to.y - from.y * sy];
}

/** A quarter turn of the displayed page about `rect`'s centre: clockwise, or counterclockwise. */
export function quarterTurn(
  rect: ViewRect,
  clockwise: boolean,
): [number, number, number, number, number, number] {
  const cx = rect.x + rect.width / 2;
  const cy = rect.y + rect.height / 2;
  // y runs down the displayed page, so (x, y) → (−y, x) turns clockwise.
  return clockwise ? [0, 1, -1, 0, cx + cy, cy - cx] : [0, -1, 1, 0, cx - cy, cx + cy];
}
