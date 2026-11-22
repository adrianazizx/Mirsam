import type {
  Backend,
  DocumentInfo,
  EditOutcome,
  KeptRequest,
  Lane,
  MarkupKind,
  OpenOutcome,
  OutlineItem,
  PageEdit,
  PageSize,
  Priority,
  RecentFile,
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

/** A markup annotation the mock keeps: displayed-page rectangles of its page. */
interface MockMarkup {
  kind: MarkupKind;
  rects: ViewRect[];
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

/** Markup over a page, drawn like the engine's appearance streams. */
function drawMarkup(ctx: CanvasRenderingContext2D, markup: MockMarkup[]) {
  const colours: Record<MarkupKind, string> = {
    highlight: "rgb(255 219 0)",
    underline: "rgb(0 107 230)",
    squiggly: "rgb(0 107 230)",
    strikeOut: "rgb(219 33 33)",
  };
  for (const { kind, rects } of markup) {
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

export const mockBackend: Backend = {
  appInfo: async () => ({ version: "0.0.1-mock", devtools: false }),
  openDocumentDialog: async () => opened(SAMPLE),
  startup: async () => ({ documents: [], active: null }),
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
  addMarkup: async (documentId, kind, pages) => {
    if (pages.length === 0) throw { code: "invalid_edit", detail: "nothing marked" };
    let state = stateOf(documentId);
    for (const { page, rects } of pages) {
      const original = state.pages[page];
      if (original === undefined) throw { code: "page_out_of_range", detail: "page out of range" };
      const markup = new Map(state.markup);
      markup.set(original, [...(markup.get(original) ?? []), { kind, rects }]);
      state = { pages: state.pages, markup, change: "annotations" };
    }
    return pushState(documentId, state);
  },
  removeAnnotation: async (documentId, page, index) => {
    const next = withMarkup(documentId, page, (list) => {
      if (!list[index]) throw { code: "invalid_edit", detail: "no such annotation" };
      return list.filter((_, i) => i !== index);
    });
    return pushState(documentId, next);
  },
  pageMarkup: async (documentId, page) => {
    const state = stateOf(documentId);
    const original = state.pages[page];
    return (original === undefined ? [] : (state.markup.get(original) ?? [])).map(
      (markup, index) => ({ index, ...markup }),
    );
  },
  // The mock "picks" a file whose pages are the sample's first three.
  insertPagesFromFiles: async (documentId, at) => {
    const state = stateOf(documentId);
    const pages = [...state.pages.slice(0, at), 0, 1, 2, ...state.pages.slice(at)];
    return pushState(documentId, { pages, markup: state.markup, change: "pages" });
  },
  extractPages: async () => "Extract.pdf",
  splitDocument: async (documentId, size) => Math.ceil(pagesOf(documentId).length / size),
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
