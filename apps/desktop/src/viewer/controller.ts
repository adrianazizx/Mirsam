import type { Backend, PageSize, RenderedPage, TileRequest, Urgency } from "../backend";
import { clampZoom } from "../state/zoom";
import {
  PAGE_PAINTED_EVENT,
  PAGE_PREVIEW_EVENT,
  type PagePaintedDetail,
  type PagePreviewDetail,
} from "./events";
import {
  intersect,
  pagePixelSize,
  type Rect,
  renderScale,
  tileId,
  tilesInRegion,
} from "./geometry";
import {
  type Anchor,
  anchorAt,
  computeLayout,
  currentPageIn,
  fitWidthZoom,
  type Layout,
  type PageBox,
  type PageOrder,
  resolveAnchor,
  rowsInBand,
  type ViewMode,
} from "./layout";
import { TileClient, type WantedTile } from "./tileClient";

/**
 * The document viewer: a virtualized, tiled, continuously scrolling view of
 * every page (docs/SPEC.md sections 5.1 and 9).
 *
 * Imperative on purpose. Scrolling has to stay at 60 fps with a 1,000-page
 * document, so the page slots, previews, and tile canvases are plain DOM owned
 * by this class, updated synchronously in the scroll event (which the browser
 * delivers at most once per frame, before animation-frame callbacks). React
 * owns only the scroll container (`DocumentView.tsx`).
 *
 * Per page, two layers:
 * 1. a low-resolution **preview** of the whole page at a fixed scale, requested
 *    as soon as the page comes near the viewport and stretched to the page box,
 *    so no page area stays blank while full-resolution tiles render;
 * 2. full-resolution **tiles** of `TILE_SIZE` device pixels for the part on
 *    screen (plus a margin), positioned in percentages of the page box so they
 *    stretch with it during a zoom until sharper ones replace them.
 */

export interface ViewState {
  zoomMode: "fit-width" | "custom";
  zoom: number;
  viewMode: ViewMode;
  pageOrder: PageOrder;
}

export interface ViewerSpacing {
  rowGap: number;
  padding: number;
  spreadGap: number;
}

export interface ViewerCallbacks {
  /** The page a reader would call current changed. */
  onCurrentPage(page: number, size: PageSize): void;
  /** The viewer changed the zoom itself (fit-width computed, pinch, Ctrl+wheel). */
  onZoom(zoom: number, mode: ViewState["zoomMode"]): void;
  /** Page sizes could not be read; the document cannot be shown. */
  onFatal(error: unknown): void;
}

/** Class names for the DOM the viewer creates (from the wrapper's CSS module). */
export interface ViewerClassNames {
  page: string;
  tiles: string;
  tile: string;
  preview: string;
}

export interface ViewerOptions {
  backend: Backend;
  classNames: ViewerClassNames;
  documentId: number;
  container: HTMLElement;
  initial: ViewState & { page: number };
  spacing: ViewerSpacing;
  callbacks: ViewerCallbacks;
  /** Accessible name of a page, e.g. "Page 3 of contract.pdf". */
  pageLabel(page: number): string;
  isMac: boolean;
}

/** What the benchmark and self-test read each frame. */
export interface ViewerSnapshot {
  ready: boolean;
  scrollTop: number;
  scrollLeft: number;
  viewportWidth: number;
  viewportHeight: number;
  contentHeight: number;
  zoom: number;
  scale: number;
  devicePixelRatio: number;
  currentPage: number;
  /** Pages intersecting the viewport, and whether anything is drawn on them. */
  visible: { page: number; preview: boolean; sharp: boolean }[];
  mountedPages: number;
  tileCanvases: number;
  inFlight: number;
  client: TileClient["stats"];
}

/** Device px per point of every preview: cheap, and reused across zoom levels. */
function previewScaleFor(devicePixelRatio: number): number {
  return Math.fround(0.25 * Math.min(2, Math.max(1, devicePixelRatio)));
}

