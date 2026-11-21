import { backend } from "./backend";
import { PAGE_PAINTED_EVENT, type PagePaintedDetail } from "./components/viewer/PageView";
import { useAppStore } from "./state/appStore";

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

/**
 * UI half of the developer self-test (see src-tauri/src/diagnostics.rs).
 * When the shell says a self-test is requested, wait for the startup document's
 * first page to reach the canvas, then report what is actually on screen:
 * layout geometry, direction, and the canvas pixels themselves.
 */
export async function runSelftestIfRequested(): Promise<void> {
  const config = await backend.selftestConfig();
  if (!config) return;

  const store = useAppStore.getState();
  store.setLocale(config.locale === "ar" ? "ar" : "en");
  store.setTheme(config.theme === "dark" ? "dark" : "light");

  const painted = await new Promise<PagePaintedDetail | null>((resolve) => {
    const timeout = window.setTimeout(() => resolve(null), 15_000);
    window.addEventListener(
      PAGE_PAINTED_EVENT,
      (event) => {
        window.clearTimeout(timeout);
        resolve((event as CustomEvent<PagePaintedDetail>).detail);
      },
      { once: true },
    );
  });
  // Let layout settle after the paint (fit-width, fonts).
  await nextFrame();
  await nextFrame();

  if (config.spike === "tile-transport" && painted) {
    const { runTileTransportSpike } = await import("./spikes/tileTransport");
    await backend.selftestReport(await runTileTransportSpike(painted.documentId), null);
    return;
  }

  const rect = (testId: string) => {
    const box = document.querySelector(`[data-testid="${testId}"]`)?.getBoundingClientRect();
    return box ? { x: box.x, y: box.y, width: box.width, height: box.height } : null;
  };
  const canvas = document.querySelector<HTMLCanvasElement>('[data-testid="page-canvas"]');
  const state = useAppStore.getState();

  let inkRatio: number | null = null;
  if (canvas && canvas.width > 0) {
    const pixels = canvas.getContext("2d")?.getImageData(0, 0, canvas.width, canvas.height).data;
    if (pixels) {
      let ink = 0;
      for (let i = 0; i < pixels.length; i += 4) {
        if (
          (pixels[i] ?? 255) < 200 ||
          (pixels[i + 1] ?? 255) < 200 ||
          (pixels[i + 2] ?? 255) < 200
        ) {
          ink += 1;
        }
      }
      inkRatio = ink / (pixels.length / 4);
    }
  }

  const results = {
    painted,
    locale: state.locale,
    dir: document.documentElement.dir,
    visibility: document.visibilityState,
    theme: document.documentElement.dataset.theme,
    documents: state.documents.map(({ name, pageCount, zoom, pageSize }) => ({
      name,
      pageCount,
      zoom,
      pageSize,
    })),
    errorCode: state.errorCode,
    window: { width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio },
    toolRail: rect("tool-rail"),
    inspector: rect("inspector"),
    canvasCss: rect("page-canvas"),
    canvasPixels: canvas ? { width: canvas.width, height: canvas.height } : null,
    inkRatio,
    uiReadyMs: performance.now(),
    fontsLoaded: [...document.fonts]
      .filter((font) => font.status === "loaded")
      .map((f) => f.family),
  };

  await backend.selftestReport(
    results,
    canvas && canvas.width > 0 ? canvas.toDataURL("image/png") : null,
  );
}
