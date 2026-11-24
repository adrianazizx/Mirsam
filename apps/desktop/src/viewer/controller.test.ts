import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, KeptRequest, Priority, RenderedPage, TileRequest } from "../backend";
import {
  freelyResizedRect,
  resizedRect,
  ViewerController,
  type ViewerMode,
  type ViewState,
} from "./controller";

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
            // A text box at (200, 120), 80 × 30.
            {
              index: 8,
              kind: "textBox",
              rects: [{ x: 200, y: 120, width: 80, height: 30 }],
              contents: "مربع نص",
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
    // Page 0's objects (ADR 0051): a shape under an image, and a link.
    pageObjects: async (_doc: number, page: number) =>
      page === 0
        ? [
            {
              index: 0,
              fingerprint: 10,
              kind: "path",
              rect: { x: 40, y: 200, width: 100, height: 50 },
              image: null,
              cropped: false,
              refusal: null,
            },
            {
              index: 1,
              fingerprint: 11,
              kind: "image",
              rect: { x: 100, y: 220, width: 80, height: 40 },
              image: { width: 160, height: 80 },
              cropped: false,
              refusal: null,
            },
          ]
        : [],
    pageLinks: async (_doc: number, page: number) =>
      page === 0
        ? [
            {
              index: 7,
              rects: [{ x: 20, y: 160, width: 100, height: 14 }],
              target: { kind: "page", page: 3 },
            },
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
    onMoveBlock: vi.fn(),
    onBlockSelect: vi.fn(),
    onObjectSelect: vi.fn(),
    onObjectReshape: vi.fn(),
    onCropChange: vi.fn(),
    onLinkSelect: vi.fn(),
    onLinkDraw: vi.fn(),
    onLinkReshape: vi.fn(),
    onLinkHover: vi.fn(),
    onLinkPress: vi.fn(),
    onDrawField: vi.fn(),
    onSelectField: vi.fn(),
    onMoveField: vi.fn(),
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
      gesture: "gesture",
      fillMoving: "fill-moving",
      fillHandle: "fill-handle",
      objectHover: "object-hover",
      objectSelected: "object-selected",
      objectCrop: "object-crop",
      linkArea: "link-area",
      linkSelected: "link-selected",
      linkHover: "link-hover",
      linkDraft: "link-draft",
      linkTip: "link-tip",
      fieldBox: "field-box",
      fieldBoxSelected: "field-selected",
      fieldTab: "field-tab",
      fieldDraft: "field-draft",
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
  it("names each page and, once the view rests, gives the pages on screen their text", async () => {
    const { container } = setup();
    await settle();
    const page = container.querySelector<HTMLElement>('[data-page="0"]');
    // A group (an image's content would be hidden), named, its pictures hidden.
    expect(page?.getAttribute("role")).toBe("group");
    expect(page?.getAttribute("aria-label")).toBe("Page 1");
    expect(page?.querySelector(".tiles")?.getAttribute("aria-hidden")).toBe("true");
    const readable = page?.querySelector("[data-readable]");
    expect(readable?.childElementCount).toBe(0);
    await new Promise((resolve) => setTimeout(resolve, 500));
    await settle();
    // The mock page has no text: the note says so (one paragraph).
    expect(readable?.childElementCount).toBe(1);
  });

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

  it("draws a zoom gesture into one canvas and lays out only when it ends", async () => {
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.querySelector<HTMLElement>("[data-viewer-content]");
    const pinch = (deltaY: number) =>
      container.dispatchEvent(
        new WheelEvent("wheel", {
          deltaY,
          ctrlKey: true,
          clientX: 500,
          clientY: 400,
          cancelable: true,
        }),
      );
    const height = content?.style.blockSize;
    const mounted = () => container.querySelectorAll("[data-page]").length;
    expect(mounted()).toBe(4);

    // Zooming out by e: nothing is laid out, the gesture canvas covers the
    // hidden page area, and the pages it brings into view are mounted.
    for (let step = 0; step < 4; step += 1) pinch(25);
    expect(content?.style.blockSize).toBe(height);
    expect(content?.style.transform).toBe("");
    expect(content?.dataset.zooming).toBe("");
    expect(container.querySelectorAll("canvas.gesture")).toHaveLength(1);
    expect(mounted()).toBe(9);
    // What the canvas shows: 800 / (1/e) CSS px about y = 400, rows 0 to 3.
    expect(viewer.snapshot().visible.map(({ page }) => page)).toEqual([0, 1, 2, 3]);
    const zoom = callbacks.onZoom.mock.lastCall?.[0];
    expect(zoom).toBeCloseTo(0.75 * Math.exp(-1));

    // 140 ms without zoom events ends the gesture: one real layout.
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(container.querySelector("canvas.gesture")).toBeNull();
    expect(content?.dataset.zooming).toBeUndefined();
    expect(content?.style.blockSize).not.toBe(height);
    expect(viewer.snapshot().zoom).toBeCloseTo(zoom ?? 0);
    viewer.destroy();
  });

  it("keeps the pages mounted while a zoom gesture narrows the view", async () => {
    const { viewer, container } = setup();
    await settle();
    const mounted = () => container.querySelectorAll("[data-page]").length;
    for (let step = 0; step < 4; step += 1) {
      container.dispatchEvent(
        new WheelEvent("wheel", { deltaY: -25, ctrlKey: true, clientX: 500, clientY: 400 }),
      );
    }
    // The canvas draws from them; they leave only once the gesture is over.
    expect(mounted()).toBe(4);
    expect(viewer.snapshot().visible.map(({ page }) => page)).toEqual([0, 1]);
    await new Promise((resolve) => setTimeout(resolve, 200));
    expect(container.querySelector("canvas.gesture")).toBeNull();
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
        0,
      );
      expect(container.querySelectorAll(".fill-moving")).toHaveLength(0);

      // Dragged past the page's edge, it stops at the edge.
      at("pointerdown", 160, 350);
      at("pointermove", 400, 395);
      at("pointerup", 400, 395);
      expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
        expect.anything(),
        { x: 240, y: 380, width: 60, height: 20 },
        0,
      );

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

  it("drags a Fill & Sign item onto the next page", async () => {
    mode = "select";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    // (x, y) on page 0; page 1 starts 410 px further down.
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
      );
    at("pointerdown", 5, 390);
    at("pointerup", 5, 390);
    await settle();

    // Grabbed 10 pt into it, and let go at (160, 50) on page 1.
    at("pointerdown", 160, 350);
    at("pointermove", 160, 400);
    at("pointermove", 160, 410 + 50);
    const previews = [...container.querySelectorAll(".fill-moving")];
    expect(previews).toHaveLength(1);
    expect(previews[0]?.closest(".page")).toBe(container.querySelectorAll(".page")[1]);
    at("pointerup", 160, 410 + 50);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ index: 6, page: 0 }),
      { x: 150, y: 40, width: 60, height: 20 },
      1,
    );
    expect(container.querySelectorAll(".fill-moving")).toHaveLength(0);
    viewer.destroy();
  });

  it("drags text boxes, notes and drawings with the Select tool, each kept on its own page", async () => {
    mode = "select";
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

    // The text box, grabbed at (210, 130) and let go 60 pt left and 40 pt down.
    at("pointerdown", 210, 130);
    at("pointermove", 190, 150);
    at("pointermove", 150, 170);
    expect(container.querySelectorAll(".fill-moving")).toHaveLength(1);
    at("pointerup", 150, 170);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ index: 8, kind: "textBox" }),
      { x: 140, y: 160, width: 80, height: 30 },
      0,
    );

    // The note, to the inline start.
    at("pointerdown", 255, 15);
    at("pointermove", 205, 25);
    at("pointerup", 205, 25);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ index: 5, kind: "note" }),
      { x: 200, y: 20, width: 20, height: 20 },
      0,
    );

    // The drawing, dragged onto the next page: it stops at its own page's bottom edge.
    at("pointerdown", 100, 300);
    at("pointermove", 100, 360);
    at("pointermove", 100, 410 + 50);
    const previews = [...container.querySelectorAll(".fill-moving")];
    expect(previews).toHaveLength(1);
    expect(previews[0]?.closest(".page")).toBe(container.querySelectorAll(".page")[0]);
    at("pointerup", 100, 410 + 50);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ index: 4, kind: "ink" }),
      { x: 8, y: 396, width: 194, height: 4 },
      0,
    );

    // A highlight stays on its words: a press on it selects it and drags nothing.
    at("pointerdown", 50, 20);
    at("pointermove", 90, 60);
    at("pointerup", 90, 60);
    expect(callbacks.onMoveItem).toHaveBeenCalledTimes(3);
    viewer.destroy();
  });

  it("drags only the Comment tool's own kind, and only Fill & Sign items with the Sign tool", async () => {
    mode = "note";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
      );
    const drag = (from: [number, number], to: [number, number]) => {
      at("pointerdown", ...from);
      at("pointermove", (from[0] + to[0]) / 2, (from[1] + to[1]) / 2);
      at("pointermove", ...to);
      at("pointerup", ...to);
    };
    at("pointerdown", 5, 390);
    at("pointerup", 5, 390);
    await settle();

    drag([255, 15], [255, 65]);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: "note" }),
      { x: 250, y: 60, width: 20, height: 20 },
      0,
    );
    // The note tool leaves the text box alone; the text box tool moves it.
    drag([210, 130], [210, 180]);
    expect(callbacks.onMoveItem).toHaveBeenCalledTimes(1);
    mode = "textBox";
    drag([210, 130], [210, 180]);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: "textBox" }),
      { x: 200, y: 170, width: 80, height: 30 },
      0,
    );
    // The Sign tool, armed, moves its own items only.
    mode = "place";
    drag([255, 15], [255, 65]);
    expect(callbacks.onMoveItem).toHaveBeenCalledTimes(2);
    viewer.destroy();
  });

  it("gives a text box's handles its width, a drawing's its scale, and a note none", async () => {
    mode = "select";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
      );
    const handles = () =>
      [...container.querySelectorAll<HTMLElement>(".fill-handle")].map((h) => h.dataset.corner);
    at("pointerdown", 5, 390);
    at("pointerup", 5, 390);
    await settle();

    // A note's icon keeps its size: selected, it shows no handles.
    at("pointerdown", 255, 15);
    at("pointerup", 255, 15);
    await settle();
    expect(callbacks.onAnnotationSelect).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: "note" }),
    );
    expect(handles()).toEqual([]);

    // The text box: four handles; its inline-start handle dragged out by 40 widens it to the
    // inline start, its top and height kept (the text is set again at the new width).
    at("pointerdown", 210, 130);
    at("pointerup", 210, 130);
    await settle();
    expect(handles()).toEqual(["nw", "ne", "sw", "se"]);
    expect(viewer.grabsSelected(350 + 240, 20 + 135)).toBe(true);
    expect(viewer.grabsSelected(350 + 120, 20 + 135)).toBe(false);
    at("pointerdown", 200, 150);
    at("pointermove", 180, 160);
    at("pointermove", 160, 200);
    at("pointerup", 160, 200);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: "textBox" }),
      { x: 160, y: 120, width: 120, height: 30 },
      0,
    );

    // The drawing scales in proportion about its opposite corner.
    at("pointerdown", 100, 300);
    at("pointerup", 100, 300);
    await settle();
    expect(handles()).toEqual(["nw", "ne", "sw", "se"]);
    expect(viewer.grabsSelected(350 + 202, 20 + 302)).toBe(true);
    at("pointerdown", 202, 302);
    at("pointermove", 100, 302);
    at("pointermove", 105, 300);
    at("pointerup", 105, 300);
    const [, rect] = callbacks.onMoveItem.mock.lastCall ?? [];
    expect(rect.x).toBe(8);
    expect(rect.y).toBe(298);
    expect(rect.width).toBeCloseTo(97);
    expect(rect.height).toBeCloseTo(2);
    // A drawn line thinner than an item's least side may still shrink, its length kept at 6 pt.
    expect(resizedRect(rect, "se", { x: 0, y: 0 }, 300, 400).width).toBeCloseTo(6);
    viewer.destroy();
  });

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
      0,
    );

    // The middle of the item still drags it.
    at("pointerdown", 180, 350);
    at("pointermove", 190, 350);
    at("pointerup", 190, 350);
    expect(callbacks.onMoveItem).toHaveBeenLastCalledWith(
      expect.anything(),
      { x: 160, y: 340, width: 60, height: 20 },
      0,
    );
    viewer.destroy();
  });

  it("draws, selects, moves, and resizes form fields in the form designer (ADR 0047)", async () => {
    mode = "draw";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
      );
    const box = {
      field: 7,
      widget: 0,
      page: 0,
      rect: { x: 50, y: 100, width: 100, height: 20 },
      tab: 1,
    };
    viewer.setFieldBoxes(
      [box, { ...box, widget: 1, rect: { x: 50, y: 50, width: 14, height: 14 }, tab: 0 }],
      null,
      [{ page: 0, rect: { x: 10, y: 380, width: 40, height: 10 } }],
    );
    expect(container.querySelectorAll(".field-box")).toHaveLength(2);
    expect([...container.querySelectorAll(".field-tab")].map((tab) => tab.textContent)).toEqual([
      "1",
    ]);
    expect(container.querySelectorAll(".field-draft")).toHaveLength(1);

    // A drag on an empty place draws a field, shown while it is drawn.
    at("pointerdown", 50, 200);
    at("pointermove", 120, 210);
    expect(container.querySelectorAll(".field-draft")).toHaveLength(2);
    at("pointermove", 150, 224);
    at("pointerup", 150, 224);
    expect(callbacks.onDrawField).toHaveBeenCalledWith(0, {
      x: 50,
      y: 200,
      width: 100,
      height: 24,
    });

    // A press on a field selects it; a drag moves it, kept on its page.
    at("pointerdown", 60, 105);
    expect(callbacks.onSelectField).toHaveBeenLastCalledWith(7, 0);
    at("pointermove", 80, 115);
    at("pointerup", 80, 115);
    expect(callbacks.onMoveField).toHaveBeenLastCalledWith(7, 0, 0, {
      x: 70,
      y: 110,
      width: 100,
      height: 20,
    });

    // Selected, it has four handles; the bottom-right one resizes it freely.
    viewer.setFieldBoxes([box], { field: 7, widget: 0 });
    expect(container.querySelectorAll(".field-selected")).toHaveLength(1);
    expect(container.querySelectorAll(".fill-handle")).toHaveLength(4);
    at("pointerdown", 150, 120);
    at("pointermove", 170, 150);
    at("pointermove", 200, 160);
    at("pointerup", 200, 160);
    expect(callbacks.onMoveField).toHaveBeenLastCalledWith(7, 0, 0, {
      x: 50,
      y: 100,
      width: 150,
      height: 60,
    });

    // Without a tool, a press on an empty place selects nothing and draws nothing.
    mode = "design";
    at("pointerdown", 250, 300);
    at("pointerup", 250, 300);
    expect(callbacks.onSelectField).toHaveBeenLastCalledWith(null, 0);
    expect(callbacks.onDrawField).toHaveBeenCalledTimes(1);
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

  it("drags a paragraph of page text in the Text mode, kept on its page; a press that stays edits it", async () => {
    mode = "text";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const at = (type: string, x: number, y: number) =>
      content.dispatchEvent(
        new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
      );
    // The page's paragraphs arrive when the pointer first goes over it.
    at("pointermove", 300, 300);
    await settle();

    // The editable paragraph, grabbed at (50, 30) and let go 60 pt right and 200 pt down.
    at("pointerdown", 50, 30);
    at("pointermove", 70, 80);
    at("pointermove", 110, 230);
    expect(container.querySelectorAll(".fill-moving")).toHaveLength(1);
    at("pointerup", 110, 230);
    expect(callbacks.onMoveBlock).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 0 }),
      { x: 80, y: 220, width: 200, height: 30 },
    );
    expect(callbacks.onEditBlock).not.toHaveBeenCalled();

    // Text it cannot edit (its font) still moves: its bytes are kept. Past the
    // page's bottom edge and onto the next page, it stops at its own page's edge.
    at("pointerdown", 50, 110);
    at("pointermove", 50, 300);
    at("pointermove", 50, 410 + 50);
    at("pointerup", 50, 410 + 50);
    expect(callbacks.onMoveBlock).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 1 }),
      { x: 20, y: 370, width: 200, height: 30 },
    );

    // A press that does not travel edits it, or says why it cannot.
    at("pointerdown", 50, 30);
    at("pointerup", 51, 31);
    expect(callbacks.onEditBlock).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 0 }),
    );
    at("pointerdown", 50, 110);
    at("pointerup", 50, 110);
    expect(callbacks.onBlockRefused).toHaveBeenLastCalledWith(
      expect.objectContaining({ reason: "font" }),
    );
    expect(callbacks.onPlaceText).not.toHaveBeenCalled();

    // Selected (to move with the arrow keys), it is outlined; a press elsewhere lets it go.
    viewer.selectBlock({ page: 0, rect: { x: 20, y: 20, width: 200, height: 30 } });
    expect(container.querySelectorAll("[data-testid='block-selected']")).toHaveLength(1);
    expect(callbacks.onBlockSelect).toHaveBeenLastCalledWith({
      page: 0,
      rect: { x: 20, y: 20, width: 200, height: 30 },
    });
    at("pointerdown", 250, 300);
    at("pointerup", 250, 300);
    await settle();
    expect(container.querySelectorAll("[data-testid='block-selected']")).toHaveLength(0);
    expect(callbacks.onBlockSelect).toHaveBeenLastCalledWith(null);
    viewer.destroy();
  });

  it("selects a moved paragraph again once the pages are read again", async () => {
    mode = "text";
    const { viewer, container, callbacks } = setup();
    await settle();
    viewer.reselectBlockAfterRefresh({ page: 0, rect: { x: 80, y: 220, width: 200, height: 30 } });
    viewer.refresh();
    await settle();
    expect(callbacks.onBlockSelect).toHaveBeenLastCalledWith({
      page: 0,
      rect: { x: 80, y: 220, width: 200, height: 30 },
    });
    expect(container.querySelectorAll("[data-testid='block-selected']")).toHaveLength(1);
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

describe("resizing a form field (ADR 0047)", () => {
  const rect = { x: 100, y: 100, width: 100, height: 20 };
  it("keeps the opposite corner, not the proportions", () => {
    expect(freelyResizedRect(rect, "se", { x: 250, y: 160 }, 600, 800)).toEqual({
      x: 100,
      y: 100,
      width: 150,
      height: 60,
    });
    expect(freelyResizedRect(rect, "nw", { x: 80, y: 90 }, 600, 800)).toEqual({
      x: 80,
      y: 90,
      width: 120,
      height: 30,
    });
  });

  it("stays at least 4 points, and on the page", () => {
    expect(freelyResizedRect(rect, "se", { x: 0, y: 0 }, 600, 800)).toEqual({
      x: 100,
      y: 100,
      width: 4,
      height: 4,
    });
    expect(freelyResizedRect(rect, "se", { x: 900, y: 900 }, 600, 800)).toEqual({
      x: 100,
      y: 100,
      width: 500,
      height: 700,
    });
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

describe("page objects and links (ADR 0051)", () => {
  const near = (r: { x: number; y: number; width: number; height: number }) => ({
    x: expect.closeTo(r.x, 5),
    y: expect.closeTo(r.y, 5),
    width: expect.closeTo(r.width, 5),
    height: expect.closeTo(r.height, 5),
  });
  const pointer = (container: HTMLElement) => {
    const content = container.firstElementChild as HTMLElement;
    return (type: string, x: number, y: number, shiftKey = false) =>
      content.dispatchEvent(
        new MouseEvent(type, {
          bubbles: true,
          clientX: 350 + x,
          clientY: 20 + y,
          button: 0,
          shiftKey,
        }),
      );
  };

  it("selects the topmost object pressed, drags it, and resizes it by a corner in proportion", async () => {
    mode = "objects";
    const { viewer, container, callbacks } = setup();
    await settle();
    const at = pointer(container);
    // The objects arrive on first hover.
    at("pointermove", 120, 230);
    await settle();
    at("pointermove", 120, 230);
    expect(container.querySelectorAll(".object-hover")).toHaveLength(1);

    // Where the image covers the shape, the image (drawn last) is taken.
    at("pointerdown", 120, 230);
    expect(callbacks.onObjectSelect).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 1 }),
    );
    at("pointermove", 130, 230);
    at("pointermove", 140, 250);
    expect(container.querySelectorAll(".fill-moving")).toHaveLength(1);
    at("pointerup", 140, 250);
    expect(callbacks.onObjectReshape).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 1 }),
      near({ x: 120, y: 240, width: 80, height: 40 }),
    );
    expect(container.querySelectorAll(".fill-handle")).toHaveLength(4);

    // The bottom-right corner, dragged out: twice the size, the top-left kept.
    at("pointerdown", 180, 260);
    at("pointermove", 200, 270);
    at("pointermove", 260, 300);
    at("pointerup", 260, 300);
    expect(callbacks.onObjectReshape).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 1 }),
      near({ x: 100, y: 220, width: 160, height: 80 }),
    );

    // Elsewhere: nothing selected.
    at("pointerdown", 280, 20);
    at("pointerup", 280, 20);
    expect(callbacks.onObjectSelect).toHaveBeenLastCalledWith(0, null);
    viewer.destroy();
  });

  it("crops inside the object by its handles", async () => {
    mode = "objects";
    const { viewer, container, callbacks } = setup();
    await settle();
    const at = pointer(container);
    at("pointermove", 120, 230);
    await settle();
    at("pointerdown", 120, 230);
    at("pointerup", 120, 230);
    viewer.setObjectCrop({ x: 100, y: 220, width: 80, height: 40 });
    expect(container.querySelectorAll(".object-crop")).toHaveLength(1);
    // The top-left crop handle, dragged in and past the image: kept inside it.
    at("pointerdown", 100, 220);
    at("pointermove", 110, 230);
    at("pointermove", 120, 225);
    at("pointerup", 120, 225);
    expect(callbacks.onCropChange).toHaveBeenLastCalledWith(
      near({ x: 120, y: 225, width: 60, height: 35 }),
    );
    expect(callbacks.onObjectReshape).not.toHaveBeenCalled();
    viewer.destroy();
  });

  it("draws a new link, selects one, and moves it in the link mode", async () => {
    mode = "links";
    const { viewer, container, callbacks } = setup();
    await settle();
    const at = pointer(container);
    at("pointermove", 50, 165);
    await settle();
    expect(container.querySelectorAll(".link-area")).toHaveLength(1);

    at("pointerdown", 30, 20);
    at("pointermove", 60, 30);
    at("pointermove", 130, 40);
    expect(container.querySelectorAll(".link-draft")).toHaveLength(1);
    at("pointerup", 130, 40);
    expect(callbacks.onLinkDraw).toHaveBeenLastCalledWith(
      0,
      near({ x: 30, y: 20, width: 100, height: 20 }),
    );

    at("pointerdown", 50, 165);
    expect(callbacks.onLinkSelect).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 7 }),
    );
    at("pointermove", 60, 170);
    at("pointermove", 70, 175);
    at("pointerup", 70, 175);
    expect(callbacks.onLinkReshape).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 7 }),
      near({ x: 40, y: 170, width: 100, height: 14 }),
    );
    viewer.destroy();
  });

  it("shows where a link goes on hover and follows it when pressed, with the Select tool", async () => {
    mode = "select";
    const { viewer, container, callbacks } = setup();
    await settle();
    const at = pointer(container);
    at("pointermove", 50, 165);
    await settle();
    at("pointermove", 51, 165);
    expect(callbacks.onLinkHover).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 7 }),
    );
    expect(container.querySelectorAll(".link-hover")).toHaveLength(1);
    at("pointerdown", 50, 165);
    at("pointerup", 50, 165);
    await settle();
    expect(callbacks.onLinkPress).toHaveBeenLastCalledWith(
      0,
      expect.objectContaining({ index: 7 }),
    );
    at("pointermove", 250, 20);
    expect(callbacks.onLinkHover).toHaveBeenLastCalledWith(0, null);
    viewer.destroy();
  });
});