/** How far beyond the viewport pages are mounted and previews fetched, in viewport heights. */
const MOUNT_MARGIN = 1;
/** How far beyond the viewport full-resolution tiles are prefetched, in viewport heights. */
const TILE_MARGIN = 0.25;
/** A zoom gesture is over once no zoom event arrived for this long; then sharp tiles are fetched. */
const ZOOM_SETTLE_MS = 140;
/** Previews kept in memory for pages that left the mount range, for instant return. */
const PREVIEW_CACHE_PAGES = 48;

interface TileCanvas {
  canvas: HTMLCanvasElement;
  scale: number;
  tile: TileRequest;
}

interface Slot {
  page: number;
  element: HTMLDivElement;
  tileLayer: HTMLDivElement;
  preview: HTMLCanvasElement | null;
  tiles: Map<string, TileCanvas>;
  box: PageBox | null;
  /** Ids of the full-resolution tiles on screen at the current scale, from the last update. */
  visibleTileIds: string[];
  sharpAt: number | null;
  firstTileRequestAt: number | null;
  renderMicros: number;
}

type Wanted = WantedTile & { kind: "preview" | "tile" };

export class ViewerController {
  private readonly backend: Backend;
  private readonly documentId: number;
  private readonly container: HTMLElement;
  private readonly content: HTMLDivElement;
  private readonly callbacks: ViewerCallbacks;
  private readonly pageLabel: (page: number) => string;
  private readonly spacing: ViewerSpacing;
  private readonly isMac: boolean;
  private readonly classNames: ViewerClassNames;
  private readonly client: TileClient;
  private readonly openedAt = performance.now();

  private sizes: PageSize[] | null = null;
  private layout: Layout | null = null;
  private view: ViewState;
  private zoom: number;
  private scale = 1;
  private devicePixelRatio = window.devicePixelRatio || 1;
  private previewScale = previewScaleFor(this.devicePixelRatio);
  private currentPage: number;
  private reportedPage: number | null = null;
  /** After a programmatic jump, the target stays current until the user scrolls. */
  private pinned: { page: number; scrollTop: number } | null = null;
  private zooming = false;
  private zoomSettleTimer: number | null = null;
  private readonly slots = new Map<number, Slot>();
  private readonly previews = new Map<number, RenderedPage>();
  private readonly previewShownAt = new Map<number, number>();
  private readonly failed = new Set<string>();
  private readonly resizeObserver: ResizeObserver;
  private dprQuery: MediaQueryList | null = null;
  private destroyed = false;

  constructor(options: ViewerOptions) {
    this.backend = options.backend;
    this.documentId = options.documentId;
    this.container = options.container;
    this.callbacks = options.callbacks;
    this.pageLabel = options.pageLabel;
    this.spacing = options.spacing;
    this.isMac = options.isMac;
    this.classNames = options.classNames;
    this.view = { ...options.initial };
    this.zoom = options.initial.zoom;
    this.currentPage = options.initial.page;

    this.content = document.createElement("div");
    this.content.dataset.viewerContent = "";
    this.content.style.position = "relative";
    this.container.append(this.content);

    this.client = new TileClient(
      this.backend,
      this.documentId,
      (wanted, bitmap) => this.onRendered(wanted as Wanted, bitmap),
      (wanted, error) => {
        this.failed.add(wanted.id);
        console.warn("tile render failed", wanted, error);
      },
    );

    this.container.addEventListener("scroll", this.onScroll, { passive: true });
    this.container.addEventListener("wheel", this.onWheel, { passive: false });
    this.resizeObserver = new ResizeObserver(() => this.onResize());
    this.resizeObserver.observe(this.container);
    this.watchDevicePixelRatio();

    this.backend.pageSizes(this.documentId).then(
      (sizes) => {
        if (this.destroyed) return;
        this.sizes = sizes;
        this.relayout(null);
        // Content wider than the view (an occasional wide page) starts centred,
        // so typical pages sit in the middle of the window.
        this.container.scrollLeft = ((this.layout?.width ?? 0) - this.container.clientWidth) / 2;
        if (this.currentPage > 0) this.goToPage(this.currentPage);
        else this.update();
      },
      (error: unknown) => {
        if (!this.destroyed) this.callbacks.onFatal(error);
      },
    );
  }

