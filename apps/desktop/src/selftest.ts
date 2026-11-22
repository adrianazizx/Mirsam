import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { backend } from "./backend";
import { runCommand } from "./commands/registry";
import { useAppStore } from "./state/appStore";
import { PAGE_PAINTED_EVENT, type PagePaintedDetail } from "./viewer/events";
import { getViewer } from "./viewer/registry";

/**
 * Resolves on the next animation frame. A webview whose window is occluded or
 * whose display is asleep gets no animation frames at all, so fall back to a
 * timer rather than hanging the self-test.
 */
export function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(resolve, 250);
    requestAnimationFrame(() => {
      window.clearTimeout(timer);
      resolve();
    });
  });
}

/** Resolves with the first painted event for `page`, or null after `timeoutMs`. */
export function waitForPagePainted(
  page: number,
  timeoutMs: number,
): Promise<PagePaintedDetail | null> {
  return new Promise((resolve) => {
    const onPainted = (event: Event) => {
      const detail = (event as CustomEvent<PagePaintedDetail>).detail;
      if (detail.page !== page) return;
      window.clearTimeout(timeout);
      window.removeEventListener(PAGE_PAINTED_EVENT, onPainted);
      resolve(detail);
    };
    const timeout = window.setTimeout(() => {
      window.removeEventListener(PAGE_PAINTED_EVENT, onPainted);
      resolve(null);
    }, timeoutMs);
    window.addEventListener(PAGE_PAINTED_EVENT, onPainted);
  });
}

/**
 * Draws page `page` as it is on screen — its full-resolution tiles at their
 * positions — into one canvas of the whole page's pixel size, and measures the
 * ink in the tiled area.
 */
function compositePage(documentId: number, painted: PagePaintedDetail) {
  const viewer = getViewer(documentId);
  if (!viewer) return null;
  const { tiles } = viewer.pageCanvases(painted.page);
  const canvas = document.createElement("canvas");
  canvas.width = painted.width;
  canvas.height = painted.height;
  const context = canvas.getContext("2d");
  if (!context) return null;
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  let ink = 0;
  let pixels = 0;
  for (const { canvas: tileCanvas, tile } of tiles) {
    context.drawImage(tileCanvas, tile.x, tile.y);
    const data = tileCanvas.getContext("2d")?.getImageData(0, 0, tile.width, tile.height).data;
    if (!data) continue;
    for (let i = 0; i < data.length; i += 4) {
      if ((data[i] ?? 255) < 200 || (data[i + 1] ?? 255) < 200 || (data[i + 2] ?? 255) < 200) {
        ink += 1;
      }
    }
    pixels += data.length / 4;
  }
  return { canvas, tiles: tiles.length, inkRatio: pixels > 0 ? ink / pixels : null };
}

/** Polls `ready` once per frame until it holds or `timeoutMs` passes. */
async function waitFor(ready: () => boolean, timeoutMs: number): Promise<boolean> {
  const until = performance.now() + timeoutMs;
  while (!ready()) {
    if (performance.now() > until) return false;
    await nextFrame();
  }
  return true;
}

/**
 * Milestone 1 navigation on the real engine: the text layer, selecting and
 * copying the first page's text, a search for one of its words with hits
 * drawn on the page, the outline, and thumbnails.
 */
