import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend, KeptRequest, Priority, RenderedPage, TileRequest } from "../backend";
import { mockBlockLayout } from "../backend/mock";
import { ViewerController, type ViewerMode, type ViewState } from "./controller";

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
          ]
        : [],
    updateViewport: async (_doc: number, _gen: number, keep: KeptRequest[]) => {
      updates.push(keep);
    },
    // Page 0's text: an editable block, and one on a broken character mapping.
    textBlocks: async (_doc: number, page: number) =>
      page === 0
        ? [
            { rect: { x: 20, y: 20, width: 200, height: 30 }, refusal: null },
            { rect: { x: 20, y: 100, width: 200, height: 30 }, refusal: "needsOcr" },
          ]
        : [],
    layoutBlockEdit: async (_token: number, text: string) =>
      mockBlockLayout(text, 200, 12, false, 20),
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
    onPlaceText: vi.fn(),
    onEditBlock: vi.fn(),
    onBlockCommit: vi.fn(),
    onBlockCancel: vi.fn(),
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
      blockEditor: "block-editor",
      blockMask: "block-mask",
      blockCanvas: "block-canvas",
      blockField: "block-field",
      blockEditBar: "block-edit-bar",
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

  it("edits the page's own text where it is pressed, and places new text elsewhere", async () => {
    mode = "text";
    const { viewer, container, callbacks } = setup();
    await settle();
    const content = container.firstElementChild as HTMLElement;
    const press = async (x: number, y: number, target: EventTarget = content) => {
      for (const type of ["pointerdown", "pointerup"]) {
        target.dispatchEvent(
          new MouseEvent(type, { bubbles: true, clientX: 350 + x, clientY: 20 + y, button: 0 }),
        );
      }
      await settle();
    };
    // On a block: it opens. On one that cannot be edited: why not. Elsewhere: new text.
    await press(50, 30);
    expect(callbacks.onEditBlock).toHaveBeenLastCalledWith(0, 0, null);
    await press(50, 110);
    expect(callbacks.onEditBlock).toHaveBeenLastCalledWith(0, 1, "needsOcr");
    await press(50, 300);
    expect(callbacks.onPlaceText).toHaveBeenCalledWith(0, 50, 300, 240);
    expect(callbacks.onEditBlock).toHaveBeenCalledTimes(2);

    const text = "Rent is due monthly";
    viewer.openBlockEditor(
      {
        token: 7,
        text,
        font: { original: "Helvetica", used: "IBMPlexSans", kind: "substitute" },
        x: 20,
        width: 200,
        original: { x: 20, y: 20, width: 200, height: 30 },
        size: 12,
        color: [0, 0, 0],
        mixed: false,
        rtl: false,
        layout: mockBlockLayout(text, 200, 12, false, 20),
      },
      0,
    );
    const page = container.querySelector<HTMLElement>('[data-page="0"]');
    const field = page?.querySelector<HTMLTextAreaElement>(".block-field");
    expect(page?.querySelector(".block-mask")).not.toBeNull();
    expect(field?.value).toBe(text);
    expect(document.activeElement).toBe(field);
    expect(viewer.blockEditBarHost.hidden).toBe(false);
    expect(viewer.blockEditorText()).toBe(text);

    // Typing goes to the field; a press inside the editor neither commits nor places.
    if (field) {
      field.value = `${text} now`;
      field.dispatchEvent(new Event("input"));
    }
    expect(viewer.blockEditorText()).toBe(`${text} now`);
    await press(60, 30, page?.querySelector(".block-canvas") ?? content);
    expect(callbacks.onBlockCommit).not.toHaveBeenCalled();
    // A press elsewhere commits, and does nothing else.
    await press(50, 300);
    expect(callbacks.onBlockCommit).toHaveBeenCalledTimes(1);
    expect(callbacks.onPlaceText).toHaveBeenCalledTimes(1);
    // Escape in the field cancels.
    field?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(callbacks.onBlockCancel).toHaveBeenCalledTimes(1);

    viewer.closeBlockEditor(false);
    expect(page?.querySelector(".block-field")).toBeNull();
    expect(page?.querySelector(".block-mask")).toBeNull();
    expect(viewer.blockEditBarHost.hidden).toBe(true);
    expect(viewer.blockEditorText()).toBeNull();
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
