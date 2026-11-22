import type {
  AnnotationKind,
  Backend,
  CommentReply,
  DocumentComment,
  DocumentInfo,
  EditOutcome,
  KeptRequest,
  Lane,
  OpenOutcome,
  OutlineItem,
  PageEdit,
  PageSize,
  ParagraphBlock,
  Priority,
  RecentFile,
  RecoveryInfo,
  RedactHit,
  RedactPreset,
  RenderedPage,
  SearchHit,
  SearchOptions,
  TextLayer,
  TextLine,
  TileRequest,
  ViewRect,
} from "./types";

/**
 * In-browser stand-in for the Rust shell, used when the UI runs outside Tauri
 * (plain `pnpm dev`, component tests). It draws placeholder pages with the 2D
 * canvas so layout, theming, scrolling, and RTL work can be reviewed in any
 * browser. Nothing here is PDF rendering, and its text geometry is an
 * approximation (each line is treated as one direction run).
 *
 * Like the engine, it renders one request at a time, most urgent first, and
 * honours viewport generations, so the viewer's cancellation path runs here too.
 */

const A4: PageSize = { width: Math.fround(595.276), height: Math.fround(841.89) };
const A4_LANDSCAPE: PageSize = { width: A4.height, height: A4.width };

const SAMPLE: DocumentInfo = { id: 1, name: "عقد إيجار — Lease Agreement.pdf", pageCount: 240 };
/** A recent file that is password-protected; its password is "1234". */
const ALL_PERMISSIONS = {
  print: true,
  printHighQuality: true,
  modify: true,
  copy: true,
  annotate: true,
  fillForms: true,
  accessibility: true,
  assemble: true,
};
const LOCKED: DocumentInfo = { id: 2, name: "سري — Confidential.pdf", pageCount: 12 };
const LOCKED_PASSWORD = "1234";

/** Every 17th page is landscape, to exercise mixed page sizes. */
const sizeOf = (page: number): PageSize => (page % 17 === 9 ? A4_LANDSCAPE : A4);

const PAGES: { direction: "ltr" | "rtl"; heading: string; lines: string[] }[] = [
  {
    direction: "rtl",
    heading: "عقد إيجار",
    lines: [
      "أُبرم هذا العقد بتاريخ 12 مارس 2026 بين الطرفين المذكورين أدناه.",
      "يلتزم المستأجر بدفع مبلغ 4,500 ريال شهريًا لمدة 12 شهرًا.",
      "رقم العقد: SA-2026-00417 (نسخة المستأجر).",
    ],
  },
  {
    direction: "ltr",
    heading: "Lease Agreement",
    lines: [
      "This agreement is made on 12 March 2026 between the parties named below.",
      "The tenant shall pay SAR 4,500 per month for a term of 12 months.",
      "Contract no. SA-2026-00417 (tenant copy).",
    ],
  },
  {
    direction: "rtl",
    heading: "الملحق (Appendix A)",
    lines: ["جدول الدفعات — Payment schedule", "الدفعة 1: 4,500 ريال", "الدفعة 2: 4,500 ريال"],
  },
];

const MARGIN = 72;
const FONT = `"IBM Plex Sans", "IBM Plex Sans Arabic", sans-serif`;

interface MockLine {
  text: string;
  size: number;
  weight: number;
  baseline: number;
}

function pageContent(page: number): { direction: "ltr" | "rtl"; lines: MockLine[] } {
  const content = PAGES[page] ?? {
    direction: page % 2 === 0 ? "rtl" : "ltr",
    heading: page % 2 === 0 ? `صفحة ${page + 1}` : `Page ${page + 1}`,
    lines: [],
  };
  return {
    direction: content.direction,
    lines: [
      { text: content.heading, size: 22, weight: 600, baseline: MARGIN + 22 },
      ...content.lines.map((text, index) => ({
        text,
        size: 11,
        weight: 400,
        baseline: MARGIN + 64 + index * 22,
      })),
    ],
  };
}

let measurer: CanvasRenderingContext2D | null | undefined;
/** Advance of `text` in points; an estimate where no canvas exists (jsdom). */
function measure(text: string, line: MockLine): number {
  if (measurer === undefined) {
    try {
      measurer = document.createElement("canvas").getContext("2d");
    } catch {
      measurer = null;
    }
  }
  if (!measurer) return text.length * line.size * 0.5;
  measurer.font = `${line.weight} ${line.size}px ${FONT}`;
  return measurer.measureText(text).width;
}

/** The mock's text layer: logical-order lines with estimated character boxes. */
function textLayer(page: number): TextLayer {
  const size = sizeOf(page);
  const { direction, lines } = pageContent(page);
  const rtl = direction === "rtl";
  const textLines: TextLine[] = lines.map((line) => {
    const chars = Array.from(line.text);
    const advances = chars.map((char) => measure(char, line));
    const natural = advances.reduce((sum, value) => sum + value, 0);
    const width = measure(line.text, line);
    const factor = natural > 0 ? width / natural : 1;
    const top = line.baseline - line.size * 0.8;
    let offset = 0;
    let index = 0;
    const spans = chars.map((char, i) => {
      const advance = (advances[i] ?? 0) * factor;
      const x = rtl ? size.width - MARGIN - offset - advance : MARGIN + offset;
      offset += advance;
      const start = index;
      index += char.length;
      return {
        start,
        end: index,
        rect: char === " " ? null : { x, y: top, width: advance, height: line.size },
      };
    });
    const x0 = rtl ? size.width - MARGIN - width : MARGIN;
    return { text: line.text, spans, bounds: { x: x0, y: top, width, height: line.size } };
  });
  return {
    direction,
    quality: "good",
    blocks: textLines.map((line) => ({ direction, lines: [line] })),
  };
}

