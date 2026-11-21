import type { Backend, DocumentInfo, PageSize, RenderedPage } from "./types";

/**
 * In-browser stand-in for the Rust shell, used when the UI runs outside Tauri
 * (plain `pnpm dev`, component tests). It draws placeholder pages with the 2D
 * canvas so layout, theming, and RTL work can be reviewed in any browser.
 * Nothing here is PDF rendering.
 */

const A4: PageSize = { width: 595.276, height: 841.89 };

const SAMPLE: DocumentInfo = { id: 1, name: "عقد إيجار — Lease Agreement.pdf", pageCount: 3 };

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

function drawPage(page: number, scale: number): RenderedPage {
  const width = Math.ceil(A4.width * scale);
  const height = Math.ceil(A4.height * scale);
  const blank = (): RenderedPage => ({
    width,
    height,
    renderMicros: 0,
    rgba: new Uint8ClampedArray(width * height * 4).fill(255),
  });

  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  // jsdom implements no canvas; tests get a blank page.
  let ctx: CanvasRenderingContext2D | null = null;
  try {
    ctx = canvas.getContext("2d");
  } catch {
    return blank();
  }
  const content = PAGES[page];
  if (!ctx || !content) return blank();

  const rtl = content.direction === "rtl";
  const margin = 72 * scale;
  const x = rtl ? width - margin : margin;
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, width, height);
  ctx.fillStyle = "#1b1f24";
  ctx.direction = content.direction;
  ctx.textAlign = "start";
  ctx.font = `600 ${22 * scale}px "IBM Plex Sans", "IBM Plex Sans Arabic", sans-serif`;
  ctx.fillText(content.heading, x, margin + 22 * scale);
  ctx.font = `400 ${11 * scale}px "IBM Plex Sans", "IBM Plex Sans Arabic", sans-serif`;
  content.lines.forEach((line, index) => {
    ctx.fillText(line, x, margin + (64 + index * 22) * scale);
  });

  const image = ctx.getImageData(0, 0, width, height);
  return { width, height, renderMicros: 0, rgba: new Uint8ClampedArray(image.data) };
}

export const mockBackend: Backend = {
  appInfo: async () => ({ version: "0.0.1-mock", devtools: false }),
  openDocumentDialog: async () => SAMPLE,
  takeStartupDocument: async () => null,
  pageSize: async () => A4,
  renderPage: async (_documentId, page, scale) => drawPage(page, scale),
  closeDocument: async () => {},
  selftestConfig: async () => null,
  selftestReport: async () => {},
};
