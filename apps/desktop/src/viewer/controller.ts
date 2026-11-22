import {
  type Backend,
  type MarkedPage,
  type MarkupKind,
  type PageMarkup,
  type PageSize,
  type RenderedPage,
  releaseRendered,
  renderedCanvas,
  type SearchHit,
  type TextLayer,
  type TileRequest,
  type Urgency,
  type ViewRect,
} from "../backend";
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
import {
  type Caret,
  caretAt,
  highlights,
  isCollapsed,
  isOverText,
  lineSelection,
  pageSelection,
  type Selection,
  selectedText,
  selectionPages,
  wordAt,
} from "./selection";
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
  /** Text became selected, or the selection was cleared. */
  onSelectionChange?(hasSelection: boolean): void;
  /** A markup annotation was selected (clicked), or the selection was cleared. */
  onMarkupSelect?(selected: SelectedMarkup | null): void;
}

/** A markup annotation the reader selected by clicking it. */
export interface SelectedMarkup {
  page: number;
  /** Its position in the page's annotations (what removing it takes). */
  index: number;
  kind: MarkupKind;
}

/** Class names for the DOM the viewer creates (from the wrapper's CSS module). */
export interface ViewerClassNames {
  page: string;
  tiles: string;
  tile: string;
  preview: string;
  /** Layer above the tiles for search hits and the selection. */
  overlay: string;
  searchHit: string;
  searchHitActive: string;
  selection: string;
  /** Outline of a selected markup annotation. */
  markupSelected: string;
  /** Anchor element the selection toolbar is rendered into. */
  selectionToolbar: string;
}

export interface ViewerOptions {
  backend: Backend;
  classNames: ViewerClassNames;
  documentId: number;
  container: HTMLElement;
  /** `offset`: how far down `page` the view starts (0–1). */
  initial: ViewState & { page: number; offset?: number };
  spacing: ViewerSpacing;
  callbacks: ViewerCallbacks;
  /** Accessible name of a page, e.g. "Page 3 of contract.pdf". */
  pageLabel(page: number): string;
  isMac: boolean;
  /** Whether dragging selects text (the Select tool is active). */
  canSelect?(): boolean;
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
/**
 * During a zoom gesture the page area is scaled with a CSS transform, which
 * WebKit applies on the compositor without repainting anything. A real
 * relayout (pages resized, pages mounted) happens only once the gesture has
 * scaled the layout this far, or this long after the last one, and when it
 * ends. Resizing every page on every wheel event made WebKit repaint dozens of
 * stretched canvases per frame: half the frames of a pinch took over 20 ms and
 * the webview grew by 160 MB (ADR 0011).
 */
const GESTURE_MIN_RATIO = 0.5;
const GESTURE_MAX_RATIO = 1.5;
const GESTURE_RELAYOUT_MS = 250;
/**
 * Preview canvases kept (detached) for pages that left the mount range, so a
 * page that comes back — scrolling back, or many at once while zooming out —
 * shows its preview with no round trip and no redraw.
 */
const PREVIEW_CACHE_PAGES = 48;
/** Text layers kept for selection; pages inside the selection are never dropped. */
const TEXT_LAYER_PAGES = 64;
/** While selecting, the view scrolls when the pointer is this close to an edge (CSS px). */
const AUTOSCROLL_EDGE = 24;
/** Presses closer than this in time and place count as a double or triple click. */
const MULTI_CLICK_MS = 500;
const MULTI_CLICK_DISTANCE = 6;

interface TileCanvas {
  canvas: HTMLCanvasElement;
  scale: number;
  tile: TileRequest;
}

interface Slot {
  page: number;
  element: HTMLDivElement;
  tileLayer: HTMLDivElement;
  overlay: HTMLDivElement;
  preview: HTMLCanvasElement | null;
  /** The preview from before a refresh, shown until the new one arrives. */
  stalePreview: HTMLCanvasElement | null;
  tiles: Map<string, TileCanvas>;
  box: PageBox | null;
  /** Ids of the full-resolution tiles on screen at the current scale, from the last update. */
  visibleTileIds: string[];
  sharpAt: number | null;
  firstTileRequestAt: number | null;
  renderMicros: number;
}

type Wanted = WantedTile & { kind: "preview" | "tile" };

/** A point on a page in displayed-page points. */
interface PagePoint {
  page: number;
  x: number;
  y: number;
}

interface HitMark {
  rects: ViewRect[];
  active: boolean;
}

/**
 * Time the viewer itself spends, by kind, while a benchmark profiles it
 * (`ViewerController.profile`). WKWebView's clock has 1 ms resolution, so
 * single calls read 0 or 1; sums over many calls are what is meaningful.
 */
export interface ViewerProfile {
  updates: number;
  updateMs: number;
  relayouts: number;
  relayoutMs: number;
  draws: number;
  drawMs: number;
  mounts: number;
  /** Longest single update, relayout, or draw. */
  longestMs: number;
}

export function emptyProfile(): ViewerProfile {
  return {
    updates: 0,
    updateMs: 0,
    relayouts: 0,
    relayoutMs: 0,
    draws: 0,
    drawMs: 0,
    mounts: 0,
    longestMs: 0,
  };
}

export class ViewerController {
  /** Set by benchmarks to count where the viewer's time goes; null otherwise. */
  profile: ViewerProfile | null = null;