const TASHKEEL = /[ؐ-ًؚ-ٰٟـ]/g;
function fold(text: string, options: SearchOptions): string {
  let folded = options.caseSensitive ? text : text.toLowerCase();
  if (options.arabicNormalize) {
    // Same length as the input, so indices carry over: marks become a
    // character that never matches instead of being removed.
    folded = folded
      .replace(TASHKEEL, "\uFFFF")
      .replace(/[أإآٱ]/g, "ا")
      .replace(/ى/g, "ي")
      .replace(/[٠-٩]/g, (digit) => String(digit.charCodeAt(0) - 0x0660));
    if (options.taMarbutaAsHa) folded = folded.replace(/ة/g, "ه");
  }
  return folded;
}

/** An annotation the mock keeps, in displayed-page points of its page. */
interface MockMarkup {
  kind: AnnotationKind;
  rects: ViewRect[];
  contents: string | null;
  strokes: [number, number][][];
  /** Its thread (ADR 0041): who and when, replies, and whether it is resolved. */
  author?: string | null;
  modified?: number;
  replies?: CommentReply[];
  resolved?: boolean;
}

/** Who and when, for a new annotation or reply: the `commentAuthor` setting, now. */
function signed(): { author: string | null; modified: number } {
  const author = (loadSettings().commentAuthor ?? "").trim();
  return { author: author === "" ? null : author, modified: Math.floor(Date.now() / 1000) };
}

/** Changes the comment at `index` of `page`, as one undoable step. */
function withComment(
  documentId: number,
  page: number,
  index: number,
  change: (comment: MockMarkup) => MockMarkup,
): EditOutcome {
  const next = withMarkup(documentId, page, (list) => {
    const found = list[index];
    if (!found) throw { code: "invalid_edit", detail: "no such annotation" };
    return list.map((item, i) => (i === index ? change(item) : item));
  });
  return pushState(documentId, next);
}

/**
 * One revision of a mock document: its pages as original page numbers (-1:
 * blank), the markup on each original page, and what the step to it changed.
 */
interface MockState {
  pages: number[];
  markup: Map<number, MockMarkup[]>;
  change: "pages" | "annotations";
}

/** Each edited mock document's revisions, with undo and redo. */
const edits = new Map<number, { states: MockState[]; at: number; saved: number }>();

function stateOf(documentId: number): MockState {
  const history = edits.get(documentId);
  return (
    history?.states[history.at] ?? {
      pages: Array.from({ length: basePageCount(documentId) }, (_, i) => i),
      markup: new Map(),
      change: "pages",
    }
  );
}

/** Records the next revision; undone revisions are forgotten. */
const MOCK_BLOCK_TEXT = "The mock page has one paragraph of text that the Edit tool can change.";
const mockBlockTexts = new Map<number, string>();

/** The watermark image the mock last "picked". */
let mockWatermarkImage: string | null = null;
/** Documents with a compressed copy waiting to be saved. */
const mockCompressed = new Set<number>();

function pushState(documentId: number, next: MockState): EditOutcome {
  const history = edits.get(documentId) ?? { states: [stateOf(documentId)], at: 0, saved: 0 };
  history.states = [...history.states.slice(0, history.at + 1), next];
  history.at += 1;
  edits.set(documentId, history);
  return outcome(documentId, next.change);
}

/** Markup on a page (by its original page number), copied before changing. */
function withMarkup(
  documentId: number,
  page: number,
  change: (list: MockMarkup[]) => MockMarkup[],
): MockState {
  const state = stateOf(documentId);
  const original = state.pages[page];
  if (original === undefined) throw { code: "page_out_of_range", detail: "page out of range" };
  const markup = new Map(state.markup);
  markup.set(original, change([...(markup.get(original) ?? [])]));
  return { pages: state.pages, markup, change: "annotations" };
}

function basePageCount(documentId: number): number {
  return documentId === LOCKED.id ? LOCKED.pageCount : SAMPLE.pageCount;
}

/** The document's current pages as original page numbers. */
function pagesOf(documentId: number): number[] {
  return stateOf(documentId).pages;
}

function applyEdit(pages: number[], edit: PageEdit): number[] {
  const chosen = new Set("pages" in edit ? edit.pages : []);
  if ([...chosen].some((page) => page < 0 || page >= pages.length)) {
    throw { code: "page_out_of_range", detail: "page out of range" };
  }
  switch (edit.kind) {
    case "rotate":
      return pages;
    case "delete": {
      const kept = pages.filter((_, i) => !chosen.has(i));
      if (kept.length === 0) throw { code: "invalid_edit", detail: "a document keeps one page" };
      return kept;
    }
    case "move": {
      const moved = pages.filter((_, i) => chosen.has(i));
      const rest = pages.filter((_, i) => !chosen.has(i));
      const at = Math.min(edit.to, rest.length);
      return [...rest.slice(0, at), ...moved, ...rest.slice(at)];
    }
    case "insertBlank":
      return [...pages.slice(0, edit.at), -1, ...pages.slice(edit.at)];
    case "duplicate":
      return pages.flatMap((page, i) => (chosen.has(i) ? [page, page] : [page]));
  }
}