  destroy(): void {
    this.destroyed = true;
    this.client.close();
    this.container.removeEventListener("scroll", this.onScroll);
    this.container.removeEventListener("wheel", this.onWheel);
    this.resizeObserver.disconnect();
    this.dprQuery?.removeEventListener("change", this.onDevicePixelRatioChange);
    if (this.zoomSettleTimer !== null) window.clearTimeout(this.zoomSettleTimer);
    for (const slot of this.slots.values()) releaseSlot(slot);
    this.content.remove();
    this.slots.clear();
    this.previews.clear();
  }

  // ------------------------------------------------------------ public API

  /** Applies zoom and view-mode changes from the store. */
  setView(next: ViewState): void {
    const modeChanged =
      next.viewMode !== this.view.viewMode || next.pageOrder !== this.view.pageOrder;
    this.view = { ...next };
    if (!this.sizes) {
      this.zoom = next.zoom;
      return;
    }
    if (modeChanged) {
      const anchor = this.anchorForCurrentPage();
      this.relayout(anchor);
      this.update();
      return;
    }
    const target = next.zoomMode === "fit-width" ? this.fitZoom() : clampZoom(next.zoom);
    if (Math.abs(target - this.zoom) > 1e-6) this.applyZoom(target, null);
  }

  /** Scrolls so that `page` is at the top of the viewport (or shown alone in single mode). */
  goToPage(page: number): void {
    const sizes = this.sizes;
    if (!sizes || sizes.length === 0) {
      this.currentPage = page;
      return;
    }
    const target = Math.min(Math.max(0, page), sizes.length - 1);
    this.currentPage = target;
    if (this.view.viewMode === "single") {
      this.relayout(null);
      this.container.scrollTop = 0;
    } else {
      const row = this.layout?.rows[this.layout.rowOf[target] ?? -1];
      if (row) this.container.scrollTop = row.top - this.spacing.padding;
    }
    this.pinned = { page: target, scrollTop: this.container.scrollTop };
    this.reportCurrentPage(target);
    this.update();
  }

  /** The page `direction` rows away from the current one, or null at either end. */
  stepTarget(direction: 1 | -1): number | null {
    const sizes = this.sizes;
    const layout = this.layout;
    if (!sizes || !layout) return null;
    if (this.view.viewMode === "single") {
      const next = this.currentPage + direction;
      return next >= 0 && next < sizes.length ? next : null;
    }
    const row = layout.rows[(layout.rowOf[this.currentPage] ?? -1) + direction];
    return row?.pages[0] ?? null;
  }

  snapshot(): ViewerSnapshot {
    const view = this.viewRect();
    const visible: ViewerSnapshot["visible"] = [];
    if (this.layout) {
      for (const rowIndex of rowsInBand(this.layout.rows, view.y0, view.y1)) {
        for (const page of this.layout.rows[rowIndex]?.pages ?? []) {
          const box = this.layout.boxes[page];
          if (!box || !intersect(view, boxRect(box))) continue;
          const slot = this.slots.get(page);
          visible.push({
            page,
            preview: Boolean(slot?.preview),
            sharp: slot !== undefined && this.isSharp(slot),
          });
        }
      }
    }
    let tileCanvases = 0;
    for (const slot of this.slots.values()) tileCanvases += slot.tiles.size;
    return {
      ready: this.layout !== null,
      scrollTop: this.container.scrollTop,
      scrollLeft: this.container.scrollLeft,
      viewportWidth: this.container.clientWidth,
      viewportHeight: this.container.clientHeight,
      contentHeight: this.layout?.height ?? 0,
      zoom: this.zoom,
      scale: this.scale,
      devicePixelRatio: this.devicePixelRatio,
      currentPage: this.currentPage,
      visible,
      mountedPages: this.slots.size,
      tileCanvases,
      inFlight: this.client.pending,
      client: { ...this.client.stats },
    };
  }