  private readonly backend: Backend;
  private readonly documentId: number;
  private readonly container: HTMLElement;
  private readonly content: HTMLDivElement;
  private readonly callbacks: ViewerCallbacks;
  private readonly pageLabel: (page: number) => string;
  private readonly spacing: ViewerSpacing;
  private readonly isMac: boolean;
  private readonly classNames: ViewerClassNames;
  private client: TileClient;
  private readonly openedAt = performance.now();

  private sizes: PageSize[] | null = null;
  /** Position within the first page to restore once page sizes arrive. */
  private initialOffset = 0;
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
  /**
   * A zoom gesture in progress: the layout is at `layoutZoom`, and a CSS
   * scale about `origin` (content px, the point under the pointer when the
   * layout was made) shows `this.zoom`. `at` is the pointer (viewport px).
   */
  private gesture: {
    layoutZoom: number;
    origin: { x: number; y: number };
    at: { x: number; y: number };
    laidOutAt: number;
  } | null = null;
  private readonly slots = new Map<number, Slot>();
  /** Detached preview canvases by page, least recently used first. */
  private readonly previews = new Map<number, HTMLCanvasElement>();
  private readonly previewShownAt = new Map<number, number>();
  private readonly failed = new Set<string>();
  private readonly resizeObserver: ResizeObserver;
  private dprQuery: MediaQueryList | null = null;
  private destroyed = false;

  private readonly canSelect: () => boolean;
  private readonly layers = new Map<number, TextLayer>();
  private readonly layerRequests = new Set<number>();
  private selection: Selection | null = null;
  /** A press on a page whose text had not arrived yet; becomes the anchor when it does. */
  private pendingAnchor: PagePoint | null = null;
  private dragging: { pointerId: number; clientX: number; clientY: number } | null = null;
  private autoScrollFrame: number | null = null;
  /** For counting double and triple clicks (`PointerEvent.detail` is always 0 in Chromium). */
  private lastPress: { time: number; x: number; y: number; count: number } | null = null;
  private selectionReported = false;
  private readonly searchMarks = new Map<number, HitMark[]>();
  private readonly selectionRects = new Map<number, ViewRect[]>();
  /** Markup annotations of recently visited pages, for clicking them. */
  private readonly markup = new Map<number, PageMarkup[]>();
  private readonly markupRequests = new Map<number, Promise<void>>();
  /** Answers about markup from before the last refresh are dropped. */
  private markupGeneration = 0;
  private selectedMarkup: SelectedMarkup | null = null;
  /** Where the last plain click pressed: markup there is selected if no text was. */
  private pressedPoint: PagePoint | null = null;
  /** Counts presses, so a late answer about markup applies only to its own click. */
  private presses = 0;
  /** Where the selection toolbar is drawn: above the selection's first line. */
  readonly selectionToolbarHost: HTMLDivElement;

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
    this.initialOffset = options.initial.offset ?? 0;

    this.canSelect = options.canSelect ?? (() => true);
    this.content = document.createElement("div");
    this.content.dataset.viewerContent = "";
    this.content.style.position = "relative";
    this.container.append(this.content);
    this.selectionToolbarHost = document.createElement("div");
    this.selectionToolbarHost.className = this.classNames.selectionToolbar;
    this.selectionToolbarHost.hidden = true;
    this.content.append(this.selectionToolbarHost);

    this.client = this.newClient();

