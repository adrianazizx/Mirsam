import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, KeptRequest, Priority, RenderedPage, TileRequest } from "../backend";
import { ViewerController, type ViewState } from "./controller";

/**
 * The controller against a fake backend and a stubbed 1000 x 800 viewport
 * (jsdom has no layout). 300 x 400 pt pages at zoom 0.75 are exactly
 * 300 x 400 CSS px, so rows sit at y = 20 + 410 * row.
 */

interface Call {
  page: number;
  tile: TileRequest;
  priority: Priority;
}

function setup(view: Partial<ViewState> = {}, pageCount = 10) {
  const calls: Call[] = [];
  const updates: KeptRequest[][] = [];
  const backend = {
    pageSizes: async () => Array.from({ length: pageCount }, () => ({ width: 300, height: 400 })),
    renderTile: async (
      _doc: number,
      _gen: number,
      page: number,
      tile: TileRequest,
      priority: Priority,
    ) => {
      calls.push({ page, tile, priority });
      const bitmap: RenderedPage = {
        width: tile.width,
        height: tile.height,
        renderMicros: 1,
        fromCache: false,
        image: {
          width: tile.width,
          height: tile.height,
          data: new Uint8ClampedArray(4),
        } as ImageData,
      };
      return bitmap;
    },
    updateViewport: async (_doc: number, _gen: number, keep: KeptRequest[]) => {
      updates.push(keep);
    },
  } as unknown as Backend;

  const container = document.createElement("div");
  Object.defineProperty(container, "clientWidth", { value: 1000 });
  Object.defineProperty(container, "clientHeight", { value: 800 });
  document.body.append(container);
  const callbacks = { onCurrentPage: vi.fn(), onZoom: vi.fn(), onFatal: vi.fn() };
  const viewer = new ViewerController({
    backend,
    documentId: 1,
    container,
    classNames: {
      page: "page",
      tiles: "tiles",
      tile: "tile",
      preview: "preview",
      overlay: "overlay",
      searchHit: "hit",
      searchHitActive: "hit-active",
      selection: "selection",
      selectionToolbar: "selection-toolbar",
    },
    initial: {
      zoomMode: "custom",
      zoom: 0.75,
      viewMode: "continuous",
      pageOrder: "ltr",
      page: 0,
      ...view,
    },
    spacing: { rowGap: 10, padding: 20, spreadGap: 4 },
    callbacks,
    pageLabel: (page) => `Page ${page + 1}`,
    isMac: false,
  });
  const scrollTo = (top: number) => {
    container.scrollTop = top;
    container.dispatchEvent(new Event("scroll"));
  };
  const mounted = () =>
    [...container.querySelectorAll<HTMLElement>("[data-page]")].map((el) =>
      Number(el.dataset.page),
    );
  const left = (page: number) =>
    Number.parseFloat(
      container.querySelector<HTMLElement>(`[data-page="${page}"]`)?.style.insetInlineStart ??
        "NaN",
    );
  return { viewer, container, calls, updates, callbacks, scrollTo, mounted, left };
}