  /** Re-reads every mounted page's accessible name (after a language change). */
  refreshLabels(): void {
    for (const slot of this.slots.values()) {
      slot.element.setAttribute("aria-label", this.pageLabel(slot.page));
    }
  }

  /** The page's on-screen canvases (preview and current-scale tiles), for the self-test's readback. */
  pageCanvases(page: number): {
    preview: HTMLCanvasElement | null;
    tiles: { canvas: HTMLCanvasElement; tile: TileRequest }[];
  } {
    const slot = this.slots.get(page);
    const tiles = slot ? [...slot.tiles.values()].filter((tile) => tile.scale === this.scale) : [];
    return {
      preview: slot?.preview ?? null,
      tiles: tiles.map(({ canvas, tile }) => ({ canvas, tile })),
    };
  }

  // ------------------------------------------------------------ events

  private readonly onScroll = () => {
    if (this.pinned && Math.abs(this.container.scrollTop - this.pinned.scrollTop) > 1) {
      this.pinned = null;
    }
    this.update();
  };

  private readonly onWheel = (event: WheelEvent) => {
    // Pinch on a trackpad arrives as a wheel event with ctrlKey (WebKit, Chromium).
    const zoomGesture = event.ctrlKey || (this.isMac && event.metaKey);
    if (!zoomGesture || !this.layout) return;
    event.preventDefault();
    const lines = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1;
    const delta = Math.max(-25, Math.min(25, event.deltaY * lines));
    const next = clampZoom(this.zoom * Math.exp(-delta * 0.01));
    if (Math.abs(next - this.zoom) < 1e-6) return;
    const bounds = this.container.getBoundingClientRect();
    this.view = { ...this.view, zoomMode: "custom", zoom: next };
    this.beginZoomGesture();
    this.applyZoom(next, { x: event.clientX - bounds.left, y: event.clientY - bounds.top });
    this.callbacks.onZoom(next, "custom");
  };

  private onResize(): void {
    if (!this.sizes || this.destroyed) return;
    if (this.view.zoomMode === "fit-width") {
      const fit = this.fitZoom();
      if (Math.abs(fit - this.zoom) > 1e-6) {
        this.applyZoom(fit, null);
        return;
      }
    }
    this.relayout(this.anchorAtViewportPoint(null));
    this.update();
  }

  private readonly onDevicePixelRatioChange = () => {
    this.watchDevicePixelRatio();
    if (!this.sizes) return;
    this.relayout(this.anchorAtViewportPoint(null));
    this.update();
  };

  private watchDevicePixelRatio(): void {
    this.dprQuery?.removeEventListener("change", this.onDevicePixelRatioChange);
    this.devicePixelRatio = window.devicePixelRatio || 1;
    this.previewScale = previewScaleFor(this.devicePixelRatio);
    if (typeof window.matchMedia !== "function") return;
    this.dprQuery = window.matchMedia(`(resolution: ${this.devicePixelRatio}dppx)`);
    this.dprQuery.addEventListener("change", this.onDevicePixelRatioChange);
  }

  private beginZoomGesture(): void {
    this.zooming = true;
    if (this.zoomSettleTimer !== null) window.clearTimeout(this.zoomSettleTimer);
    this.zoomSettleTimer = window.setTimeout(() => {
      this.zoomSettleTimer = null;
      this.zooming = false;
      this.update();
    }, ZOOM_SETTLE_MS);
  }

  // ------------------------------------------------------------ layout

