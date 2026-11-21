/**
 * Milestone 0 spike (throwaway): tile transport benchmark.
 *
 * Runs inside the real webview and times the same PDFium tile arriving through
 * each candidate transport, plus PDFium compiled to WebAssembly rendering in
 * the webview itself. Results and the decision: docs/adr/0004-tile-transport.md.
 * Run with scripts/spike-tile-transport.sh.
 */
import { Channel, invoke } from "@tauri-apps/api/core";
import { decodeBitmap } from "../backend/bitmap";
import type { RenderedPage } from "../backend/types";
import { nextFrame } from "../selftest";

interface Tile {
  scale: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A tile request in flight: resolves with the bitmap and the engine's render time. */
type Transport = (documentId: number, tile: Tile) => Promise<RenderedPage>;

const PAGE = 0;
const WARMUP = 3;
const ITERATIONS = 25;

/** 4 px/pt is 300% zoom on a standard display, 150% on a 2x display. */
const SCALE = 4;
const CASES: { name: string; tile: Tile }[] = [
  { name: "256x256", tile: { scale: SCALE, x: 768, y: 512, width: 256, height: 256 } },
  { name: "512x512", tile: { scale: SCALE, x: 768, y: 512, width: 512, height: 512 } },
  { name: "1024x1024", tile: { scale: SCALE, x: 768, y: 512, width: 1024, height: 1024 } },
  // A whole A4 page at 3 px/pt: what Milestone 0's single-bitmap viewer sends.
  { name: "page 1785x2526", tile: { scale: 3, x: 0, y: 0, width: 1785, height: 2526 } },
];

function decodeBase64(text: string): ArrayBuffer {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

const TRANSPORTS: Record<string, Transport> = {
  "invoke-raw": async (documentId, tile) =>
    decodeBitmap(await invoke<ArrayBuffer>("spike_tile_raw", { documentId, page: PAGE, tile })),

  "custom-protocol": async (documentId, tile) => {
    const { scale, x, y, width, height } = tile;
    const url = `safha-tile://localhost/${documentId}/${PAGE}?scale=${scale}&x=${x}&y=${y}&w=${width}&h=${height}`;
    const response = await fetch(url);
    if (!response.ok) throw new Error(`protocol ${response.status}`);
    return decodeBitmap(await response.arrayBuffer());
  },

  channel: (documentId, tile) =>
    new Promise<RenderedPage>((resolve, reject) => {
      const channel = new Channel<ArrayBuffer>();
      channel.onmessage = (buffer) => resolve(decodeBitmap(buffer));
      invoke("spike_tile_channel", { documentId, page: PAGE, tile, channel }).catch(reject);
    }),

  "base64-json": async (documentId, tile) =>
    decodeBitmap(
      decodeBase64(await invoke<string>("spike_tile_base64", { documentId, page: PAGE, tile })),
    ),
};

// ---------------------------------------------------------------- PDFium in WebAssembly

interface PdfiumModule {
  HEAPU8: Uint8Array;
  _malloc(size: number): number;
  _FPDF_InitLibrary(): void;
  _FPDF_LoadMemDocument(data: number, size: number, password: number): number;
  _FPDF_LoadPage(document: number, index: number): number;
  _FPDF_GetPageWidthF(page: number): number;
  _FPDF_GetPageHeightF(page: number): number;
  _FPDFBitmap_Create(width: number, height: number, alpha: number): number;
  _FPDFBitmap_FillRect(bitmap: number, l: number, t: number, w: number, h: number, c: number): void;
  _FPDF_RenderPageBitmap(
    bitmap: number,
    page: number,
    startX: number,
    startY: number,
    sizeX: number,
    sizeY: number,
    rotate: number,
    flags: number,
  ): void;
  _FPDFBitmap_GetBuffer(bitmap: number): number;
  _FPDFBitmap_Destroy(bitmap: number): void;
}

const FPDF_ANNOT = 0x01;
const FPDF_REVERSE_BYTE_ORDER = 0x10;

async function loadWasmEngine() {
  const startedAt = performance.now();
  const module = await new Promise<PdfiumModule>((resolve, reject) => {
    const target = window as unknown as { Module?: unknown };
    target.Module = {
      locateFile: (file: string) => `/spike-pdfium/${file}`,
      onRuntimeInitialized: () => resolve(target.Module as PdfiumModule),
    };
    const script = document.createElement("script");
    script.src = "/spike-pdfium/pdfium.js";
    script.onerror = () => reject(new Error("could not load /spike-pdfium/pdfium.js"));
    document.head.append(script);
  });
  module._FPDF_InitLibrary();
  const pdf = new Uint8Array(await invoke<ArrayBuffer>("spike_pdf_bytes"));
  const pointer = module._malloc(pdf.length);
  module.HEAPU8.set(pdf, pointer);
  const doc = module._FPDF_LoadMemDocument(pointer, pdf.length, 0);
  const page = module._FPDF_LoadPage(doc, PAGE);
  if (!doc || !page) throw new Error("PDFium (wasm) failed to open the document");
  const widthPt = module._FPDF_GetPageWidthF(page);
  const heightPt = module._FPDF_GetPageHeightF(page);
  const initMs = performance.now() - startedAt;

  /** Renders a tile and hands `use` a zero-copy view of it in the wasm heap. */
  const render = (tile: Tile, use: (bitmap: RenderedPage) => void) => {
    const startedRender = performance.now();
    const bitmap = module._FPDFBitmap_Create(tile.width, tile.height, 1);
    module._FPDFBitmap_FillRect(bitmap, 0, 0, tile.width, tile.height, 0xffffffff);
    module._FPDF_RenderPageBitmap(
      bitmap,
      page,
      -tile.x,
      -tile.y,
      Math.ceil(widthPt * tile.scale),
      Math.ceil(heightPt * tile.scale),
      0,
      FPDF_ANNOT | FPDF_REVERSE_BYTE_ORDER,
    );
    const renderMicros = (performance.now() - startedRender) * 1000;
    const buffer = module._FPDFBitmap_GetBuffer(bitmap);
    const rgba = new Uint8ClampedArray(
      module.HEAPU8.buffer as ArrayBuffer,
      buffer,
      tile.width * tile.height * 4,
    );
    use({ width: tile.width, height: tile.height, renderMicros, rgba });
    module._FPDFBitmap_Destroy(bitmap);
  };

  return { render, initMs, heapBytes: () => module.HEAPU8.length };
}

// ---------------------------------------------------------------- measurement

const median = (values: number[]) => quantile(values, 0.5);

function quantile(values: number[], q: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? Number.NaN;
}

const round = (value: number) => Math.round(value * 100) / 100;

function summarize(samples: { total: number; render: number; block: number }[], bytes: number) {
  const total = samples.map((sample) => sample.total);
  const overhead = samples.map((sample) => sample.total - sample.render);
  return {
    megabytes: round(bytes / 1e6),
    totalMedianMs: round(median(total)),
    totalP95Ms: round(quantile(total, 0.95)),
    renderMedianMs: round(median(samples.map((sample) => sample.render))),
    overheadMedianMs: round(median(overhead)),
    overheadP95Ms: round(quantile(overhead, 0.95)),
    /** Main-thread time spent synchronously per tile (decode + putImageData, or the wasm render). */
    mainThreadBlockMedianMs: round(median(samples.map((sample) => sample.block))),
  };
}

function paint(context: CanvasRenderingContext2D, bitmap: RenderedPage) {
  context.putImageData(new ImageData(bitmap.rgba, bitmap.width, bitmap.height), 0, 0);
}

/** Watches frame pacing while `work` runs. */
async function withFrameMonitor(work: () => Promise<void>) {
  const gaps: number[] = [];
  let running = true;
  let last = performance.now();
  const tick = (now: number) => {
    gaps.push(now - last);
    last = now;
    if (running) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  const startedAt = performance.now();
  await work();
  const elapsedMs = performance.now() - startedAt;
  await nextFrame();
  running = false;
  return {
    elapsedMs: round(elapsedMs),
    longestFrameMs: round(Math.max(...gaps)),
    framesOver20Ms: gaps.filter((gap) => gap > 20).length,
    frames: gaps.length,
  };
}

export async function runTileTransportSpike(documentId: number) {
  const canvas = document.createElement("canvas");
  canvas.width = 2048;
  canvas.height = 2600;
  canvas.style.cssText = "position:fixed;inset-block-start:0;inset-inline-start:0;opacity:0.01;";
  document.body.append(canvas);
  const context = canvas.getContext("2d");
  if (!context) throw new Error("no 2d context");

  const rssMegabytes = async () => {
    const kb = await invoke<number | null>("spike_rss_kb");
    return kb ? round(kb / 1024) : null;
  };
  const idle = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  const results: Record<string, Record<string, unknown>> = {};
  /** Shell process RSS after each phase, to attribute memory growth to a transport. */
  const shellRss: Record<string, number | null> = { start: await rssMegabytes() };

  // 1. Latency per tile, one request at a time.
  for (const [name, transport] of Object.entries(TRANSPORTS)) {
    results[name] = {};
    for (const { name: caseName, tile } of CASES) {
      const samples = [];
      for (let i = 0; i < WARMUP + ITERATIONS; i += 1) {
        const startedAt = performance.now();
        const bitmap = await transport(documentId, tile);
        const arrivedAt = performance.now();
        paint(context, bitmap);
        const doneAt = performance.now();
        if (i >= WARMUP) {
          samples.push({
            total: doneAt - startedAt,
            render: bitmap.renderMicros / 1000,
            block: doneAt - arrivedAt,
          });
        }
      }
      (results[name] as Record<string, unknown>)[caseName] = summarize(
        samples,
        tile.width * tile.height * 4,
      );
    }
    shellRss[`after ${name}`] = await rssMegabytes();
  }
  await idle(3000);
  shellRss["after 3 s idle"] = await rssMegabytes();

  // 2. PDFium compiled to WebAssembly, rendering on the webview's main thread.
  let wasm: Record<string, unknown> = { error: "not run" };
  let wasmEngine: Awaited<ReturnType<typeof loadWasmEngine>> | null = null;
  try {
    wasmEngine = await loadWasmEngine();
    const engine = wasmEngine;
    const cases: Record<string, unknown> = {};
    for (const { name: caseName, tile } of CASES) {
      const samples = [];
      for (let i = 0; i < WARMUP + ITERATIONS; i += 1) {
        const startedAt = performance.now();
        let render = 0;
        engine.render(tile, (bitmap) => {
          render = bitmap.renderMicros / 1000;
          paint(context, bitmap);
        });
        const total = performance.now() - startedAt;
        if (i >= WARMUP) samples.push({ total, render, block: total });
      }
      cases[caseName] = summarize(samples, tile.width * tile.height * 4);
    }
    wasm = { initMs: round(engine.initMs), heapMegabytes: round(engine.heapBytes() / 1e6), cases };
  } catch (error) {
    wasm = { error: String(error) };
  }

  // 3. A viewport's worth of tiles at once (4 x 3 tiles of 512 px), with frame pacing.
  const grid: Tile[] = [];
  for (let row = 0; row < 3; row += 1) {
    for (let column = 0; column < 4; column += 1) {
      grid.push({ scale: SCALE, x: column * 512, y: 512 + row * 512, width: 512, height: 512 });
    }
  }
  const burst: Record<string, unknown> = {};
  for (const [name, transport] of Object.entries(TRANSPORTS)) {
    const runs = [];
    for (let i = 0; i < 5; i += 1) {
      runs.push(
        await withFrameMonitor(async () => {
          await Promise.all(
            grid.map(async (tile) => paint(context, await transport(documentId, tile))),
          );
        }),
      );
    }
    burst[name] = runs.sort((a, b) => a.elapsedMs - b.elapsedMs)[2];
  }
  if (wasmEngine) {
    const engine = wasmEngine;
    const runs = [];
    for (let i = 0; i < 5; i += 1) {
      runs.push(
        await withFrameMonitor(async () => {
          // One tile per frame: the best a main-thread renderer can do for pacing.
          for (const tile of grid) {
            engine.render(tile, (bitmap) => paint(context, bitmap));
            await nextFrame();
          }
        }),
      );
    }
    burst["wasm-main-thread"] = runs.sort((a, b) => a.elapsedMs - b.elapsedMs)[2];
  }

  canvas.remove();
  return {
    spike: "tile-transport",
    userAgent: navigator.userAgent,
    devicePixelRatio: window.devicePixelRatio,
    // Frame-pacing numbers are only meaningful when the window was actually on screen.
    visibility: document.visibilityState,
    iterations: ITERATIONS,
    megabytesSentPerTransport: round(
      (CASES.reduce((sum, { tile }) => sum + tile.width * tile.height * 4, 0) *
        (WARMUP + ITERATIONS)) /
        1e6,
    ),
    shellRssMegabytes: shellRss,
    transports: results,
    wasm,
    burstOf12Tiles: burst,
  };
}