async function navigationChecks(documentId: number) {
  const viewer = getViewer(documentId);
  const layer = await backend.pageText(documentId, 0);
  const layerText = layer.blocks
    .map((block) => block.lines.map((line) => line.text).join("\n"))
    .join("\n\n");

  // Select all of page 1 through the command, as a user would.
  await runCommand("edit.selectAll");
  await waitFor(() => viewer?.hasSelection() ?? false, 3_000);
  const selection = viewer?.selectedText() ?? "";
  const overlay = document.querySelector('[data-page="0"] [class*="overlay"]');
  const selectionRects = overlay?.childElementCount ?? 0;
  const toolbar = document.querySelector('[data-testid="selection-toolbar"]') !== null;
  await runCommand("edit.deselect");

  // Search for the longest word on the page's first line.
  const words = (layer.blocks[0]?.lines[0]?.text ?? "")
    .split(/\s+/u)
    .filter((w) => /\p{L}{3,}/u.test(w));
  const query =
    words.sort((a, b) => b.length - a.length)[0]?.replace(/[^\p{L}\p{M}\p{N}]/gu, "") ?? "";
  const started = performance.now();
  await runCommand("search.run", { query });
  const search = useAppStore.getState().documents.find((open) => open.id === documentId)?.search;
  const searchMs = performance.now() - started;
  await nextFrame();
  const hitMarks = document.querySelectorAll('[data-page="0"] [class*="searchHit"]').length;

  const outline = await backend.documentOutline(documentId);
  const labels = await backend.pageLabels(documentId);

  // Thumbnails: the panel's first one gets its image.
  await runCommand("view.showThumbnails");
  const thumbnailDrawn = await waitFor(
    () => document.querySelector('[data-testid="thumbnails"] canvas') !== null,
    5_000,
  );
  const thumbnails = document.querySelectorAll('[data-testid="thumbnails"] button').length;

  // Editing (ADR 0014): duplicate the page, see the new revision painted,
  // then undo back to the file as opened.
  const editing = await (async () => {
    const pagesBefore = (await backend.pageSizes(documentId)).length;
    const started = performance.now();
    await runCommand("pages.duplicate");
    const pagesAfterEdit = (await backend.pageSizes(documentId)).length;
    const editMs = Math.round(performance.now() - started);
    const repainted = (await waitForPagePainted(0, 10_000)) !== null;
    await runCommand("edit.undo");
    const pagesAfterUndo = (await backend.pageSizes(documentId)).length;
    const open = useAppStore.getState().documents.find((candidate) => candidate.id === documentId);
    return {
      pagesBefore,
      pagesAfterEdit,
      pagesAfterUndo,
      editMs,
      repainted,
      modifiedAfterUndo: open?.modified ?? null,
      revision: open?.revision ?? null,
    };
  })();

  // A second window opens, and the shell has both.
  await runCommand("window.new");
  let windows: string[] | null = null;
  await waitFor(() => {
    void backend.windowLabels().then((labels) => {
      windows = labels;
    });
    return (windows?.length ?? 0) >= 2;
  }, 5_000);

  return {
    textQuality: layer.quality,
    textBlocks: layer.blocks.length,
    selection,
    selectionMatchesLayer: selection === layerText,
    selectionRects,
    selectionToolbar: toolbar,
    query,
    searchStatus: search?.status ?? null,
    searchHits: search?.hits.length ?? 0,
    searchHitsOnPage1: search?.hits.filter((hit) => hit.page === 0).length ?? 0,
    searchHitMarks: hitMarks,
    searchMs,
    outlineItems: outline.length,
    labels: labels?.slice(0, 5) ?? null,
    thumbnailDrawn,
    thumbnails,
    editing,
    windows,
  };
}

/**
 * UI half of the developer self-test (see src-tauri/src/diagnostics.rs).
 * When the shell says a self-test is requested, wait for the startup
 * document's first page to be sharp on screen, then report what is actually
 * there: layout geometry, directions, and the page's pixels themselves.
 * With `SAFHA_SELFTEST_SPIKE=scroll`, run the scroll benchmark instead.
 */