  private fitZoom(): number {
    const { padding, spreadGap } = this.spacing;
    return fitWidthZoom(
      this.sizes ?? [],
      this.view.viewMode,
      this.container.clientWidth,
      padding,
      spreadGap,
    );
  }

  private viewRect(): Rect {
    const x0 = this.container.scrollLeft;
    const y0 = this.container.scrollTop;
    return { x0, y0, x1: x0 + this.container.clientWidth, y1: y0 + this.container.clientHeight };
  }

  /** Relayouts at the current zoom and restores `anchor` to where it was on screen. */
  private relayout(anchor: { anchor: Anchor; at: { x: number; y: number } } | null): void {
    const sizes = this.sizes;
    if (!sizes) return;
    if (this.view.zoomMode === "fit-width") {
      const fit = this.fitZoom();
      if (Math.abs(fit - this.zoom) > 1e-6 || this.view.zoom !== fit) {
        this.zoom = fit;
        this.view = { ...this.view, zoom: fit };
        this.callbacks.onZoom(fit, "fit-width");
      }
    }
    this.scale = renderScale(this.zoom, this.devicePixelRatio);
    this.layout = computeLayout({
      sizes,
      scale: this.scale,
      devicePixelRatio: this.devicePixelRatio,
      mode: this.view.viewMode,
      order: this.view.pageOrder,
      currentPage: this.currentPage,
      rowGap: this.spacing.rowGap,
      padding: this.spacing.padding,
      spreadGap: this.spacing.spreadGap,
      viewportWidth: this.container.clientWidth,
    });
    this.content.style.inlineSize = `${this.layout.width}px`;
    this.content.style.blockSize = `${this.layout.height}px`;
    for (const slot of this.slots.values()) {
      const box = this.layout.boxes[slot.page];
      if (box) this.placeSlot(slot, box);
    }
    if (anchor) {
      const point = resolveAnchor(this.layout, anchor.anchor);
      if (point) {
        this.container.scrollLeft = point.x - anchor.at.x;
        this.container.scrollTop = point.y - anchor.at.y;
      }
    }
    if (this.pinned) this.pinned.scrollTop = this.container.scrollTop;
  }

  /** `at` is relative to the viewport's top-left; null means its centre. */
  private anchorAtViewportPoint(
    at: { x: number; y: number } | null,
  ): { anchor: Anchor; at: { x: number; y: number } } | null {
    if (!this.layout) return null;
    const point = at ?? { x: this.container.clientWidth / 2, y: this.container.clientHeight / 2 };
    const anchor = anchorAt(
      this.layout,
      this.container.scrollLeft + point.x,
      this.container.scrollTop + point.y,
    );
    return anchor ? { anchor, at: point } : null;
  }

  /** Keeps the current page's visible top edge where it is across a mode change. */
  private anchorForCurrentPage(): { anchor: Anchor; at: { x: number; y: number } } | null {
    const box = this.layout?.boxes[this.currentPage];
    if (!box) return null;
    const fy = Math.max(0, (this.container.scrollTop + this.spacing.padding - box.y) / box.height);
    return {
      anchor: { page: this.currentPage, fx: 0.5, fy },
      at: { x: this.container.clientWidth / 2, y: this.spacing.padding },
    };
  }

  private applyZoom(zoom: number, at: { x: number; y: number } | null): void {
    const anchor = this.anchorAtViewportPoint(at);
    this.zoom = zoom;
    this.relayout(anchor);
    this.update();
  }

  // ------------------------------------------------------------ the frame update

