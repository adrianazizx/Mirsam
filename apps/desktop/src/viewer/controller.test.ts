import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, KeptRequest, Priority, RenderedPage, TileRequest } from "../backend";
import { resizedRect, ViewerController, type ViewerMode, type ViewState } from "./controller";

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

/** What a press does in the viewer under test. */
let mode: ViewerMode = "select";

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
    pageText: async () => ({ blocks: [], direction: "ltr", quality: "noText" }),
    // Page 0 has a highlight over (10, 10)-(110, 30) in page points.
    pageAnnotations: async (_doc: number, page: number) =>
      page === 0
        ? [
            {
              index: 3,
              kind: "highlight",
              rects: [{ x: 10, y: 10, width: 100, height: 20 }],
              contents: null,
              strokes: [],
            },
            // A stroke along y = 300, and a note at (250, 10).
            {
              index: 4,
              kind: "ink",
              rects: [{ x: 8, y: 298, width: 194, height: 4 }],
              contents: null,
              strokes: [
                [
                  [10, 300],
                  [200, 300],
                ],
              ],
            },
            {
              index: 5,
              kind: "note",
              rects: [{ x: 250, y: 10, width: 20, height: 20 }],
              contents: "ملاحظة",
              strokes: [],
            },
            // A signature placed with Fill & Sign (ADR 0045).
            {
              index: 6,
              kind: "signature",
              rects: [{ x: 150, y: 340, width: 60, height: 20 }],
              contents: null,
              strokes: [],
            },
          ]
        : [],
    updateViewport: async (_doc: number, _gen: number, keep: KeptRequest[]) => {
      updates.push(keep);
    },
    // Page 0's text: a paragraph to edit, and text in a font that cannot be read.
    pageTextBlocks: async (_doc: number, page: number) =>
      page === 0
        ? [
            { index: 0, x: 20, y: 20, width: 200, height: 30, editable: true, reason: null },
            { index: 1, x: 20, y: 100, width: 200, height: 30, editable: false, reason: "font" },
          ]
        : [],
  } as unknown as Backend;

  const container = document.createElement("div");
  Object.defineProperty(container, "clientWidth", { value: 1000 });
  Object.defineProperty(container, "clientHeight", { value: 800 });
  document.body.append(container);
  const callbacks = {
    onCurrentPage: vi.fn(),
    onZoom: vi.fn(),
    onFatal: vi.fn(),
    onAnnotationSelect: vi.fn(),
    onInk: vi.fn(),
    onErase: vi.fn(),
    onPlaceNote: vi.fn(),
    onPlaceTextBox: vi.fn(),
    onPlace: vi.fn(),
    onMoveItem: vi.fn(),
    onPlaceText: vi.fn(),
    onEditBlock: vi.fn(),
    onBlockRefused: vi.fn(),
  };
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
      markupSelected: "markup-selected",
      inkPreview: "ink-preview",
      inkErasing: "ink-erasing",
      notePending: "note-pending",
      textBoxPending: "text-box-pending",
      noteEditor: "note-editor",
      selectionToolbar: "selection-toolbar",
      fillMoving: "fill-moving",
      fillHandle: "fill-handle",
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
    mode: () => mode,
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
  mode = "select";
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

  it("paints pages again after an annotation edit, keeping the old tiles until new ones arrive", async () => {
    const { viewer, container, calls } = setup();
    await settle();
    const canvases = () => container.querySelectorAll("[data-page='0'] canvas").length;
    const before = canvases();
    const requested = calls.length;
    viewer.refresh();
    // Nothing went blank: the old canvases stay while the page is drawn again.
    expect(canvases()).toBe(before);
    expect(viewer.snapshot().visible[0]?.sharp).toBe(false);
    const again = calls.slice(requested).filter((call) => call.page === 0);
    expect(again.map((call) => call.tile.scale).sort()).toEqual([Math.fround(0.25), 1]);
    await settle();
    // The new ones replaced them.
    expect(canvases()).toBe(before);
    expect(viewer.snapshot().visible[0]).toEqual({ page: 0, preview: true, sharp: true });
    viewer.destroy();
  });

  it("selects markup with a click, and forgets it on Escape or an annotation edit", async () => {
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    // Page 0 sits at (350, 20), one CSS px per point: page point (45, 15) is at (395, 35).
    const press = (type: string, clientX = 395) =>
      content.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX, clientY: 35, button: 0 }),
      );
    // The first click works even before the page's markup has arrived.
    press("pointerdown");
    press("pointerup");
    await settle();
    expect(callbacks.onAnnotationSelect).toHaveBeenLastCalledWith({
      page: 0,
      index: 3,
      kind: "highlight",
      contents: null,
      rect: { x: 10, y: 10, width: 100, height: 20 },
    });
    expect(viewer.currentAnnotation()?.index).toBe(3);
    expect(container.querySelectorAll(".markup-selected")).toHaveLength(1);

    // Escape's command clears it; an annotation edit forgets it too.
    viewer.clearSelection();
    expect(callbacks.onAnnotationSelect).toHaveBeenLastCalledWith(null);
    expect(container.querySelectorAll(".markup-selected")).toHaveLength(0);
    // Further along the highlight, so it is not taken for a double click.
    press("pointerdown", 420);
    press("pointerup", 420);
    await settle();
    expect(viewer.currentAnnotation()?.index).toBe(3);
    viewer.refresh();
    expect(viewer.currentAnnotation()).toBeNull();
    viewer.destroy();
  });

  it("draws with the pen on the page it started on, and keeps the stroke until the page repaints", async () => {
    mode = "pen";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    // Page 0 sits at (350, 20), one CSS px per point: page point (x, y) is at (350 + x, 20 + y).
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, {
          bubbles: true,
          clientX: 350 + x,
          clientY: 20 + y,
          button: 0,
        }),
      );
    at("pointerdown", 100, 200);
    at("pointermove", 150, 210);
    // Past the page's bottom edge (and onto the next page): kept on this page.
    at("pointermove", 200, 450);
    at("pointerup", 200, 450);
    expect(callbacks.onInk).toHaveBeenCalledWith(0, [
      [
        { x: 100, y: 200, pressure: 0.5 },
        { x: 150, y: 210, pressure: 0.5 },
        { x: 200, y: 400, pressure: 0.5 },
      ],
    ]);
    expect(container.querySelectorAll(".ink-preview path").length).toBeGreaterThan(0);
    // The stroke stays drawn until the engine's repaint covers it.
    viewer.refresh();
    expect(container.querySelectorAll(".ink-preview path").length).toBeGreaterThan(0);
    await settle();
    expect(container.querySelectorAll(".ink-preview")).toHaveLength(0);
    viewer.destroy();
  });

  it("erases the ink a drag touches, and only that", async () => {
    mode = "eraser";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, {
          bubbles: true,
          clientX: 350 + x,
          clientY: 20 + y,
          button: 0,
        }),
      );
    at("pointerdown", 50, 250);
    await settle();
    at("pointermove", 100, 299);
    expect(container.querySelectorAll(".ink-erasing")).toHaveLength(1);
    at("pointerup", 100, 299);
    expect(callbacks.onErase).toHaveBeenCalledWith(0, [4]);
    viewer.destroy();
  });

  it("places a note where it is pressed, and opens one that is pressed", async () => {
    mode = "note";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const click = async (x: number, y: number) => {
      for (const type of ["pointerdown", "pointerup"]) {
        content.dispatchEvent(
          new MouseEvent(type, {
            bubbles: true,
            clientX: 350 + x,
            clientY: 20 + y,
            button: 0,
          }),
        );
      }
      await settle();
    };
    // On empty page: the icon centred on the press.
    await click(150, 200);
    expect(callbacks.onPlaceNote).toHaveBeenCalledWith(0, 140, 190);
    viewer.showPendingNote({ page: 0, x: 140, y: 190, kind: "note", width: 20 });
    expect(container.querySelectorAll(".note-pending")).toHaveLength(1);
    expect(viewer.noteEditorHost.hidden).toBe(false);
    viewer.showPendingNote(null);
    expect(container.querySelectorAll(".note-pending")).toHaveLength(0);

    // With the text box tool: the box's corner at the press, as wide as the page allows.
    mode = "textBox";
    await click(150, 250);
    expect(callbacks.onPlaceTextBox).toHaveBeenCalledWith(0, 150, 250, 146);
    mode = "note";

    // On a note: it opens, with its text.
    await click(255, 15);
    expect(callbacks.onAnnotationSelect).toHaveBeenLastCalledWith(
      expect.objectContaining({ index: 5, kind: "note", contents: "ملاحظة" }),
    );
    expect(viewer.noteEditorHost.hidden).toBe(false);
    expect(callbacks.onPlaceNote).toHaveBeenCalledTimes(1);
    viewer.destroy();
  });

  it.each(["place", "select"] as const)(
    "drags a Fill & Sign item in %s mode, kept on its page; a press that stays selects it",
    async (pressMode) => {
      mode = pressMode;
      const { viewer, container, callbacks } = setup();
      await settle();
      const content = container.firstElementChild as HTMLElement;
      const at = (type: string, x: number, y: number) =>
        content.dispatchEvent(
          new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
        );
      // The page's annotations arrive with the first press.
      at("pointerdown", 5, 390);
      at("pointerup", 5, 390);
      await settle();
      callbacks.onPlace.mockClear();

      // Pressed at (160, 350), dragged by (+40, -100): the box follows.
      at("pointerdown", 160, 350);
      at("pointermove", 180, 300);
      at("pointermove", 200, 250);
      expect(container.querySelectorAll(".fill-moving")).toHaveLength(1);
      at("pointerup", 200, 250);
      expect(callbacks.onMoveItem).toHaveBeenCalledWith(
        expect.objectContaining({ index: 6, kind: "signature" }),
        { x: 190, y: 240, width: 60, height: 20 },
      );
      expect(container.querySelectorAll(".fill-moving")).toHaveLength(0);

      // Dragged past the page's edge, it stops at the edge.
      at("pointerdown", 160, 350);
      at("pointermove", 400, 395);
      at("pointerup", 400, 395);
      expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(expect.anything(), {
        x: 240,
        y: 380,
        width: 60,
        height: 20,
      });

      // A press that does not travel selects it, and places nothing.
      at("pointerdown", 160, 350);
      at("pointermove", 161, 351);
      at("pointerup", 161, 351);
      expect(callbacks.onAnnotationSelect).toHaveBeenLastCalledWith(
        expect.objectContaining({ index: 6, kind: "signature" }),
      );
      expect(callbacks.onMoveItem).toHaveBeenCalledTimes(2);
      expect(callbacks.onPlace).not.toHaveBeenCalled();
      viewer.destroy();
    },
  );

  it("resizes a selected Fill & Sign item by its corner handles, in proportion", async () => {
    mode = "place";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
      );
    at("pointerdown", 5, 390);
    at("pointerup", 5, 390);
    await settle();
    expect(container.querySelectorAll(".fill-handle")).toHaveLength(0);

    // Selected (a press that stays), it shows a handle at each corner.
    at("pointerdown", 180, 350);
    at("pointerup", 180, 350);
    const handles = [...container.querySelectorAll<HTMLElement>(".fill-handle")];
    expect(handles.map((handle) => handle.dataset.corner)).toEqual(["nw", "ne", "sw", "se"]);

    // The bottom-right handle, dragged out by half: half as large again, the top-left kept.
    at("pointerdown", 211, 361);
    at("pointermove", 230, 365);
    at("pointermove", 240, 370);
    expect(container.querySelectorAll(".fill-moving")).toHaveLength(1);
    at("pointerup", 240, 370);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ index: 6, kind: "signature" }),
      { x: 150, y: 340, width: 90, height: 30 },
    );

    // The middle of the item still drags it.
    at("pointerdown", 180, 350);
    at("pointermove", 190, 350);
    at("pointerup", 190, 350);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(expect.anything(), {
      x: 160,
      y: 340,
      width: 60,
      height: 20,
    });
    viewer.destroy();
  });

  it("edits the page's text where pressed, says why where it cannot, and places new text elsewhere", async () => {
    mode = "text";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const press = async (x: number, y: number) => {
      for (const type of ["pointerdown", "pointerup"]) {
        content.dispatchEvent(
          new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
        );
      }
      await settle();
    };
    await press(50, 30);
    expect(callbacks.onEditBlock).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 0 }),
    );
    // On text it cannot edit: the reason, and no new text on top of it.
    await press(50, 110);
    expect(callbacks.onBlockRefused).toHaveBeenLastCalledWith(
      expect.objectContaining({ reason: "font" }),
    );
    expect(callbacks.onPlaceText).not.toHaveBeenCalled();
    await press(50, 300);
    expect(callbacks.onPlaceText).toHaveBeenCalledTimes(1);
    expect(callbacks.onEditBlock).toHaveBeenCalledTimes(1);
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