export async function runSelftestIfRequested(): Promise<void> {
  const config = await backend.selftestConfig();
  if (!config) return;
  // Only the main window runs it; windows it opens itself must not.
  if (isTauri() && getCurrentWebviewWindow().label !== "main") return;

  const store = useAppStore.getState();
  store.setLocale(config.locale === "ar" ? "ar" : "en");
  store.setTheme(config.theme === "dark" ? "dark" : "light");

  void backend.selftestLog(`started (${document.visibilityState})`);
  const painted = await waitForPagePainted(0, 15_000);
  void backend.selftestLog(`first page painted: ${JSON.stringify(painted)}`);
  // Let layout settle after the paint (fit-width, fonts).
  await nextFrame();
  await nextFrame();

  if (config.spike === "tile-transport" && painted) {
    const { runTileTransportSpike } = await import("./spikes/tileTransport");
    await backend.selftestReport(await runTileTransportSpike(painted.documentId), null);
    return;
  }
  if (config.spike === "print" && painted) {
    // Print the open document. On Linux the shell resolves once GTK has
    // finished (with SAFHA_PRINT_EXPORT, into a PDF file the harness checks;
    // ADR 0013). On macOS and Windows it resolves once the print window opens,
    // so the harness waits for screenshots (ADR 0012).
    const started = performance.now();
    let error: string | null = null;
    try {
      await backend.printDocument(painted.documentId);
    } catch (caught) {
      error = JSON.stringify(caught);
    }
    const printMs = Math.round(performance.now() - started);
    void backend.selftestLog(`print_document returned after ${printMs} ms`);
    if (!navigator.userAgent.includes("Linux")) {
      await new Promise((resolve) => window.setTimeout(resolve, 12_000));
    }
    await backend.selftestReport({ spike: "print", error, printMs }, null);
    return;
  }
  if (config.spike?.startsWith("tile-memory:") && painted) {
    const { runTileMemorySpike } = await import("./spikes/tileMemory");
    const variant = config.spike.slice("tile-memory:".length);
    try {
      await backend.selftestReport(await runTileMemorySpike(painted.documentId, variant), null);
    } catch (error) {
      void backend.selftestLog(`tile-memory failed: ${String(error)}`);
      await backend.selftestReport({ spike: "tile-memory", variant, error: String(error) }, null);
    }
    return;
  }
  if (config.spike === "scroll" && painted) {
    const { runScrollBenchmark } = await import("./benchmarks/scroll");
    await backend.selftestReport(await runScrollBenchmark(painted), null);
    return;
  }

  const rect = (testId: string) => {
    const box = document.querySelector(`[data-testid="${testId}"]`)?.getBoundingClientRect();
    return box ? { x: box.x, y: box.y, width: box.width, height: box.height } : null;
  };
  const state = useAppStore.getState();
  const composite = painted ? compositePage(painted.documentId, painted) : null;
  const viewerElement = document.querySelector('[data-testid="viewer"]');

  const results = {
    painted,
    locale: state.locale,
    dir: document.documentElement.dir,
    viewerDirection: viewerElement ? getComputedStyle(viewerElement).direction : null,
    visibility: document.visibilityState,
    theme: document.documentElement.dataset.theme,
    documents: state.documents.map(({ name, pageCount, zoom, pageSize, viewMode, pageOrder }) => ({
      name,
      pageCount,
      zoom,
      pageSize,
      viewMode,
      pageOrder,
    })),
    errorCode: state.errorCode,
    window: { width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio },
    toolRail: rect("tool-rail"),
    inspector: rect("inspector"),
    viewerCss: rect("viewer"),
    pageCss: rect("page-0"),
    canvasPixels: composite
      ? { width: composite.canvas.width, height: composite.canvas.height }
      : null,
    tiles: composite?.tiles ?? 0,
    inkRatio: composite?.inkRatio ?? null,
    viewer: painted ? getViewer(painted.documentId)?.snapshot() : null,
    renderStats: await backend.renderStats(),
    uiReadyMs: performance.now(),
    fontsLoaded: [...document.fonts]
      .filter((font) => font.status === "loaded")
      .map((f) => f.family),
  };

  const canvasPng = composite ? composite.canvas.toDataURL("image/png") : null;
  const navigation = painted ? await navigationChecks(painted.documentId) : null;
  await backend.selftestReport({ ...results, navigation }, canvasPng);
}