  private update(): void {
    const layout = this.layout;
    const sizes = this.sizes;
    if (!layout || !sizes || this.destroyed) return;
    const view = this.viewRect();
    const height = view.y1 - view.y0;
    const centre = { x: (view.x0 + view.x1) / 2, y: (view.y0 + view.y1) / 2 };

    // 1. Mount the pages near the viewport, unmount the rest.
    const mountBand = { y0: view.y0 - height * MOUNT_MARGIN, y1: view.y1 + height * MOUNT_MARGIN };
    const mounted = new Set<number>();
    for (const rowIndex of rowsInBand(layout.rows, mountBand.y0, mountBand.y1)) {
      for (const page of layout.rows[rowIndex]?.pages ?? []) mounted.add(page);
    }
    for (const [page, slot] of this.slots) {
      if (!mounted.has(page)) {
        releaseSlot(slot);
        this.slots.delete(page);
      }
    }

    // 2. Decide what each mounted page needs.
    const wanted: Wanted[] = [];
    const tileBand: Rect = {
      x0: view.x0,
      x1: view.x1,
      y0: view.y0 - height * TILE_MARGIN,
      y1: view.y1 + height * TILE_MARGIN,
    };
    for (const page of mounted) {
      const box = layout.boxes[page];
      if (!box) continue;
      const slot = this.slots.get(page) ?? this.mountSlot(page, box);
      const onScreen = intersect(view, boxRect(box)) !== null;
      const distance = distanceTo(centre, box);

      if (!slot.preview) {
        const preview = this.previewTile(sizes[page]);
        if (preview) {
          wanted.push({
            kind: "preview",
            id: `preview:${tileId(page, preview)}`,
            page,
            tile: preview,
            priority: { urgency: onScreen ? "placeholder" : "prefetch", distance },
          });
        }
      }

      slot.visibleTileIds = [];
      const near = intersect(tileBand, boxRect(box));
      const pageSize = sizes[page];
      if (!near || !pageSize) {
        this.pruneTiles(slot, null);
        continue;
      }
      const pixels = pagePixelSize(pageSize, this.scale);
      const dpr = this.devicePixelRatio;
      const region = {
        x0: (near.x0 - box.x) * dpr,
        y0: (near.y0 - box.y) * dpr,
        x1: (near.x1 - box.x) * dpr,
        y1: (near.y1 - box.y) * dpr,
      };
      const keep = new Set<string>();
      for (const tile of tilesInRegion(pixels.width, pixels.height, region, this.scale)) {
        const id = tileId(page, tile);
        keep.add(id);
        const tileBox = {
          x0: box.x + tile.x / dpr,
          y0: box.y + tile.y / dpr,
          x1: box.x + (tile.x + tile.width) / dpr,
          y1: box.y + (tile.y + tile.height) / dpr,
        };
        const visible = intersect(view, tileBox) !== null;
        if (visible) slot.visibleTileIds.push(id);
        if (slot.tiles.has(id) || this.failed.has(id) || this.zooming) continue;
        const urgency: Urgency = visible ? "visible" : "prefetch";
        const tileCentre = { x: (tileBox.x0 + tileBox.x1) / 2, y: (tileBox.y0 + tileBox.y1) / 2 };
        if (slot.firstTileRequestAt === null) slot.firstTileRequestAt = performance.now();
        wanted.push({
          kind: "tile",
          id,
          page,
          tile,
          priority: {
            urgency,
            distance: Math.hypot(tileCentre.x - centre.x, tileCentre.y - centre.y),
          },
        });
      }
      this.pruneTiles(slot, keep);
      this.checkSharp(slot);
    }

    // 3. Tell the engine.
    this.client.update(wanted, (id) => this.failed.has(id));

    // 4. Current page.
    const current = this.pinned?.page ?? currentPageIn(layout, view);
    if (current !== null) this.reportCurrentPage(current);
  }

  private reportCurrentPage(page: number): void {
    this.currentPage = page;
    if (this.reportedPage === page) return;
    this.reportedPage = page;
    const size = this.sizes?.[page];
    if (size) this.callbacks.onCurrentPage(page, size);
  }

  private previewTile(size: PageSize | undefined): TileRequest | null {
    if (!size) return null;
    const { width, height } = pagePixelSize(size, this.previewScale);
    if (width <= 0 || height <= 0) return null;
    return { scale: this.previewScale, x: 0, y: 0, width, height };
  }

