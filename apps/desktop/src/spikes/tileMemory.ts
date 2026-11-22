/**
 * Milestone 1 experiment (throwaway; results in docs/adr/0011): where does the
 * webview's memory go when thousands of tiles arrive? Each run of the app
 * delivers the same 2,000 distinct 512 px tiles one way and records the
 * WebKit processes' resident memory. Variants (SAFHA_SELFTEST_SPIKE=tile-memory:<variant>):
 *
 * - raw-none:   invoke → ArrayBuffer, then dropped (transport alone)
 * - raw-put:    invoke → ArrayBuffer → putImageData into one reused canvas (the
 *               product's path before ADR 0011)
 * - raw-bitmap: invoke → ArrayBuffer → createImageBitmap → drawImage, closed at once
 * - bmp-blob:   fetch(safha-tile://…&format=bmp) → Blob → createImageBitmap → drawImage
 * - product:    the product's path after ADR 0011 (backend.renderTile: the
 *               `safha` scheme through the render queue, BMP, ImageBitmap)
 * - bmp-img:    <img src="safha-tile://…&format=bmp"> → decode → drawImage
 */
import { invoke } from "@tauri-apps/api/core";
import {
  backend,
  drawRendered,
  releaseRendered,
  renderedCanvas,
  type TileRequest,
} from "../backend";
import { nextFrame } from "../selftest";
import { pagePixelSize, tilesInRegion } from "../viewer/geometry";
import { decodeBitmap } from "./rawBitmap";

const TILES = 2000;
const IN_FLIGHT = 4;
const TILE = 512;

function tileUrl(documentId: number, page: number, tile: TileRequest, format = "bmp"): string {
  const base = navigator.userAgent.includes("Windows")
    ? "http://safha-tile.localhost"
    : "safha-tile://localhost";
  return `${base}/${documentId}/${page}?scale=${tile.scale}&x=${tile.x}&y=${tile.y}&w=${tile.width}&h=${tile.height}&format=${format}`;
}