function outcome(documentId: number, change: MockState["change"]): EditOutcome {
  const history = edits.get(documentId);
  return {
    pageCount: pagesOf(documentId).length,
    canUndo: (history?.at ?? 0) > 0,
    canRedo: history ? history.at < history.states.length - 1 : false,
    modified: history ? history.at !== history.saved : false,
    pagesChanged: change === "pages",
  };
}

/** The presets as the engine runs them, on text whose Arabic-Indic digits are Western. */
const PRESET_PATTERNS: Record<RedactPreset, RegExp> = {
  email: /[\p{L}\p{N}._%+-]+@[\p{L}\p{N}-]+(?:\.[\p{L}\p{N}-]+)*\.\p{L}{2,}/gu,
  phone: /(?:\+|\b00)?[0-9][0-9 ().-]{6,18}[0-9]/gu,
  iban: /\b[a-z]{2}[0-9]{2}(?: ?[a-z0-9]{4}){2,7}(?: ?[a-z0-9]{1,3})?\b/giu,
  idNumber: /\b[0-9]{9,14}\b/gu,
  cardNumber: /\b(?:[0-9][ -]?){12,18}[0-9]\b/gu,
};

/** Finds a preset's matches line by line (the mock has no real text; the shape is the engine's). */
function findPreset(documentId: number, preset: RedactPreset): RedactHit[] {
  const hits: RedactHit[] = [];
  const pageCount = pagesOf(documentId).length;
  for (let page = 0; page < pageCount; page += 1) {
    for (const block of textLayer(page).blocks) {
      for (const line of block.lines) {
        // Arabic-Indic and Persian digits are folded one for one, so indices still line up.
        const folded = line.text
          .replace(/[\u0660-\u0669]/g, (c) => String(c.charCodeAt(0) - 0x0660))
          .replace(/[\u06f0-\u06f9]/g, (c) => String(c.charCodeAt(0) - 0x06f0));
        for (const match of folded.matchAll(PRESET_PATTERNS[preset])) {
          const start = match.index ?? 0;
          const end = start + match[0].length;
          const rects = line.spans
            .filter((span) => span.rect && span.start < end && start < span.end)
            .map((span) => span.rect)
            .filter((rect) => rect !== null);
          hits.push({
            page,
            text: line.text.slice(start, end),
            before: line.text.slice(Math.max(0, start - 24), start),
            after: line.text.slice(end, end + 24),
            rects,
          });
        }
      }
    }
  }
  return hits;
}

function search(documentId: number, query: string, options: SearchOptions): SearchHit[] {
  const needle = fold(query, options)
    .replace(/\uFFFF/g, "")
    .trim();
  if (!needle) throw { code: "invalid_query", detail: "empty" };
  const pageCount = pagesOf(documentId).length;
  const hits: SearchHit[] = [];
  for (let page = 0; page < pageCount; page += 1) {
    const layer = textLayer(page);
    layer.blocks.forEach((block, b) => {
      block.lines.forEach((line, l) => {
        const haystack = fold(line.text, options).replace(/\uFFFF/g, "");
        // Indices into the folded text without marks; map back by walking the original.
        const keep: number[] = [];
        Array.from(fold(line.text, options)).forEach((char, i) => {
          if (char !== "\uFFFF") keep.push(i);
        });
        let from = 0;
        for (;;) {
          const at = haystack.indexOf(needle, from);
          if (at < 0) break;
          from = at + needle.length;
          const start = keep[at] ?? 0;
          const end = (keep[at + needle.length - 1] ?? start) + 1;
          const rects = line.spans
            .filter((span) => span.rect && span.start < end && start < span.end)
            .map((span) => span.rect)
            .filter((rect) => rect !== null);
          hits.push({
            page,
            block: b,
            line: l,
            start,
            end,
            direction: block.direction,
            rects,
            context: {
              before: line.text.slice(Math.max(0, start - 24), start),
              matched: line.text.slice(start, end),
              after: line.text.slice(end, end + 24),
            },
          });
        }
      });
    });
  }
  return hits;
}

const OUTLINE: OutlineItem[] = [
  {
    title: "عقد إيجار",
    page: 0,
    top: null,
    children: [
      { title: "Lease Agreement", page: 1, top: null, children: [] },
      { title: "الملحق (Appendix A)", page: 2, top: null, children: [] },
    ],
  },
  {
    title: "الشروط العامة — General terms",
    page: 9,
    top: null,
    children: [
      { title: "المادة 1: مدة العقد", page: 10, top: null, children: [] },
      { title: "Article 2: Payment", page: 14, top: null, children: [] },
    ],
  },
  { title: "Signatures — التوقيعات", page: 239, top: null, children: [] },
];

/** jsdom has no ImageData; tests only need the shape. */
function blankImage(width: number, height: number): ImageData {
  const data = new Uint8ClampedArray(width * height * 4).fill(255);
  return typeof ImageData === "function"
    ? new ImageData(data, width, height)
    : ({ width, height, data, colorSpace: "srgb" } as ImageData);
}

function blank(width: number, height: number): RenderedPage {
  return { width, height, renderMicros: 0, fromCache: false, image: blankImage(width, height) };
}