  // ------------------------------------------------------------ DOM

  private mountSlot(page: number, box: PageBox): Slot {
    const element = document.createElement("div");
    element.className = this.classNames.page;
    element.dataset.page = String(page);
    element.dataset.testid = `page-${page}`;
    element.setAttribute("role", "img");
    element.setAttribute("aria-label", this.pageLabel(page));
    const tileLayer = document.createElement("div");
    tileLayer.className = this.classNames.tiles;
    element.append(tileLayer);
    const slot: Slot = {
      page,
      element,
      tileLayer,
      preview: null,
      tiles: new Map(),
      box: null,
      visibleTileIds: [],
      sharpAt: null,
      firstTileRequestAt: null,
      renderMicros: 0,
    };
    this.placeSlot(slot, box);
    this.slots.set(page, slot);
    this.content.append(element);
    // A preview kept from an earlier visit is drawn at once: no round trip.
    const cached = this.previews.get(page);
    if (cached) this.drawPreview(slot, cached);
    return slot;
  }

  private placeSlot(slot: Slot, box: PageBox): void {
    const previous = slot.box;
    if (
      previous &&
      previous.x === box.x &&
      previous.y === box.y &&
      previous.width === box.width &&
      previous.height === box.height
    ) {
      return;
    }
    slot.box = box;
    const style = slot.element.style;
    // The page area is always `direction: ltr`, so inline-start is the left edge.
    style.insetInlineStart = `${box.x}px`;
    style.insetBlockStart = `${box.y}px`;
    style.inlineSize = `${box.width}px`;
    style.blockSize = `${box.height}px`;
    // The page changed size: it is sharp again only once tiles at the new scale cover it.
    slot.sharpAt = null;
    slot.firstTileRequestAt = null;
    slot.renderMicros = 0;
  }

  private onRendered(wanted: Wanted, bitmap: RenderedPage): void {
    if (wanted.kind === "preview") {
      this.rememberPreview(wanted.page, bitmap);
      const slot = this.slots.get(wanted.page);
      if (slot && !slot.preview) this.drawPreview(slot, bitmap);
      return;
    }
    const slot = this.slots.get(wanted.page);
    if (!slot || wanted.tile.scale !== this.scale || slot.tiles.has(wanted.id)) return;
    const sizes = this.sizes;
    const pageSize = sizes?.[wanted.page];
    if (!pageSize) return;
    const pixels = pagePixelSize(pageSize, this.scale);
    const canvas = bitmapCanvas(bitmap);
    canvas.className = this.classNames.tile;
    const { x, y, width, height } = wanted.tile;
    const style = canvas.style;
    style.insetInlineStart = `${(100 * x) / pixels.width}%`;
    style.insetBlockStart = `${(100 * y) / pixels.height}%`;
    style.inlineSize = `${(100 * width) / pixels.width}%`;
    style.blockSize = `${(100 * height) / pixels.height}%`;
    slot.tileLayer.append(canvas);
    slot.tiles.set(wanted.id, { canvas, scale: this.scale, tile: wanted.tile });
    slot.renderMicros += bitmap.renderMicros;
    this.checkSharp(slot);
  }

  private rememberPreview(page: number, bitmap: RenderedPage): void {
    this.previews.delete(page);
    this.previews.set(page, bitmap);
    while (this.previews.size > PREVIEW_CACHE_PAGES) {
      const oldest = this.previews.keys().next().value;
      if (oldest === undefined) break;
      this.previews.delete(oldest);
    }
  }