export async function runTileMemorySpike(documentId: number, variant: string) {
  const sizes = await backend.pageSizes(documentId);
  const scale = Math.fround(2);
  const jobs: { page: number; tile: TileRequest }[] = [];
  for (let page = 0; page < sizes.length && jobs.length < TILES; page += 1) {
    const size = sizes[page];
    if (!size) continue;
    const pixels = pagePixelSize(size, scale);
    const region = { x0: 0, y0: 0, x1: pixels.width, y1: pixels.height };
    for (const tile of tilesInRegion(pixels.width, pixels.height, region, scale)) {
      if (tile.width === TILE && tile.height === TILE) jobs.push({ page, tile });
    }
  }
  jobs.length = Math.min(jobs.length, TILES);

  const canvas = document.createElement("canvas");
  canvas.width = TILE;
  canvas.height = TILE;
  const context = canvas.getContext("2d", { alpha: false });
  if (!context) throw new Error("no 2d context");
  let generation = Date.now() * 1000;
  const live: HTMLCanvasElement[] = [];
  const pool: HTMLCanvasElement[] = [];

  const deliver = async ({ page, tile }: { page: number; tile: TileRequest }) => {
    switch (variant) {
      case "raw-none": {
        decodeBitmap(await invoke<ArrayBuffer>("spike_tile_raw", { documentId, page, tile }));
        return;
      }
      case "raw-put": {
        const bitmap = decodeBitmap(
          await invoke<ArrayBuffer>("spike_tile_raw", { documentId, page, tile }),
        );
        context.putImageData(new ImageData(bitmap.rgba, bitmap.width, bitmap.height), 0, 0);
        return;
      }
      case "raw-bitmap": {
        const bitmap = decodeBitmap(
          await invoke<ArrayBuffer>("spike_tile_raw", { documentId, page, tile }),
        );
        const image = await createImageBitmap(
          new ImageData(bitmap.rgba, bitmap.width, bitmap.height),
        );
        context.drawImage(image, 0, 0);
        image.close();
        return;
      }
      case "bmp-blob": {
        const response = await fetch(tileUrl(documentId, page, tile));
        const image = await createImageBitmap(await response.blob());
        context.drawImage(image, 0, 0);
        image.close();
        return;
      }
      case "product": {
        const rendered = await backend.renderTile(documentId, ++generation, page, tile, {
          urgency: "visible",
          distance: 0,
        });
        drawRendered(context, rendered);
        releaseRendered(rendered);
        return;
      }
      case "product-canvas":
      case "product-pool": {
        // Like the viewer: a canvas per tile, the newest 30 kept alive (on
        // screen), older ones released. "pool" reuses released canvases.
        const rendered = await backend.renderTile(documentId, ++generation, page, tile, {
          urgency: "visible",
          distance: 0,
        });
        let canvas = variant === "product-pool" ? pool.pop() : undefined;
        if (canvas) {
          canvas.getContext("2d", { alpha: false })?.clearRect(0, 0, canvas.width, canvas.height);
          const ctx2 = canvas.getContext("2d", { alpha: false });
          if (ctx2) drawRendered(ctx2, rendered);
        } else {
          canvas = renderedCanvas(rendered);
        }
        releaseRendered(rendered);
        document.body.append(canvas);
        live.push(canvas);
        if (live.length > 30) {
          const old = live.shift();
          if (old) {
            old.remove();
            if (variant === "product-pool") pool.push(old);
            else {
              old.width = 0;
              old.height = 0;
            }
          }
        }
        context.drawImage(canvas, 0, 0);
        return;
      }
      case "product-img": {
        const { tileUrlFor } = await import("../backend/tauri");
        const query = new URLSearchParams({
          doc: String(documentId),
          lane: "main",
          gen: String(++generation),
          page: String(page),
          scale: String(tile.scale),
          x: String(tile.x),
          y: String(tile.y),
          w: String(tile.width),
          h: String(tile.height),
          u: "visible",
          d: "0",
        });
        const image = new Image();
        // The scheme answers with Access-Control-Allow-Origin: *, so the canvas stays readable.
        image.crossOrigin = "anonymous";
        image.src = tileUrlFor(query);
        await image.decode();
        context.drawImage(image, 0, 0);
        image.src = "";
        return;
      }
      case "bmp-img": {
        const image = new Image();
        image.src = tileUrl(documentId, page, tile);
        await image.decode();
        context.drawImage(image, 0, 0);
        image.src = "";
        return;
      }
      default:
        throw new Error(`unknown variant ${variant}`);
    }
  };

  const memory = async () => ({
    webviewMb: Math.round(((await backend.webviewRssKb()) ?? 0) / 1024),
    shellMb: Math.round(((await backend.shellRssKb()) ?? 0) / 1024),
  });
  const samples: Record<string, unknown> = { start: await memory() };
  const started = performance.now();
  let next = 0;
  const worker = async () => {
    while (next < jobs.length) {
      const job = jobs[next++];
      if (job) await deliver(job);
      if (next % 500 === 0) {
        samples[`after ${next}`] = await memory();
        void backend.selftestLog(`tile-memory ${variant}: ${next} tiles`);
      }
    }
  };
  await Promise.all(Array.from({ length: IN_FLIGHT }, worker));
  const elapsedMs = performance.now() - started;
  samples.end = await memory();
  for (const wait of [2, 10]) {
    const until = performance.now() + (wait === 2 ? 2000 : 8000);
    while (performance.now() < until) await nextFrame();
    samples[`idle ${wait} s`] = await memory();
  }
  // A check that the pixels arrived: the last tile drawn has ink.
  const data = context.getImageData(0, 0, TILE, TILE).data;
  let ink = 0;
  for (let i = 0; i < data.length; i += 4) if ((data[i] ?? 255) < 200) ink += 1;
  return {
    spike: "tile-memory",
    variant,
    tiles: jobs.length,
    megabytesMoved: Math.round((jobs.length * TILE * TILE * 4) / 1e6),
    msPerTile: elapsedMs / jobs.length,
    inkPixels: ink,
    memory: samples,
  };
}