/** Annotations over a page, drawn like the engine's appearance streams. */
function drawMarkup(ctx: CanvasRenderingContext2D, markup: MockMarkup[]) {
  const colours: Record<AnnotationKind, string> = {
    highlight: "rgb(255 219 0)",
    underline: "rgb(0 107 230)",
    squiggly: "rgb(0 107 230)",
    strikeOut: "rgb(219 33 33)",
    ink: "rgb(13 56 153)",
    note: "rgb(255 219 0)",
    textBox: "rgb(0 0 0)",
  };
  for (const { kind, rects, strokes, contents } of markup) {
    if (kind === "textBox") {
      ctx.globalCompositeOperation = "source-over";
      ctx.fillStyle = colours.textBox;
      ctx.font = `400 12px ${FONT}`;
      ctx.direction = /[\u0600-\u06FF]/.test(contents?.[0] ?? "") ? "rtl" : "ltr";
      for (const rect of rects) {
        ctx.textAlign = ctx.direction === "rtl" ? "right" : "left";
        const x = ctx.direction === "rtl" ? rect.x + rect.width - 2 : rect.x + 2;
        ctx.fillText(contents ?? "", x, rect.y + 14, rect.width - 4);
      }
      ctx.direction = "ltr";
      ctx.textAlign = "start";
      continue;
    }
    if (kind === "ink" || kind === "note") {
      ctx.globalCompositeOperation = "source-over";
      ctx.strokeStyle = kind === "ink" ? colours.ink : "rgb(64 64 64)";
      ctx.fillStyle = colours[kind];
      ctx.lineWidth = kind === "ink" ? 2 : 0.6;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      if (kind === "note") {
        for (const rect of rects) {
          ctx.fillRect(rect.x + 1.5, rect.y + 1.5, rect.width - 3, rect.height - 7);
          ctx.strokeRect(rect.x + 1.5, rect.y + 1.5, rect.width - 3, rect.height - 7);
        }
      }
      for (const stroke of strokes) {
        ctx.beginPath();
        for (const [x, y] of stroke) ctx.lineTo(x, y);
        ctx.stroke();
      }
      continue;
    }
    ctx.fillStyle = colours[kind];
    ctx.strokeStyle = colours[kind];
    ctx.globalCompositeOperation = kind === "highlight" ? "multiply" : "source-over";
    for (const rect of rects) {
      const width = Math.max(0.5, rect.height * 0.07);
      const bottom = rect.y + rect.height - width;
      ctx.lineWidth = width;
      ctx.beginPath();
      if (kind === "highlight") {
        ctx.fillRect(rect.x, rect.y, rect.width, rect.height);
        continue;
      }
      if (kind === "squiggly") {
        const step = Math.max(1, rect.height / 8);
        for (let x = 0, i = 0; x <= rect.width; x += step, i += 1) {
          ctx.lineTo(rect.x + x, bottom + (i % 2 === 0 ? step / 2 : -step / 2));
        }
      } else {
        const y = kind === "underline" ? bottom : rect.y + rect.height / 2;
        ctx.moveTo(rect.x, y);
        ctx.lineTo(rect.x + rect.width, y);
      }
      ctx.stroke();
    }
  }
  ctx.globalCompositeOperation = "source-over";
}

function drawTile(page: number, tile: TileRequest, markup: MockMarkup[] = []): RenderedPage {
  const { width, height, scale } = tile;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  // jsdom implements no canvas; tests get blank tiles.
  let ctx: CanvasRenderingContext2D | null = null;
  try {
    ctx = canvas.getContext("2d");
  } catch {
    return blank(width, height);
  }
  if (!ctx) return blank(width, height);

  const size = sizeOf(page);
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, width, height);
  // Draw in page points; the transform selects this tile's window of the page.
  ctx.setTransform(scale, 0, 0, scale, -tile.x, -tile.y);
  const content = pageContent(page);
  const rtl = content.direction === "rtl";
  const x = rtl ? size.width - MARGIN : MARGIN;
  ctx.fillStyle = "#1b1f24";
  ctx.direction = content.direction;
  ctx.textAlign = "start";
  for (const line of content.lines) {
    ctx.font = `${line.weight} ${line.size}px ${FONT}`;
    ctx.fillText(line.text, x, line.baseline);
  }
  // Grey bars stand in for body text, ragged on the side text ends on.
  ctx.fillStyle = "#c9ced6";
  const textWidth = size.width - 2 * MARGIN;
  for (let line = 0; line < 30; line += 1) {
    const y = MARGIN + 140 + line * 18;
    if (y > size.height - MARGIN - 30) break;
    const length = textWidth * (line % 6 === 5 ? 0.55 : 0.92 + ((line * 37) % 8) / 100);
    const barWidth = Math.min(textWidth, length);
    ctx.fillRect(rtl ? size.width - MARGIN - barWidth : MARGIN, y, barWidth, 7);
  }
  ctx.fillStyle = "#57606a";
  ctx.textAlign = "center";
  ctx.direction = "ltr";
  ctx.font = `400 11px ${FONT}`;
  ctx.fillText(`— ${page + 1} —`, size.width / 2, size.height - MARGIN / 2);
  drawMarkup(ctx, markup);

  return {
    width,
    height,
    renderMicros: 0,
    fromCache: false,
    image: ctx.getImageData(0, 0, width, height),
  };
}

