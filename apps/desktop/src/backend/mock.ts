import type {
  Backend,
  DocumentInfo,
  KeptRequest,
  PageSize,
  Priority,
  RenderedPage,
  TileRequest,
} from "./types";

/**
 * In-browser stand-in for the Rust shell, used when the UI runs outside Tauri
 * (plain `pnpm dev`, component tests). It draws placeholder pages with the 2D
 * canvas so layout, theming, scrolling, and RTL work can be reviewed in any
 * browser. Nothing here is PDF rendering.
 *
 * Like the engine, it renders one request at a time, most urgent first, and
 * honours viewport generations, so the viewer's cancellation path runs here too.
 */

const A4: PageSize = { width: Math.fround(595.276), height: Math.fround(841.89) };
const A4_LANDSCAPE: PageSize = { width: A4.height, height: A4.width };

const SAMPLE: DocumentInfo = { id: 1, name: "عقد إيجار — Lease Agreement.pdf", pageCount: 240 };

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

function blank(width: number, height: number): RenderedPage {
  return {
    width,
    height,
    renderMicros: 0,
    fromCache: false,
    rgba: new Uint8ClampedArray(width * height * 4).fill(255),
  };
}

function drawTile(page: number, tile: TileRequest): RenderedPage {
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
  const content = PAGES[page] ?? {
    direction: page % 2 === 0 ? "rtl" : "ltr",
    heading: page % 2 === 0 ? `صفحة ${page + 1}` : `Page ${page + 1}`,
    lines: [],
  };
  const rtl = content.direction === "rtl";
  const margin = 72;
  const x = rtl ? size.width - margin : margin;
  ctx.fillStyle = "#1b1f24";
  ctx.direction = content.direction;
  ctx.textAlign = "start";
  ctx.font = `600 22px "IBM Plex Sans", "IBM Plex Sans Arabic", sans-serif`;
  ctx.fillText(content.heading, x, margin + 22);
  ctx.font = `400 11px "IBM Plex Sans", "IBM Plex Sans Arabic", sans-serif`;
  content.lines.forEach((line, index) => {
    ctx.fillText(line, x, margin + 64 + index * 22);
  });
  // Grey bars stand in for body text, ragged on the side text ends on.
  ctx.fillStyle = "#c9ced6";
  const textWidth = size.width - 2 * margin;
  for (let line = 0; line < 30; line += 1) {
    const y = margin + 140 + line * 18;
    if (y > size.height - margin - 30) break;
    const length = textWidth * (line % 6 === 5 ? 0.55 : 0.92 + ((line * 37) % 8) / 100);
    const barWidth = Math.min(textWidth, length);
    ctx.fillRect(rtl ? size.width - margin - barWidth : margin, y, barWidth, 7);
  }
  ctx.fillStyle = "#57606a";
  ctx.textAlign = "center";
  ctx.direction = "ltr";
  ctx.fillText(`— ${page + 1} —`, size.width / 2, size.height - margin / 2);

  const image = ctx.getImageData(0, 0, width, height);
  return {
    width,
    height,
    renderMicros: 0,
    fromCache: false,
    rgba: new Uint8ClampedArray(image.data),
  };
}

interface Job {
  generation: number;
  page: number;
  tile: TileRequest;
  priority: Priority;
  seq: number;
  resolve(page: RenderedPage): void;
  reject(error: unknown): void;
}

const URGENCY = { placeholder: 0, visible: 1, prefetch: 2 } as const;
const CANCELLED = { code: "cancelled", detail: "the viewport moved on" };
const sameTile = (a: TileRequest, b: TileRequest) =>
  a.scale === b.scale && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;

const queue: Job[] = [];
let generation = 0;
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
    job.resolve(drawTile(job.page, job.tile));
    pump();
  }, 1);
}

function updateViewport(next: number, keep: KeptRequest[]) {
  if (next < generation) return;
  generation = next;
  for (let index = queue.length - 1; index >= 0; index -= 1) {
    const job = queue[index];
    if (!job) continue;
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

export const mockBackend: Backend = {
  appInfo: async () => ({ version: "0.0.1-mock", devtools: false }),
  openDocumentDialog: async () => SAMPLE,
  takeStartupDocument: async () => null,
  pageSizes: async () => Array.from({ length: SAMPLE.pageCount }, (_, page) => sizeOf(page)),
  renderTile: (_documentId, requestGeneration, page, tile, priority) =>
    new Promise<RenderedPage>((resolve, reject) => {
      if (requestGeneration < generation) {
        reject(CANCELLED);
        return;
      }
      queue.push({
        generation: requestGeneration,
        page,
        tile,
        priority,
        seq: ++seq,
        resolve,
        reject,
      });
      pump();
    }),
  updateViewport: async (_documentId, next, keep) => updateViewport(next, keep),
  closeDocument: async () => {},
  selftestConfig: async () => null,
  selftestReport: async () => {},
  renderStats: async () => null,
  shellRssKb: async () => null,
  webviewRssKb: async () => null,
  selftestLog: async () => {},
};