    this.container.addEventListener("scroll", this.onScroll, { passive: true });
    this.container.addEventListener("wheel", this.onWheel, { passive: false });
    this.content.addEventListener("pointerdown", this.onPointerDown);
    this.content.addEventListener("pointermove", this.onPointerMove);
    this.content.addEventListener("pointerup", this.onPointerUp);
    this.content.addEventListener("pointercancel", this.onPointerUp);
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
        if (this.currentPage > 0 || this.initialOffset > 0) {
          this.goToPage(this.currentPage, this.initialOffset);
        } else this.update();
      },
      (error: unknown) => {
        if (!this.destroyed) this.callbacks.onFatal(error);
      },
    );
  }

  private newClient(): TileClient {
    return new TileClient(
      this.backend,
      this.documentId,
      (wanted, bitmap) => this.onRendered(wanted as Wanted, bitmap),
      (wanted, error) => {
        this.failed.add(wanted.id);
        console.warn("tile render failed", wanted, error);
      },
    );
  }

  destroy(): void {
    this.destroyed = true;
    this.client.close();
    this.container.removeEventListener("scroll", this.onScroll);
    this.container.removeEventListener("wheel", this.onWheel);
    this.content.removeEventListener("pointerdown", this.onPointerDown);
    this.content.removeEventListener("pointermove", this.onPointerMove);
    this.content.removeEventListener("pointerup", this.onPointerUp);
    this.content.removeEventListener("pointercancel", this.onPointerUp);
    this.stopAutoScroll();
    this.resizeObserver.disconnect();
    this.dprQuery?.removeEventListener("change", this.onDevicePixelRatioChange);
    if (this.zoomSettleTimer !== null) window.clearTimeout(this.zoomSettleTimer);
    for (const slot of this.slots.values()) releaseSlot(slot);
    this.content.remove();
    this.slots.clear();
    for (const canvas of this.previews.values()) releaseCanvas(canvas);
    this.previews.clear();
  }

  // ------------------------------------------------------------ public API

  /**
   * The document's annotations changed but not its pages (markup added or
   * removed, or that undone): every page is rendered again. What is on
   * screen stays until the new renders cover it, so nothing flashes.
   */
  refresh(): void {
    if (this.destroyed) return;
    // Answers still on their way show the old revision: a new client ignores them.
    this.client.close();
    this.client = this.newClient();
    this.failed.clear();
    for (const canvas of this.previews.values()) releaseCanvas(canvas);
    this.previews.clear();
    for (const slot of this.slots.values()) {
      // Stale tiles keep their place under new ids, at no scale: `checkSharp`
      // drops them once the page's new tiles cover it.
      const stale = new Map<string, TileCanvas>();
      for (const [id, tile] of slot.tiles) stale.set(`stale:${id}`, { ...tile, scale: Number.NaN });
      slot.tiles = stale;
      if (slot.preview) {
        if (slot.stalePreview) releaseCanvas(slot.stalePreview);
        slot.stalePreview = slot.preview;
        slot.preview = null;
      }
      slot.sharpAt = null;
      slot.firstTileRequestAt = null;
      slot.renderMicros = 0;
    }
    this.markupGeneration += 1;
    this.markup.clear();
    this.markupRequests.clear();
    this.setSelectedMarkup(null);
    this.update();
  }

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
      this.settleGesture();
      const anchor = this.anchorForCurrentPage();
      this.relayout(anchor);
      this.update();
      return;
    }
    const target = next.zoomMode === "fit-width" ? this.fitZoom() : clampZoom(next.zoom);
    if (Math.abs(target - this.zoom) > 1e-6) {
      this.settleGesture();
      this.applyZoom(target, null);
    }
  }

  /**
   * Scrolls so that `page` is at the top of the viewport (or shown alone in
   * single mode). `offset` (0–1) starts the view that far down the page.
   */
  goToPage(page: number, offset = 0): void {
    const sizes = this.sizes;
    if (!sizes || sizes.length === 0) {
      this.currentPage = page;
      this.initialOffset = offset;
      return;
    }
    this.settleGesture();
    const target = Math.min(Math.max(0, page), sizes.length - 1);
    this.currentPage = target;
    const fraction = Math.min(Math.max(0, offset), 1);
    if (this.view.viewMode === "single") {
      this.relayout(null);
      const box = this.layout?.boxes[target];
      this.container.scrollTop = box ? fraction * box.height : 0;
    } else {
      const row = this.layout?.rows[this.layout.rowOf[target] ?? -1];
      const box = this.layout?.boxes[target];
      if (row) {
        this.container.scrollTop = row.top - this.spacing.padding + fraction * (box?.height ?? 0);
      }
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

  /** How far down the current page the view's top edge is, 0–1 (for remembering the position). */
  currentOffset(): number {
    const box = this.layout?.boxes[this.currentPage];
    if (!box || box.height <= 0) return 0;
    const top = this.view.viewMode === "single" ? 0 : this.spacing.padding;
    return Math.min(1, Math.max(0, (this.container.scrollTop + top - box.y) / box.height));
  }

  // ------------------------------------------------------------ search hits

  /** Marks every hit on its page; `active` (an index into `hits`) stands out. */
  setSearchHits(hits: SearchHit[], active: number): void {
    const previous = new Set(this.searchMarks.keys());
    this.searchMarks.clear();
    hits.forEach((hit, index) => {
      const marks = this.searchMarks.get(hit.page) ?? [];
      marks.push({ rects: hit.rects, active: index === active });
      this.searchMarks.set(hit.page, marks);
    });
    for (const page of new Set([...previous, ...this.searchMarks.keys()])) {
      const slot = this.slots.get(page);
      if (slot) this.paintOverlay(slot);
    }
  }

  /** Scrolls the hit into view (about a third of the way down) unless it already is. */
  revealHit(hit: SearchHit): void {
    if (!this.sizes || !this.layout) return;
    this.settleGesture();
    if (this.view.viewMode === "single" && this.currentPage !== hit.page) {
      this.goToPage(hit.page);
    }
    const box = this.layout.boxes[hit.page];
    const size = this.sizes[hit.page];
    if (!box || !size || hit.rects.length === 0) {
      this.goToPage(hit.page);
      return;
    }
    const sx = box.width / size.width;
    const sy = box.height / size.height;
    const x0 = Math.min(...hit.rects.map((rect) => rect.x));
    const y0 = Math.min(...hit.rects.map((rect) => rect.y));
    const x1 = Math.max(...hit.rects.map((rect) => rect.x + rect.width));
    const y1 = Math.max(...hit.rects.map((rect) => rect.y + rect.height));
    const target = {
      x0: box.x + x0 * sx,
      y0: box.y + y0 * sy,
      x1: box.x + x1 * sx,
      y1: box.y + y1 * sy,
    };
    const view = this.viewRect();
    const margin = 16;
    if (target.y0 < view.y0 + margin || target.y1 > view.y1 - margin) {
      this.container.scrollTop = target.y0 - this.container.clientHeight / 3;
    }
    if (target.x0 < view.x0 || target.x1 > view.x1) {
      this.container.scrollLeft = (target.x0 + target.x1) / 2 - this.container.clientWidth / 2;
    }
    this.pinned = { page: hit.page, scrollTop: this.container.scrollTop };
    this.reportCurrentPage(hit.page);
    this.update();
  }

  // ------------------------------------------------------------ selection

  hasSelection(): boolean {
    return this.selection !== null;
  }

  /** The selected text in logical order (spec section 6.1). */
  selectedText(): string {
    return this.selection ? selectedText(this.selection, this.layerLookup) : "";
  }

  clearSelection(): void {
    this.pendingAnchor = null;
    this.setSelection(null);
    this.setSelectedMarkup(null);
  }

  /** The selected text's runs, page by page: what marking it covers (ADR 0017). */
  selectionMarks(): MarkedPage[] {
    return [...this.selectionRects]
      .filter(([, rects]) => rects.length > 0)
      .sort(([a], [b]) => a - b)
      .map(([page, rects]) => ({ page, rects: rects.map((rect) => ({ ...rect })) }));
  }

  /** The markup annotation selected by clicking it, if any. */
  selectedMarkupAnnotation(): SelectedMarkup | null {
    return this.selectedMarkup;
  }

  /** Selects all text on the current page. */
  selectAllOnPage(): void {
    const page = this.currentPage;
    const layer = this.layers.get(page);
    if (!layer) {
      void this.loadLayer(page).then((loaded) => {
        if (loaded && this.currentPage === page) this.setSelection(pageSelection(loaded, page));
      });
      return;
    }
    this.setSelection(pageSelection(layer, page));
  }

  private readonly layerLookup = (page: number) => this.layers.get(page);

  private loadLayer(page: number): Promise<TextLayer | null> {
    const cached = this.layers.get(page);
    if (cached) return Promise.resolve(cached);
    if (this.layerRequests.has(page)) return Promise.resolve(null);
    this.layerRequests.add(page);
    return this.backend.pageText(this.documentId, page).then(
      (layer) => {
        this.layerRequests.delete(page);
        if (this.destroyed) return null;
        this.layers.set(page, layer);
        this.trimLayers();
        this.onLayerLoaded(page, layer);
        return layer;
      },
      (error: unknown) => {
        this.layerRequests.delete(page);
        console.warn("page text unavailable", page, error);
        return null;
      },
    );
  }

  private trimLayers(): void {
    if (this.layers.size <= TEXT_LAYER_PAGES) return;
    const [first, last] = this.selection ? selectionPages(this.selection) : [-1, -1];
    for (const page of this.layers.keys()) {
      if (this.layers.size <= TEXT_LAYER_PAGES) break;
      if (page >= first && page <= last) continue;
      this.layers.delete(page);
    }
  }

  private onLayerLoaded(page: number, layer: TextLayer): void {
    const pending = this.pendingAnchor;
    if (pending && pending.page === page) {
      this.pendingAnchor = null;
      const caret = caretAt(layer, page, pending.x, pending.y);
      if (caret) {
        const focus = this.dragging
          ? this.caretAtClient(this.dragging.clientX, this.dragging.clientY)
          : null;
        this.setSelection({ anchor: caret, focus: focus ?? caret });
      }
      return;
    }
    // A selection reaching into this page can now paint (and copy) it.
    if (this.selection) {
      const [first, last] = selectionPages(this.selection);
      if (page >= first && page <= last) this.setSelection(this.selection);
    }
  }

  /** The page's markup, from the cache or asked for once; settles when it is known. */
  private loadMarkup(page: number): Promise<void> {
    const pending = this.markupRequests.get(page);
    if (this.markup.has(page)) return Promise.resolve();
    if (pending) return pending;
    const generation = this.markupGeneration;
    const request = this.backend.pageMarkup(this.documentId, page).then(
      (list) => {
        if (this.destroyed || generation !== this.markupGeneration) return;
        this.markupRequests.delete(page);
        this.markup.set(page, list);
        if (this.markup.size > TEXT_LAYER_PAGES) {
          const oldest = this.markup.keys().next().value;
          if (oldest !== undefined && oldest !== this.selectedMarkup?.page)
            this.markup.delete(oldest);
        }
      },
      (error: unknown) => {
        if (generation === this.markupGeneration) this.markupRequests.delete(page);
        console.warn("page markup unavailable", page, error);
      },
    );
    this.markupRequests.set(page, request);
    return request;
  }

  /** The markup annotation under a point, the one drawn last first. */
  private markupAt(point: PagePoint): SelectedMarkup | null {
    const list = this.markup.get(point.page) ?? [];
    for (let i = list.length - 1; i >= 0; i -= 1) {
      const markup = list[i];
      const hit = markup?.rects.some(
        (rect) =>
          point.x >= rect.x &&
          point.x <= rect.x + rect.width &&
          point.y >= rect.y &&
          point.y <= rect.y + rect.height,
      );
      if (markup && hit) return { page: point.page, index: markup.index, kind: markup.kind };
    }
    return null;
  }

  private setSelectedMarkup(selected: SelectedMarkup | null): void {
    const previous = this.selectedMarkup;
    if (previous?.page === selected?.page && previous?.index === selected?.index) return;
    this.selectedMarkup = selected;
    for (const page of new Set([previous?.page, selected?.page])) {
      const slot = page === undefined ? undefined : this.slots.get(page);
      if (slot) this.paintOverlay(slot);
    }
    this.placeSelectionToolbar();
    this.callbacks.onMarkupSelect?.(selected);
  }

  private selectedMarkupRects(): ViewRect[] {
    const selected = this.selectedMarkup;
    if (!selected) return [];
    return this.markup.get(selected.page)?.find((m) => m.index === selected.index)?.rects ?? [];
  }

  /** The page point under a client position: the nearest page, clamped to it. */
  private pointAt(clientX: number, clientY: number): PagePoint | null {
    if (!this.layout || !this.sizes) return null;
    const bounds = this.container.getBoundingClientRect();
    const x = clientX - bounds.left + this.container.scrollLeft;
    const y = clientY - bounds.top + this.container.scrollTop;
    const anchor = anchorAt(this.layout, x, y);
    const size = anchor && this.sizes[anchor.page];
    if (!anchor || !size) return null;
    return { page: anchor.page, x: anchor.fx * size.width, y: anchor.fy * size.height };
  }

  private caretAtClient(clientX: number, clientY: number): Caret | null {
    const point = this.pointAt(clientX, clientY);
    if (!point) return null;
    const layer = this.layers.get(point.page);
    if (!layer) {
      void this.loadLayer(point.page);
      return null;
    }
    return caretAt(layer, point.page, point.x, point.y);
  }

  private readonly onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0 || !this.canSelect() || !this.layout) return;
    // The selection toolbar's own buttons.
    if (event.target instanceof Node && this.selectionToolbarHost.contains(event.target)) return;
    // Clicks on the scrollbars land on the container, not the content.
    this.container.focus({ preventScroll: true });
    const point = this.pointAt(event.clientX, event.clientY);
    if (!point) return;
    event.preventDefault();
    const clicks = this.countClick(event);
    // A plain click on markup selects it, unless the press turns into a text selection.
    this.presses += 1;
    this.pressedPoint = clicks === 1 && !event.shiftKey ? point : null;
    void this.loadMarkup(point.page);
    this.setSelectedMarkup(null);
    const layer = this.layers.get(point.page);
    this.dragging = { pointerId: event.pointerId, clientX: event.clientX, clientY: event.clientY };
    this.content.setPointerCapture?.(event.pointerId);
    if (!layer) {
      this.setSelection(null);
      this.pendingAnchor = point;
      void this.loadLayer(point.page);
      return;
    }
    const caret = caretAt(layer, point.page, point.x, point.y);
    if (!caret) {
      this.setSelection(null);
      return;
    }
    if (clicks >= 3) {
      this.setSelection(lineSelection(caret));
    } else if (clicks === 2) {
      this.setSelection(wordAt(layer, point.page, point.x, point.y));
    } else if (event.shiftKey && this.selection) {
      this.setSelection({ anchor: this.selection.anchor, focus: caret });
    } else {
      this.setSelection({ anchor: caret, focus: caret });
    }
  };

  /** 1, 2, or 3 for a single, double, or triple click (presses close in time and place). */
  private countClick(event: PointerEvent): number {
    const now = event.timeStamp;
    const last = this.lastPress;
    const repeat =
      last !== null &&
      now - last.time < MULTI_CLICK_MS &&
      Math.hypot(event.clientX - last.x, event.clientY - last.y) < MULTI_CLICK_DISTANCE;
    const count = repeat && last ? (last.count % 3) + 1 : 1;
    this.lastPress = { time: now, x: event.clientX, y: event.clientY, count };
    return count;
  }

  private readonly onPointerMove = (event: PointerEvent) => {
    const dragging = this.dragging;
    if (!dragging) {
      if (event.target instanceof Node && this.selectionToolbarHost.contains(event.target)) return;
      this.updateCursor(event);
      return;
    }
    if (event.pointerId !== dragging.pointerId) return;
    dragging.clientX = event.clientX;
    dragging.clientY = event.clientY;
    this.extendSelection();
    const bounds = this.container.getBoundingClientRect();
    const outside =
      event.clientY < bounds.top + AUTOSCROLL_EDGE ||
      event.clientY > bounds.bottom - AUTOSCROLL_EDGE;
    if (outside) this.startAutoScroll();
    else this.stopAutoScroll();
  };

  private readonly onPointerUp = (event: PointerEvent) => {
    if (!this.dragging || event.pointerId !== this.dragging.pointerId) return;
    this.dragging = null;
    this.pendingAnchor = null;
    this.stopAutoScroll();
    this.content.releasePointerCapture?.(event.pointerId);
    if (this.selection && isCollapsed(this.selection, this.layerLookup)) this.setSelection(null);
    const pressed = this.pressedPoint;
    this.pressedPoint = null;
    if (!pressed || this.selection !== null) return;
    // The page's markup may still be on its way (a click right after an edit).
    const press = this.presses;
    void this.loadMarkup(pressed.page).then(() => {
      if (this.destroyed || press !== this.presses || this.selection !== null) return;
      const hit = this.markupAt(pressed);
      if (hit) this.setSelectedMarkup(hit);
    });
  };

  private extendSelection(): void {
    const dragging = this.dragging;
    if (!dragging || !this.selection) return;
    const focus = this.caretAtClient(dragging.clientX, dragging.clientY);
    if (focus) this.setSelection({ anchor: this.selection.anchor, focus });
  }

  private startAutoScroll(): void {
    if (this.autoScrollFrame !== null) return;
    const step = () => {
      const dragging = this.dragging;
      if (!dragging || this.destroyed) {
        this.autoScrollFrame = null;
        return;
      }
      const bounds = this.container.getBoundingClientRect();
      const above = bounds.top + AUTOSCROLL_EDGE - dragging.clientY;
      const below = dragging.clientY - (bounds.bottom - AUTOSCROLL_EDGE);
      const delta = above > 0 ? -above : Math.max(below, 0);
      if (delta === 0) {
        this.autoScrollFrame = null;
        return;
      }
      this.container.scrollTop += Math.max(-40, Math.min(40, delta / 2));
      this.extendSelection();
      this.autoScrollFrame = requestAnimationFrame(step);
    };
    this.autoScrollFrame = requestAnimationFrame(step);
  }

  private stopAutoScroll(): void {
    if (this.autoScrollFrame !== null) cancelAnimationFrame(this.autoScrollFrame);
    this.autoScrollFrame = null;
  }

  /** The text cursor over text; text arrives on first hover over a page. */
  private updateCursor(event: PointerEvent): void {
    if (!this.canSelect()) {
      delete this.content.dataset.overText;
      return;
    }
    const point = this.pointAt(event.clientX, event.clientY);
    const layer = point ? this.layers.get(point.page) : undefined;
    if (point && !layer) void this.loadLayer(point.page);
    if (point) void this.loadMarkup(point.page);
    const over = point !== null && layer !== undefined && isOverText(layer, point.x, point.y);
    if (over) this.content.dataset.overText = "";
    else delete this.content.dataset.overText;
  }

  private setSelection(selection: Selection | null): void {
    // Selecting text and selecting markup exclude each other.
    if (selection) this.setSelectedMarkup(null);
    this.selection = selection;
    const previous = new Set(this.selectionRects.keys());
    this.selectionRects.clear();
    if (selection) {
      for (const { page, rects } of highlights(selection, this.layerLookup)) {
        this.selectionRects.set(page, rects);
      }
    }
    for (const page of new Set([...previous, ...this.selectionRects.keys()])) {
      const slot = this.slots.get(page);
      if (slot) this.paintOverlay(slot);
    }
    this.placeSelectionToolbar();
    const has = selection !== null && this.selectionRects.size > 0;
    if (has !== this.selectionReported) {
      this.selectionReported = has;
      this.callbacks.onSelectionChange?.(has);
    }
  }

  /**
   * Puts the toolbar host above the top of the selection's first highlighted
   * line, or above a selected markup annotation.
   */
  private placeSelectionToolbar(): void {
    const host = this.selectionToolbarHost;
    let first = [...this.selectionRects.keys()].sort((a, b) => a - b)[0];
    let rects = first === undefined ? undefined : this.selectionRects.get(first);
    if (!rects?.length && this.selectedMarkup) {
      first = this.selectedMarkup.page;
      rects = this.selectedMarkupRects();
    }
    const box = first === undefined ? undefined : this.layout?.boxes[first];
    const size = first === undefined ? undefined : this.sizes?.[first];
    if (!rects || rects.length === 0 || !box || !size) {
      host.hidden = true;
      return;
    }
    const top = Math.min(...rects.map((rect) => rect.y));
    const line = rects.filter((rect) => rect.y <= top + 0.5);
    const x0 = Math.min(...line.map((rect) => rect.x));
    const x1 = Math.max(...line.map((rect) => rect.x + rect.width));
    host.hidden = false;
    host.style.insetInlineStart = `${box.x + ((x0 + x1) / 2) * (box.width / size.width)}px`;
    host.style.insetBlockStart = `${box.y + top * (box.height / size.height)}px`;
  }

  private paintOverlay(slot: Slot): void {
    const size = this.sizes?.[slot.page];
    slot.overlay.replaceChildren();
    if (!size) return;
    const place = (element: HTMLElement, rect: ViewRect) => {
      const style = element.style;
      style.insetInlineStart = `${(100 * rect.x) / size.width}%`;
      style.insetBlockStart = `${(100 * rect.y) / size.height}%`;
      style.inlineSize = `${(100 * rect.width) / size.width}%`;
      style.blockSize = `${(100 * rect.height) / size.height}%`;
    };
    for (const mark of this.searchMarks.get(slot.page) ?? []) {
      for (const rect of mark.rects) {
        const element = document.createElement("div");
        element.className = mark.active
          ? `${this.classNames.searchHit} ${this.classNames.searchHitActive}`
          : this.classNames.searchHit;
        place(element, rect);
        slot.overlay.append(element);
      }
    }
    for (const rect of this.selectionRects.get(slot.page) ?? []) {
      const element = document.createElement("div");
      element.className = this.classNames.selection;
      place(element, rect);
      slot.overlay.append(element);
    }
    if (this.selectedMarkup?.page === slot.page) {
      for (const rect of this.selectedMarkupRects()) {
        const element = document.createElement("div");
        element.className = this.classNames.markupSelected;
        place(element, rect);
        slot.overlay.append(element);
      }
    }
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
    this.zoomBy(next, { x: event.clientX - bounds.left, y: event.clientY - bounds.top });
    this.callbacks.onZoom(next, "custom");
  };

  /** One step of a zoom gesture: a CSS scale when it can, a real relayout when it must. */
  private zoomBy(next: number, at: { x: number; y: number }): void {
    const now = performance.now();
    const gesture = this.gesture;
    if (gesture) {
      const ratio = next / gesture.layoutZoom;
      if (
        ratio > GESTURE_MIN_RATIO &&
        ratio < GESTURE_MAX_RATIO &&
        now - gesture.laidOutAt < GESTURE_RELAYOUT_MS
      ) {
        this.zoom = next;
        gesture.at = at;
        // Scale the existing raster rather than re-rasterizing at each step.
        this.content.style.willChange = "transform";
        this.content.style.transformOrigin = `${gesture.origin.x}px ${gesture.origin.y}px`;
        this.content.style.transform = `scale(${ratio})`;
        return;
      }
    }
    this.clearTransform();
    this.applyZoom(next, at);
    this.gesture = {
      layoutZoom: next,
      origin: { x: this.container.scrollLeft + at.x, y: this.container.scrollTop + at.y },
      at,
      laidOutAt: now,
    };
  }

  private clearTransform(): void {
    if (!this.content.style.transform) return;
    this.content.style.transform = "";
    this.content.style.transformOrigin = "";
    this.content.style.willChange = "";
  }

  /** Ends a zoom gesture: a scaled-but-not-laid-out view gets its real layout. */
  private settleGesture(): void {
    const gesture = this.gesture;
    this.gesture = null;
    if (!gesture || !this.content.style.transform) return;
    this.clearTransform();
    this.applyZoom(this.zoom, gesture.at);
  }

  private onResize(): void {
    if (!this.sizes || this.destroyed) return;
    this.settleGesture();
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
    this.settleGesture();
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
      this.settleGesture();
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
    const profile = this.profile;
    if (!profile) {
      this.relayoutNow(anchor);
      return;
    }
    const started = performance.now();
    this.relayoutNow(anchor);
    const ms = performance.now() - started;
    profile.relayouts += 1;
    profile.relayoutMs += ms;
    profile.longestMs = Math.max(profile.longestMs, ms);
  }

  private relayoutNow(anchor: { anchor: Anchor; at: { x: number; y: number } } | null): void {
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
    this.placeSelectionToolbar();
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
    const profile = this.profile;
    if (!profile) {
      this.updateNow();
      return;
    }
    const started = performance.now();
    this.updateNow();
    const ms = performance.now() - started;
    profile.updates += 1;
    profile.updateMs += ms;
    profile.longestMs = Math.max(profile.longestMs, ms);
  }

  private updateNow(): void {
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
        this.unmountSlot(slot);
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
    if (this.profile) this.profile.mounts += 1;
    const element = document.createElement("div");
    element.className = this.classNames.page;
    element.dataset.page = String(page);
    element.dataset.testid = `page-${page}`;
    element.setAttribute("role", "img");
    element.setAttribute("aria-label", this.pageLabel(page));
    const tileLayer = document.createElement("div");
    tileLayer.className = this.classNames.tiles;
    const overlay = document.createElement("div");
    overlay.className = this.classNames.overlay;
    element.append(tileLayer, overlay);
    const slot: Slot = {
      page,
      element,
      tileLayer,
      overlay,
      preview: null,
      stalePreview: null,
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
    if (
      this.searchMarks.has(page) ||
      this.selectionRects.has(page) ||
      this.selectedMarkup?.page === page
    ) {
      this.paintOverlay(slot);
    }
    // A preview kept from an earlier visit goes straight back: no round trip, no redraw.
    const cached = this.previews.get(page);
    if (cached) {
      this.previews.delete(page);
      this.attachPreview(slot, cached);
    }
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

  /** Owns `bitmap`: it is drawn once and released here whatever happens. */
  private onRendered(wanted: Wanted, bitmap: RenderedPage): void {
    const profile = this.profile;
    if (!profile) {
      this.drawRendered(wanted, bitmap);
      return;
    }
    const started = performance.now();
    this.drawRendered(wanted, bitmap);
    const ms = performance.now() - started;
    profile.draws += 1;
    profile.drawMs += ms;
    profile.longestMs = Math.max(profile.longestMs, ms);
  }

  private drawRendered(wanted: Wanted, bitmap: RenderedPage): void {
    if (wanted.kind === "preview") {
      const slot = this.slots.get(wanted.page);
      if (slot?.preview) {
        releaseRendered(bitmap);
        return;
      }
      const canvas = renderedCanvas(bitmap);
      releaseRendered(bitmap);
      canvas.className = this.classNames.preview;
      if (slot) this.attachPreview(slot, canvas);
      else this.cachePreview(wanted.page, canvas);
      return;
    }
    const slot = this.slots.get(wanted.page);
    const pageSize = this.sizes?.[wanted.page];
    if (!slot || !pageSize || wanted.tile.scale !== this.scale || slot.tiles.has(wanted.id)) {
      releaseRendered(bitmap);
      return;
    }
    const pixels = pagePixelSize(pageSize, this.scale);
    const canvas = renderedCanvas(bitmap);
    releaseRendered(bitmap);
    canvas.className = this.classNames.tile;
    const { x, y, width, height } = wanted.tile;
    const style = canvas.style;
    style.insetInlineStart = `${(100 * x) / pixels.width}%`;
    style.insetBlockStart = `${(100 * y) / pixels.height}%`;
    style.inlineSize = `${(100 * width) / pixels.width}%`;
    style.blockSize = `${(100 * height) / pixels.height}%`;
    slot.tileLayer.append(canvas);
    slot.tiles.set(wanted.id, { canvas, scale: this.scale, tile: wanted.tile });
    slot.renderMicros += bitmap.renderMicros ?? 0;
    this.checkSharp(slot);
  }

  /** Keeps a detached preview canvas for when its page comes back. */
  private cachePreview(page: number, canvas: HTMLCanvasElement): void {
    const previous = this.previews.get(page);
    if (previous && previous !== canvas) releaseCanvas(previous);
    this.previews.delete(page);
    this.previews.set(page, canvas);
    while (this.previews.size > PREVIEW_CACHE_PAGES) {
      const [oldest, stale] = this.previews.entries().next().value ?? [];
      if (oldest === undefined || !stale) break;
      this.previews.delete(oldest);
      releaseCanvas(stale);
    }
  }

  private attachPreview(slot: Slot, canvas: HTMLCanvasElement): void {
    if (slot.stalePreview) {
      releaseCanvas(slot.stalePreview);
      slot.stalePreview = null;
    }
    slot.element.prepend(canvas);
    slot.preview = canvas;
    if (!this.previewShownAt.has(slot.page)) {
      this.previewShownAt.set(slot.page, performance.now() - this.openedAt);
    }
    const detail: PagePreviewDetail = { documentId: this.documentId, page: slot.page };
    window.dispatchEvent(new CustomEvent(PAGE_PREVIEW_EVENT, { detail }));
  }

  /** Takes a page off screen: its tiles are freed, its preview kept for its return. */
  private unmountSlot(slot: Slot): void {
    if (slot.stalePreview) releaseCanvas(slot.stalePreview);
    slot.stalePreview = null;
    const preview = slot.preview;
    slot.preview = null;
    if (preview) {
      preview.remove();
      this.cachePreview(slot.page, preview);
    }
    releaseSlot(slot);
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
  if (slot.stalePreview) releaseCanvas(slot.stalePreview);
  slot.stalePreview = null;
  slot.element.remove();
}