interface Job {
  lane: Lane;
  generation: number;
  page: number;
  /** The page's original content after edits (-1: a blank page). */
  original: number;
  /** The markup on it when the render was asked for. */
  markup: MockMarkup[];
  tile: TileRequest;
  priority: Priority;
  seq: number;
  resolve(page: RenderedPage): void;
  reject(error: unknown): void;
}

const URGENCY = { placeholder: 0, visible: 1, thumbnail: 2, prefetch: 3 } as const;
const CANCELLED = { code: "cancelled", detail: "the viewport moved on" };
const sameTile = (a: TileRequest, b: TileRequest) =>
  a.scale === b.scale && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;

const queue: Job[] = [];
const generations: Record<Lane, number> = { main: 0, thumbnails: 0 };
let seq = 0;
let busy = false;

function pump() {
  if (busy || queue.length === 0) return;
  queue.sort(
    (a, b) =>
      URGENCY[a.priority.urgency] - URGENCY[b.priority.urgency] ||
      a.priority.distance - b.priority.distance ||
      a.seq - b.seq,
  );
  const job = queue.shift();
  if (!job) return;
  busy = true;
  setTimeout(() => {
    busy = false;
    job.resolve(
      job.original < 0
        ? blank(job.tile.width, job.tile.height)
        : drawTile(job.original, job.tile, job.markup),
    );
    pump();
  }, 1);
}

function updateViewport(lane: Lane, next: number, keep: KeptRequest[]) {
  if (next < generations[lane]) return;
  generations[lane] = next;
  for (let index = queue.length - 1; index >= 0; index -= 1) {
    const job = queue[index];
    if (!job || job.lane !== lane) continue;
    const kept = keep.find((k) => k.page === job.page && sameTile(k.tile, job.tile));
    if (kept) {
      job.priority = kept.priority;
      job.generation = Math.max(job.generation, next);
    } else if (job.generation < next) {
      queue.splice(index, 1);
      job.reject(CANCELLED);
    }
  }
}

const SETTINGS_KEY = "safha.mock.settings";
function loadSettings(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "{}") as Record<string, string>;
  } catch {
    return {};
  }
}

let recents: RecentFile[] = [
  { token: 1, name: SAMPLE.name, lastOpened: 1_795_000_000, exists: true },
  { token: 2, name: LOCKED.name, lastOpened: 1_794_900_000, exists: true },
  { token: 3, name: "old-report.pdf", lastOpened: 1_790_000_000, exists: false },
];

const opened = (document: DocumentInfo): OpenOutcome => ({ kind: "opened", document });

/** Whether the file the mock's insert "picks" needs a password; tests set it. */
let mockInsertIsLocked = false;
let pendingInsert: { documentId: number; at: number } | null = null;
export function setMockInsertIsLocked(locked: boolean): void {
  mockInsertIsLocked = locked;
  pendingInsert = null;
}

/** Inserts the sample's first three pages before page `at`. */
function insertSample(documentId: number, at: number): EditOutcome {
  const state = stateOf(documentId);
  const pages = [...state.pages.slice(0, at), 0, 1, 2, ...state.pages.slice(at)];
  return pushState(documentId, { pages, markup: state.markup, change: "pages" });
}

/** Recovery files the mock's startup offers; tests set them. */
let mockRecoveries: RecoveryInfo[] = [];
export function setMockRecoveries(list: RecoveryInfo[]): void {
  mockRecoveries = list;
}