const settle = async () => {
  for (let i = 0; i < 3; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
};

let getContext: typeof HTMLCanvasElement.prototype.getContext;
beforeEach(() => {
  // jsdom implements no canvas; drawing is not under test here.
  getContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = (() => null) as typeof getContext;
});
afterEach(() => {
  HTMLCanvasElement.prototype.getContext = getContext;
  document.body.replaceChildren();
});

describe("viewer controller", () => {
  it("mounts only the pages near the viewport and asks for their previews first", async () => {
    const { viewer, calls, mounted } = setup();
    await settle();
    // Visible: rows 0-1; mounted one viewport height beyond: rows 2-3.
    expect(mounted().sort()).toEqual([0, 1, 2, 3]);
    const previews = calls.filter((call) => call.tile.scale === Math.fround(0.25));
    expect(previews.map((call) => [call.page, call.priority.urgency])).toEqual([
      [0, "placeholder"],
      [1, "placeholder"],
      [2, "prefetch"],
      [3, "prefetch"],
    ]);
    const tiles = calls.filter((call) => call.tile.scale === 1);
    expect(tiles.map((call) => [call.page, call.priority.urgency])).toEqual([
      [0, "visible"],
      [1, "visible"],
      [2, "prefetch"],
    ]);
    // Everything requested has arrived: the visible pages are drawn and sharp.
    expect(viewer.snapshot().visible).toEqual([
      { page: 0, preview: true, sharp: true },
      { page: 1, preview: true, sharp: true },
    ]);
    viewer.destroy();
  });

  it("unmounts pages that scroll away and reports the most visible page", async () => {
    const { viewer, callbacks, scrollTo, mounted } = setup({}, 40);
    await settle();
    scrollTo(800); // rows 1 (30 px), 2 (400 px), 3 (350 px) on screen
    expect(callbacks.onCurrentPage).toHaveBeenLastCalledWith(2, { width: 300, height: 400 });
    // View 8220-9020; mounted band one viewport beyond: 7420-9820, rows 18-23.
    scrollTo(20 + 410 * 20);
    expect(mounted().sort((a, b) => a - b)).toEqual([18, 19, 20, 21, 22, 23]);
    expect(callbacks.onCurrentPage).toHaveBeenLastCalledWith(20, { width: 300, height: 400 });
    viewer.destroy();
  });

  it("goes to a page and steps by rows in each view mode", async () => {
    const { viewer, container, callbacks } = setup();
    await settle();
    viewer.goToPage(6);
    expect(container.scrollTop).toBe(20 + 410 * 6 - 20);
    expect(callbacks.onCurrentPage).toHaveBeenLastCalledWith(6, { width: 300, height: 400 });
    expect([viewer.stepTarget(1), viewer.stepTarget(-1)]).toEqual([7, 5]);

    const base: ViewState = {
      zoomMode: "custom",
      zoom: 0.75,
      viewMode: "spread",
      pageOrder: "ltr",
    };
    viewer.setView(base); // rows [0,1] [2,3] [4,5] [6,7] [8,9]
    expect([viewer.stepTarget(1), viewer.stepTarget(-1)]).toEqual([8, 4]);
    viewer.setView({ ...base, viewMode: "book" }); // [0] [1,2] [3,4] [5,6] [7,8] [9]
    expect([viewer.stepTarget(1), viewer.stepTarget(-1)]).toEqual([7, 3]);
    viewer.setView({ ...base, viewMode: "single" });
    expect([viewer.stepTarget(1), viewer.stepTarget(-1)]).toEqual([7, 5]);
    viewer.goToPage(9);
    expect(viewer.stepTarget(1)).toBeNull();
    viewer.destroy();
  });

  it("orders a spread right to left on request, whatever the UI direction", async () => {
    const { viewer, left } = setup({ viewMode: "spread" });
    await settle();
    expect(left(0)).toBeLessThan(left(1));
    viewer.setView({ zoomMode: "custom", zoom: 0.75, viewMode: "spread", pageOrder: "rtl" });
    expect(left(0)).toBeGreaterThan(left(1));
    viewer.destroy();
  });

  it("fits the width, and zooms with Ctrl+wheel as a custom zoom", async () => {
    const { viewer, container, callbacks } = setup({ zoomMode: "fit-width" });
    await settle();
    // (1000 - 2 * 20 - 1 px of slack) / (300 pt at 4/3 CSS px per point)
    expect(callbacks.onZoom).toHaveBeenLastCalledWith(959 / 400, "fit-width");
    container.dispatchEvent(
      new WheelEvent("wheel", {
        deltaY: -10,
        ctrlKey: true,
        clientX: 500,
        clientY: 400,
        cancelable: true,
      }),
    );
    const [zoom, mode] = callbacks.onZoom.mock.lastCall ?? [];
    expect(mode).toBe("custom");
    expect(zoom).toBeCloseTo((959 / 400) * Math.exp(0.1));
    viewer.destroy();
  });

  it("scales with a CSS transform during a zoom gesture and lays out when it ends", async () => {
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.querySelector<HTMLElement>("[data-viewer-content]");
    const pinch = () =>
      container.dispatchEvent(
        new WheelEvent("wheel", {
          deltaY: -5,
          ctrlKey: true,
          clientX: 500,
          clientY: 400,
          cancelable: true,
        }),
      );
    const height = () => content?.style.blockSize;
    pinch(); // the first step lays out at the new zoom
    const laidOut = height();
    pinch(); // later small steps only scale
    pinch();
    expect(content?.style.transform).toMatch(/^scale\(1\.\d+\)$/);
    expect(height()).toBe(laidOut);
    const zoom = callbacks.onZoom.mock.lastCall?.[0];
    expect(zoom).toBeCloseTo(0.75 * Math.exp(0.15));
    // 140 ms without zoom events ends the gesture: a real layout, no transform.
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(content?.style.transform).toBe("");
    expect(height()).not.toBe(laidOut);
    expect(viewer.snapshot().zoom).toBeCloseTo(zoom ?? 0);
    viewer.destroy();
  });

  it("cancels what it still waits for and removes its pages when destroyed", async () => {
    const { viewer, container, updates } = setup();
    viewer.destroy();
    await settle();
    expect(container.children).toHaveLength(0);
    // Nothing was in flight before the page sizes arrived, so nothing to cancel.
    expect(updates).toEqual([]);
  });
});