describe("resizing a Fill & Sign item (ADR 0045)", () => {
  const rect = { x: 100, y: 100, width: 60, height: 20 };

  it("keeps the opposite corner and the item's proportions", () => {
    // Out from the bottom-right corner: the farther axis decides.
    expect(resizedRect(rect, "se", { x: 220, y: 125 }, 600, 800)).toEqual({
      x: 100,
      y: 100,
      width: 120,
      height: 40,
    });
    // In from the top-left corner: the bottom-right stays.
    expect(resizedRect(rect, "nw", { x: 130, y: 110 }, 600, 800)).toEqual({
      x: 130,
      y: 110,
      width: 30,
      height: 10,
    });
  });

  it("stays at least 6 points and on the page", () => {
    // Dragged past the anchor: the smallest it may be.
    expect(resizedRect(rect, "se", { x: 50, y: 50 }, 600, 800)).toEqual({
      x: 100,
      y: 100,
      width: 18,
      height: 6,
    });
    // Dragged off a 200 × 800 page: as large as fits.
    expect(resizedRect(rect, "ne", { x: 400, y: 0 }, 200, 800).width).toBeCloseTo(100);
    expect(resizedRect(rect, "ne", { x: 400, y: 0 }, 200, 800).y).toBeCloseTo(120 - 100 / 3);
  });
});