export const mockBackend: Backend = {
  appInfo: async () => ({ version: "0.0.1-mock", devtools: false }),
  openDocumentDialog: async () => opened(SAMPLE),
  startup: async () => ({ documents: [], active: null, recoveries: [...mockRecoveries] }),
  // A restored recovery file reopens with unsaved changes; the locked sample's asks first.
  restoreRecovery: async (token) => {
    const entry = mockRecoveries.find((candidate) => candidate.token === token);
    mockRecoveries = mockRecoveries.filter((candidate) => candidate.token !== token);
    if (entry?.name === LOCKED.name) {
      return {
        outcome: { kind: "needsPassword", token: 2, name: LOCKED.name, retry: false },
        replaced: null,
      };
    }
    const canUndo = !entry?.originalChanged;
    return { outcome: opened({ ...SAMPLE, modified: true, canUndo }), replaced: null };
  },
  unlockRecovery: async (_token, password) => ({
    outcome:
      password === LOCKED_PASSWORD
        ? opened({ ...LOCKED, modified: true, canUndo: true })
        : { kind: "needsPassword", token: 2, name: LOCKED.name, retry: true },
    replaced: null,
  }),
  discardRecovery: async (token) => {
    mockRecoveries = mockRecoveries.filter((entry) => entry.token !== token);
  },
  hasRecoveryFile: async () => null,
  unlockDocument: async (_token, password) =>
    password === LOCKED_PASSWORD
      ? opened(LOCKED)
      : { kind: "needsPassword", token: 2, name: LOCKED.name, retry: true },
  cancelUnlock: async () => {},
  recentFiles: async (limit) => recents.slice(0, limit),
  openRecent: async (token) => {
    const recent = recents.find((candidate) => candidate.token === token);
    if (!recent?.exists) throw { code: "file_missing", detail: "gone" };
    return token === 2
      ? { kind: "needsPassword", token: 2, name: LOCKED.name, retry: false }
      : opened(SAMPLE);
  },
  forgetRecent: async (token) => {
    recents = recents.filter((recent) => recent.token !== token);
  },
  clearRecents: async () => {
    recents = [];
  },
  saveViewState: async () => {},
  saveSession: async () => {},
  settings: async () => loadSettings(),
  setSetting: async (key, value) => {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify({ ...loadSettings(), [key]: value }));
    } catch {
      // Storage unavailable: settings last one session.
    }
  },
  pageSizes: async (documentId) =>
    pagesOf(documentId).map((original) =>
      original < 0 ? { width: 595, height: 842 } : sizeOf(original),
    ),
  editPages: async (documentId, edit) => {
    const state = stateOf(documentId);
    return pushState(documentId, {
      pages: applyEdit(state.pages, edit),
      markup: state.markup,
      change: "pages",
    });
  },
  undoEdit: async (documentId) => {
    const history = edits.get(documentId);
    if (!history || history.at === 0) return outcome(documentId, "annotations");
    const undone = history.states[history.at]?.change ?? "pages";
    history.at -= 1;
    return outcome(documentId, undone);
  },
  findRedactions: async (documentId, pattern) => {
    if (pattern.kind === "preset")
      return { hits: findPreset(documentId, pattern.preset), truncated: false };
    const hits = search(documentId, pattern.query, pattern.options).map((hit) => ({
      page: hit.page,
      text: hit.context.matched,
      before: hit.context.before,
      after: hit.context.after,
      rects: hit.rects,
    }));
    return { hits, truncated: false };
  },
  applyRedactions: async (documentId, marks) => {
    if (marks.length === 0) throw { code: "invalid_edit", detail: "no areas to redact" };
    const state = stateOf(documentId);
    // The mock draws no pages' content, so there is nothing to remove: it only takes the step.
    const edit = pushState(documentId, { ...state, change: "pages" });
    const report = {
      pages: new Set(marks.map((mark) => mark.page)).size,
      counts: { glyphs: 0, imagesBlanked: 0, imagesDropped: 0, paths: 0, hidden: 0 },
      annotations: 0,
      strings: 0,
    };
    return { edit, report };
  },
  sanitizeDocument: async (documentId) => {
    const edit = pushState(documentId, { ...stateOf(documentId), change: "pages" });
    return { edit, report: { metadata: 0, scripts: 0, attachments: 0, comments: 0, hidden: 0 } };
  },
  addMarkup: async (documentId, kind, pages) => {
    if (pages.length === 0) throw { code: "invalid_edit", detail: "nothing marked" };
    let state = stateOf(documentId);
    for (const { page, rects } of pages) {
      const original = state.pages[page];
      if (original === undefined) throw { code: "page_out_of_range", detail: "page out of range" };
      const markup = new Map(state.markup);
      markup.set(original, [
        ...(markup.get(original) ?? []),
        { kind, rects, contents: null, strokes: [], ...signed() },
      ]);
      state = { pages: state.pages, markup, change: "annotations" };
    }
    return pushState(documentId, state);
  },
  addInk: async (documentId, page, strokes) => {
    const points = strokes.flat();
    if (points.length === 0) throw { code: "invalid_edit", detail: "no ink" };
    const xs = points.map((p) => p.x);
    const ys = points.map((p) => p.y);
    const rect = {
      x: Math.min(...xs) - 2,
      y: Math.min(...ys) - 2,
      width: Math.max(...xs) - Math.min(...xs) + 4,
      height: Math.max(...ys) - Math.min(...ys) + 4,
    };
    const ink: MockMarkup = {
      kind: "ink",
      rects: [rect],
      contents: null,
      strokes: strokes.map((stroke) => stroke.map((p): [number, number] => [p.x, p.y])),
      ...signed(),
    };
    return pushState(
      documentId,
      withMarkup(documentId, page, (list) => [...list, ink]),
    );
  },
  addNote: async (documentId, page, x, y, contents) => {
    const note: MockMarkup = {
      kind: "note",
      rects: [{ x, y, width: 20, height: 20 }],
      contents,
      strokes: [],
      ...signed(),
    };
    return pushState(
      documentId,
      withMarkup(documentId, page, (list) => [...list, note]),
    );
  },
  addTextBox: async (documentId, page, x, y, width, contents) => {
    if (contents.trim() === "") throw { code: "invalid_edit", detail: "a text box needs text" };
    const lines = Math.max(1, Math.ceil((contents.length * 6) / Math.max(1, width - 4)));
    const box: MockMarkup = {
      kind: "textBox",
      rects: [{ x, y, width, height: lines * 16 + 4 }],
      contents,
      strokes: [],
      ...signed(),
    };
    return pushState(
      documentId,
      withMarkup(documentId, page, (list) => [...list, box]),
    );
  },
  setAnnotationContents: async (documentId, page, index, contents) => {
    const next = withMarkup(documentId, page, (list) => {
      const found = list[index];
      if (found?.kind !== "note" && found?.kind !== "textBox") {
        throw { code: "invalid_edit", detail: "not a note or a text box" };
      }
      return list.map((item, i) => (i === index ? { ...item, contents } : item));
    });
    return pushState(documentId, next);
  },
  removeAnnotations: async (documentId, page, indices) => {
    const next = withMarkup(documentId, page, (list) => {
      if (indices.some((index) => !list[index])) {
        throw { code: "invalid_edit", detail: "no such annotation" };
      }
      return list.filter((_, i) => !indices.includes(i));
    });
    return pushState(documentId, next);
  },
  pageAnnotations: async (documentId, page) => {
    const state = stateOf(documentId);
    const original = state.pages[page];
    return (original === undefined ? [] : (state.markup.get(original) ?? [])).map(
      ({ kind, rects, contents, strokes }, index) => ({ index, kind, rects, contents, strokes }),
    );
  },
  // The mock keeps no page content of its own: the text is a step to undo.
  addText: async (documentId, page, x, y, width, text) => {
    if (text.trim() === "") throw { code: "invalid_edit", detail: "new text needs some text" };
    if (!(width >= 1) || !Number.isFinite(x) || !Number.isFinite(y)) {
      throw { code: "invalid_edit", detail: "text needs a place on the page" };
    }
    const state = stateOf(documentId);
    if (state.pages[page] === undefined)
      throw { code: "page_out_of_range", detail: "page out of range" };
    return {
      ...pushState(documentId, { pages: state.pages, markup: state.markup, change: "annotations" }),
      textChanged: true,
    };
  },
  // One paragraph on page 0, so the Edit tool can be tried in `pnpm dev`.
  pageTextBlocks: async (documentId, page) => {
    if (page !== 0) return [];
    const text = mockBlockTexts.get(documentId) ?? MOCK_BLOCK_TEXT;
    if (text.trim() === "") return [];
    return [
      {
        index: 0,
        fingerprint: text.length + 1,
        x: 72,
        y: 120,
        width: 300,
        height: 36,
        text,
        rtl: false,
        align: "left",
        size: 12,
        color: [0, 0, 0],
        lines: 2,
        font: "IBMPlexSans-Regular",
        fontReusable: true,
        flattened: false,
        editable: true,
        reason: null,
      } satisfies ParagraphBlock,
    ];
  },
  editTextBlock: async (documentId, edit) => {
    const state = stateOf(documentId);
    if (state.pages[edit.page] === undefined)
      throw { code: "page_out_of_range", detail: "page out of range" };
    const current = mockBlockTexts.get(documentId) ?? MOCK_BLOCK_TEXT;
    if (edit.page !== 0 || edit.index !== 0 || edit.fingerprint !== current.length + 1) {
      throw { code: "invalid_edit", detail: "the page's text changed since it was read" };
    }
    mockBlockTexts.set(documentId, edit.text);
    const arabic = /[\u0600-\u06FF]/.test(edit.text);
    return {
      ...pushState(documentId, { pages: state.pages, markup: state.markup, change: "annotations" }),
      textChanged: true,
      substituted:
        arabic || edit.family ? { from: "IBMPlexSans-Regular", to: edit.family ?? "sans" } : null,
      lines: Math.max(1, Math.ceil(edit.text.length / 50)),
    };
  },
  documentComments: async (documentId) => {
    const state = stateOf(documentId);
    return state.pages.flatMap((original, page) =>
      (state.markup.get(original) ?? []).map(
        (item, index): DocumentComment => ({
          page,
          index,
          kind: item.kind,
          author: item.author ?? null,
          modified: item.modified ?? null,
          contents:
            item.kind === "note" || item.kind === "textBox" ? (item.contents ?? null) : null,
          // The mock's pages have no text of their own.
          markedText: null,
          rect: item.rects.reduce((box, rect) => ({
            x: Math.min(box.x, rect.x),
            y: Math.min(box.y, rect.y),
            width: Math.max(box.x + box.width, rect.x + rect.width) - Math.min(box.x, rect.x),
            height: Math.max(box.y + box.height, rect.y + rect.height) - Math.min(box.y, rect.y),
          })),
          resolved: item.resolved ?? false,
          replies: item.replies ?? [],
        }),
      ),
    );
  },
  replyToComment: async (documentId, page, index, contents) => {
    if (contents.trim() === "") throw { code: "invalid_edit", detail: "a reply needs some text" };
    return withComment(documentId, page, index, (comment) => {
      const reply = { index: 1000 + (comment.replies?.length ?? 0), contents, ...signed() };
      return { ...comment, replies: [...(comment.replies ?? []), reply] };
    });
  },
  setCommentResolved: async (documentId, page, index, resolved) =>
    withComment(documentId, page, index, (comment) => ({ ...comment, resolved })),
  // The mock "picks" a file whose pages are the sample's first three.
  // The mock "picks" a file whose pages are the sample's first three; when
  // tests say so, it is the locked sample's, which asks for its password.
  insertPagesFromFiles: async (documentId, at) => {
    if (mockInsertIsLocked) {
      pendingInsert = { documentId, at };
      return { kind: "needsPassword", token: 9, name: LOCKED.name, retry: false };
    }
    return { kind: "inserted", edit: insertSample(documentId, at) };
  },
  unlockInsert: async (_token, password) => {
    const pending = pendingInsert;
    if (!pending) throw { code: "unknown_token", detail: "no insert waiting" };
    if (password !== LOCKED_PASSWORD) {
      return { kind: "needsPassword", token: 9, name: LOCKED.name, retry: true };
    }
    pendingInsert = null;
    return { kind: "inserted", edit: insertSample(pending.documentId, pending.at) };
  },
  // The mock draws nothing into pages: a stamp only takes the step (and is checked).
  stampPages: async (documentId, pages, stamp) => {
    const state = stateOf(documentId);
    if (pages.length === 0) throw { code: "invalid_edit", detail: "no pages given" };
    if (pages.some((page) => state.pages[page] === undefined)) {
      throw { code: "page_out_of_range", detail: "page out of range" };
    }
    if (
      stamp.kind === "watermark" &&
      stamp.watermark.mark.kind === "image" &&
      !mockWatermarkImage
    ) {
      throw { code: "no_image", detail: "no watermark image picked" };
    }
    return {
      ...pushState(documentId, { pages: state.pages, markup: state.markup, change: "annotations" }),
      textChanged: true,
    };
  },
  pickWatermarkImage: async () => {
    mockWatermarkImage = "logo.png";
    return mockWatermarkImage;
  },
  exportPageImages: async (_documentId, pages) => pages.length,
  exportText: async (_documentId, _pages, format) =>
    format === "markdown" ? "Sample.md" : "Sample.txt",
  // Sizes as a real compression of a scanned sample might report them.
  compressDocument: async (documentId, options) => {
    mockCompressed.add(documentId);
    const reduced = options.imageDpi !== null;
    return {
      before: 4_200_000,
      after: reduced ? 1_350_000 : 3_900_000,
      imagesReduced: reduced ? 12 : 0,
      imagesKept: 0,
      streamsCompressed: 3,
      objectsRemoved: 40,
    };
  },
  saveCompressed: async (documentId) => {
    if (!mockCompressed.delete(documentId)) {
      throw { code: "compress_stale", detail: "nothing compressed" };
    }
    return "Sample (compressed).pdf";
  },
  extractPages: async () => "Extract.pdf",
  // The mock "writes" parts of 20 pages when split by size, whatever the limit.
  splitDocument: async (documentId, by) => {
    const count = pagesOf(documentId).length;
    if (by.kind === "bookmarks") return (await mockBackend.bookmarkParts(documentId)).length;
    return Math.ceil(count / (by.kind === "pages" ? by.size : 20));
  },
  bookmarkParts: async (documentId) => {
    const count = pagesOf(documentId).length;
    const starts = (documentId === SAMPLE.id ? OUTLINE : [])
      .filter((item) => item.page !== null && item.page < count)
      .map((item) => ({ title: item.title, page: item.page ?? 0 }))
      .sort((a, b) => a.page - b.page)
      .filter((item, i, all) => i === 0 || item.page !== all[i - 1]?.page);
    return starts.map((item, i) => ({
      title: item.title,
      start: i === 0 ? 0 : item.page,
      end: starts[i + 1]?.page ?? count,
    }));
  },
  saveDocument: async (documentId) => {
    const history = edits.get(documentId);
    if (history) history.saved = history.at;
    return {
      name: documentId === LOCKED.id ? LOCKED.name : SAMPLE.name,
      modified: false,
      incremental: false,
    };
  },
  saveDocumentAs: async (documentId) => {
    const history = edits.get(documentId);
    if (history) history.saved = history.at;
    return { name: "Copy.pdf", modified: false, incremental: false };
  },
  redoEdit: async (documentId) => {
    const history = edits.get(documentId);
    if (!history || history.at >= history.states.length - 1) {
      return outcome(documentId, "annotations");
    }
    history.at += 1;
    return outcome(documentId, history.states[history.at]?.change ?? "pages");
  },
  documentOutline: async (documentId) => (documentId === SAMPLE.id ? OUTLINE : []),
  pageLabels: async () => null,
  pageText: async (documentId, page) => {
    const original = pagesOf(documentId)[page] ?? page;
    return original < 0 ? { blocks: [], direction: "ltr", quality: "noText" } : textLayer(original);
  },
  searchDocument: async (documentId, query, options) => {
    const hits = search(documentId, query, options);
    return { hits: hits.slice(0, 5000), truncated: hits.length > 5000 };
  },
  renderTile: (documentId, requestGeneration, page, tile, priority, lane = "main") =>
    new Promise<RenderedPage>((resolve, reject) => {
      if (requestGeneration < generations[lane]) {
        reject(CANCELLED);
        return;
      }
      queue.push({
        lane,
        generation: requestGeneration,
        page,
        original: pagesOf(documentId)[page] ?? page,
        markup: stateOf(documentId).markup.get(pagesOf(documentId)[page] ?? page) ?? [],
        tile,
        priority,
        seq: ++seq,
        resolve,
        reject,
      });
      pump();
    }),
  updateViewport: async (_documentId, next, keep, lane = "main") =>
    updateViewport(lane, next, keep),
  closeDocument: async () => {},
  // One window in the browser.
  newWindow: async () => {},
  printDocument: async () => {},
  secureDocument: async (documentId, change) => ({
    name: documentId === LOCKED.id ? LOCKED.name : SAMPLE.name,
    modified: false,
    security:
      change.kind === "set"
        ? {
            encrypted: true,
            cipher: change.cipher,
            revision: change.cipher === "aes256" ? 6 : 4,
            permissions: change.permissions,
            owner: true,
          }
        : {
            encrypted: false,
            cipher: null,
            revision: 0,
            permissions: ALL_PERMISSIONS,
            owner: true,
          },
  }),
  moveToNewWindow: async () => {},
  onSettingChanged: async () => () => {},
  onCloseRequested: async () => () => {},
  onQuitRequested: async () => () => {},
  closeWindowDiscarding: async () => {},
  quitDiscarding: async () => {},
  selftestConfig: async () => null,
  selftestReport: async () => {},
  renderStats: async () => null,
  windowLabels: async () => null,
  shellRssKb: async () => null,
  webviewRssKb: async () => null,
  selftestLog: async () => {},
};