  private drawPreview(slot: Slot, bitmap: RenderedPage): void {
    const canvas = bitmapCanvas(bitmap);
    canvas.className = this.classNames.preview;
    slot.element.prepend(canvas);
    slot.preview = canvas;
    if (!this.previewShownAt.has(slot.page)) {
      this.previewShownAt.set(slot.page, performance.now() - this.openedAt);
    }
    const detail: PagePreviewDetail = { documentId: this.documentId, page: slot.page };
    window.dispatchEvent(new CustomEvent(PAGE_PREVIEW_EVENT, { detail }));
  }

  private isSharp(slot: Slot): boolean {
    if (slot.visibleTileIds.length === 0) return false;
    return slot.visibleTileIds.every((id) => slot.tiles.has(id));
  }

  /** Once a page is sharp at the current scale, older-scale tiles go; fires the painted event once. */
  private checkSharp(slot: Slot): void {
    if (!this.isSharp(slot)) return;
    for (const [id, tile] of slot.tiles) {
      if (tile.scale !== this.scale) {
        releaseCanvas(tile.canvas);
        slot.tiles.delete(id);
      }
    }
    if (slot.sharpAt !== null) return;
    slot.sharpAt = performance.now();
    const size = this.sizes?.[slot.page];
    if (!size) return;
    const pixels = pagePixelSize(size, this.scale);
    const detail: PagePaintedDetail = {
      documentId: this.documentId,
      page: slot.page,
      width: pixels.width,
      height: pixels.height,
      scale: this.scale,
      tiles: slot.visibleTileIds.length,
      renderMicros: slot.renderMicros,
      roundTripMs: slot.sharpAt - (slot.firstTileRequestAt ?? slot.sharpAt),
      sinceOpenMs: slot.sharpAt - this.openedAt,
      previewSinceOpenMs: this.previewShownAt.get(slot.page) ?? null,
    };
    window.dispatchEvent(new CustomEvent(PAGE_PAINTED_EVENT, { detail }));
  }

  /**
   * Drops tiles the page no longer needs: everything when the page left the
   * tile band (`keep` null), otherwise current-scale tiles outside `keep`.
   * Older-scale tiles stay as a stretched stand-in until `checkSharp` sees the
   * page covered at the new scale.
   */
  private pruneTiles(slot: Slot, keep: Set<string> | null): void {
    for (const [id, tile] of slot.tiles) {
      const drop = keep === null || (tile.scale === this.scale && !keep.has(id));
      if (drop) {
        releaseCanvas(tile.canvas);
        slot.tiles.delete(id);
      }
    }
  }
}

function boxRect(box: PageBox): Rect {
  return { x0: box.x, y0: box.y, x1: box.x + box.width, y1: box.y + box.height };
}

function distanceTo(point: { x: number; y: number }, box: PageBox): number {
  const dx =
    point.x < box.x
      ? box.x - point.x
      : point.x > box.x + box.width
        ? point.x - box.x - box.width
        : 0;
  const dy =
    point.y < box.y
      ? box.y - point.y
      : point.y > box.y + box.height
        ? point.y - box.y - box.height
        : 0;
  return Math.hypot(dx, dy);
}

/**
 * Takes a canvas off the page and frees its pixels now. WebKit otherwise keeps
 * a detached canvas's backing store until garbage collection, and a long
 * scroll discards hundreds of megabytes of tiles (spec section 11: < 500 MB).
 */
function releaseCanvas(canvas: HTMLCanvasElement): void {
  canvas.width = 0;
  canvas.height = 0;
  canvas.remove();
}

function releaseSlot(slot: Slot): void {
  for (const tile of slot.tiles.values()) releaseCanvas(tile.canvas);
  slot.tiles.clear();
  if (slot.preview) releaseCanvas(slot.preview);
  slot.preview = null;
  slot.element.remove();
}

function bitmapCanvas(bitmap: RenderedPage): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  // Pages are opaque; an opaque backing store composites faster.
  const context = canvas.getContext("2d", { alpha: false });
  context?.putImageData(new ImageData(bitmap.rgba, bitmap.width, bitmap.height), 0, 0);
  return canvas;
}
