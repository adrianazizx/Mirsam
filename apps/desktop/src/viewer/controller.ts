import {
  type AnnotationKind,
  type Backend,
  type MarkedPage,
  type PageAnnotation,
  type PageSize,
  type ParagraphBlock,
  type RenderedPage,
  releaseRendered,
  renderedCanvas,
  type SearchHit,
  type StrokePoint,
  type TextLayer,
  type TileRequest,
  type Urgency,
  type ViewRect,
} from "../backend";
import { clampZoom } from "../state/zoom";
import { annotationAt, clampToPage, erased, inkRuns } from "./annotations";
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
  /** An annotation was selected (clicked), or the selection was cleared. */
  onAnnotationSelect?(selected: SelectedAnnotation | null): void;
  /** The pen finished a stroke (Comment tool). */
  onInk?(page: number, strokes: StrokePoint[][]): void;
  /** The eraser touched these ink annotations of a page (Comment tool). */
  onErase?(page: number, indices: number[]): void;
  /** A press placed a new note: its icon's top-left corner (Comment tool). */
  onPlaceNote?(page: number, x: number, y: number): void;
  /** A press placed a new text box: its top-left corner and width (Comment tool). */
  onPlaceTextBox?(page: number, x: number, y: number, width: number): void;
  /** The Sign tool placed something (ADR 0045): the press, in displayed-page points. */
  onPlace?(page: number, x: number, y: number): void;
  /** A drag marked an area (Protect tool): the box on the displayed page. */
  onMarkArea?(page: number, rect: ViewRect): void;
  /** The Edit tool placed new page text (ADR 0021): its top-left corner and width. */
  onPlaceText?(page: number, x: number, y: number, width: number): void;
  /** The Edit tool pressed a paragraph of the page's own text (ADR 0023). */
  onEditBlock?(page: number, block: ParagraphBlock): void;
  /** The Edit tool pressed text that cannot be edited (ADR 0023). */
  onBlockRefused?(block: ParagraphBlock): void;
}

/** An annotation the reader selected by clicking it. */
export interface SelectedAnnotation {
  page: number;
  /** Its position in the page's annotations (what changing or removing it takes). */
  index: number;
  kind: AnnotationKind;
  /** A note's text. */
  contents: string | null;
  /** Its box on the displayed page. */
  rect: ViewRect;
}

/**
 * What a press on a page does: select text and annotations; place a note,
 * draw, or erase (the Comment tool); mark an area to redact (the Protect
 * tool, ADR 0042); place Fill & Sign items (the Sign tool, ADR 0045); or
 * nothing (other tools).
 */
export type ViewerMode =
  | "select"
  | "note"
  | "textBox"
  | "text"
  | "pen"
  | "eraser"
  | "redact"
  | "place"
  | "none";

/** Fill & Sign items, which a press with the Sign tool selects rather than places over. */
const FILL_ITEMS: readonly string[] = ["fillText", "mark", "signature"];

/** A note's icon, in points (the engine's `NOTE_SIZE`). */
export const NOTE_SIZE = 20;
/** A new text box's width, in points, where the page has room (`TextBox::DEFAULT_WIDTH`). */
export const TEXT_BOX_WIDTH = 240;
/** The height a new, empty text box is shown with: one line of 12-point text. */
const TEXT_BOX_LINE = 20;
/** The narrowest a new text box is made. */
const TEXT_BOX_MIN_WIDTH = 40;

const SVG_NS = "http://www.w3.org/2000/svg";

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
  /** Outline of a selected annotation. */
  markupSelected: string;
  /** Ink drawn but not yet rendered by the engine (an SVG over the page). */
  inkPreview: string;
  /** Ink the eraser is touching. */
  inkErasing: string;
  /** Where a new note's icon goes, while its text is written. */
  notePending: string;
  /** Where a new text box goes, while its text is written. */
  textBoxPending: string;
  /** Anchor element the note editor is rendered into. */
  noteEditor: string;
  /** Anchor element the selection toolbar is rendered into. */
  selectionToolbar: string;
  /** The canvas that shows the pages during a zoom gesture. */
  gesture?: string;
  /** An area marked for redaction, and the one being dragged (ADR 0042). */
  redactMark?: string;
  redactPreview?: string;
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
  /** What a press on a page does now (the active tool); "select" if absent. */
  mode?(): ViewerMode;
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
/*
 * During a zoom gesture nothing is laid out again. The pages on screen are
 * drawn, scaled about the point under the pointer, into one canvas over the
 * viewport, from the previews and tiles they already have; the page area
 * underneath is hidden. Pages the gesture brings into view are mounted (so
 * their previews are fetched) but not shown. The real layout happens once,
 * when the gesture ends.
 *
 * Scaling the page area itself with a CSS transform, laid out again every
 * 250 ms or 0.5–1.5× of scale, cost WebKit's compositor a dropped frame at
 * each relayout and more as a zoom-out revealed more of the scaled layer
 * tree: an extreme pinch-out (×1/20 in 1.2 s) dropped 3–6 frames, p95 25–31
 * ms. One canvas costs the compositor the same at any scale (ADR 0011,
 * pinch-out amendment).
 */
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
  /** The SVG drawing this page's ink preview, when there is one. */
  ink: SVGSVGElement | null;
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
  /**
   * When set, what happened and when (`performance.now()`), so a benchmark
   * can line slow frames up with the viewer's own steps.
   */
  trace: ViewerTraceEvent[] | null;
}

export interface ViewerTraceEvent {
  at: number;
  kind: "scale" | "relayout" | "mount" | "unmount" | "preview" | "tile";
  page?: number;
  ms?: number;
}

export function emptyProfile(trace = false): ViewerProfile {
  return {
    updates: 0,
    updateMs: 0,
    relayouts: 0,
    relayoutMs: 0,
    draws: 0,
    drawMs: 0,
    mounts: 0,
    longestMs: 0,
    trace: trace ? [] : null,
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
   * A zoom gesture in progress: the layout is still at `layoutZoom`, and the
   * gesture canvas shows it at `this.zoom`, scaled about `origin` (a point of
   * the page area, under the pointer when the gesture began). `at` is the
   * pointer (viewport px).
   */
  private gesture: {
    layoutZoom: number;
    origin: { x: number; y: number };
    at: { x: number; y: number };
    canvas: HTMLCanvasElement | null;
    /** The page colour (a token), read once per gesture. */
    paper: string;
  } | null = null;
  private readonly slots = new Map<number, Slot>();
  /** Detached preview canvases by page, least recently used first. */
  private readonly previews = new Map<number, HTMLCanvasElement>();
  private readonly previewShownAt = new Map<number, number>();
  private readonly failed = new Set<string>();
  private readonly resizeObserver: ResizeObserver;
  private dprQuery: MediaQueryList | null = null;
  private destroyed = false;

  private readonly mode: () => ViewerMode;
  private readonly layers = new Map<number, TextLayer>();
  private readonly layerRequests = new Set<number>();
  /** Bumped when the pages' text may have changed: answers asked for before are dropped. */
  private layerGeneration = 0;
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
  /** Annotations of recently visited pages, for clicking and erasing them. */
  private readonly annotations = new Map<number, PageAnnotation[]>();
  private readonly annotationRequests = new Map<number, Promise<void>>();
  /** Answers about annotations from before the last refresh are dropped. */
  private annotationGeneration = 0;
  private selectedAnnotation: SelectedAnnotation | null = null;
  /** Where the last plain click pressed: an annotation there is selected if no text was. */
  private pressedPoint: PagePoint | null = null;
  /** Counts presses, so a late answer about annotations applies only to its own click. */
  private presses = 0;
  /** A pen stroke being drawn, kept on the page it started on. */
  private drawing: { pointerId: number; page: number; points: StrokePoint[] } | null = null;
  /** Strokes drawn but not yet in the page's tiles: shown until the page repaints. */
  private readonly inkPreview = new Map<number, StrokePoint[][]>();
  /** An area being dragged out for redaction, kept on the page it started on. */
  private marking: { pointerId: number; page: number; from: PagePoint; to: PagePoint } | null =
    null;
  /** Areas marked for redaction, by page, shown until they are applied or removed. */
  private readonly redactMarks = new Map<number, ViewRect[]>();
  /** The ink annotations the eraser has touched during this press. */
  private erasing: { pointerId: number; page: number; indices: Set<number> } | null = null;
  /** Where the note editor sits: a new note's icon, or the selected note. */
  private noteAnchor: {
    page: number;
    rect: ViewRect;
    pending: boolean;
    kind: "note" | "textBox";
  } | null = null;
  /** Where the note editor is drawn: under the note's icon. */
  readonly noteEditorHost: HTMLDivElement;
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

    this.mode = options.mode ?? (() => "select");
    this.content = document.createElement("div");
    this.content.dataset.viewerContent = "";
    this.content.style.position = "relative";
    this.container.append(this.content);
    this.selectionToolbarHost = document.createElement("div");
    this.selectionToolbarHost.className = this.classNames.selectionToolbar;
    this.selectionToolbarHost.hidden = true;
    this.content.append(this.selectionToolbarHost);
    this.noteEditorHost = document.createElement("div");
    this.noteEditorHost.className = this.classNames.noteEditor;
    this.noteEditorHost.hidden = true;
    this.content.append(this.noteEditorHost);

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
    if (this.gesture?.canvas) releaseCanvas(this.gesture.canvas);
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
    this.annotationGeneration += 1;
    this.annotations.clear();
    this.annotationRequests.clear();
    this.setSelectedAnnotation(null);
    // Text written into the pages (ADR 0021) changes what they say: their
    // text is read again when next needed.
    this.layerGeneration += 1;
    this.layers.clear();
    this.layerRequests.clear();
    this.setSelection(null);
    const keep = this.keepAfterRefresh;
    this.keepAfterRefresh = null;
    if (keep) this.setSelectedAnnotation(keep);
    this.update();
  }

  /**
   * Selects `annotation` again once the next revision is shown: an item just
   * resized stays selected, to resize again (ADR 0045).
   */
  reselectAfterRefresh(annotation: SelectedAnnotation): void {
    this.keepAfterRefresh = annotation;
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
    // During a zoom gesture, what the gesture canvas shows.
    const view = this.shownRect();
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

  /** Shows the areas marked for redaction (ADR 0042). */
  setRedactionMarks(marks: { page: number; rects: ViewRect[] }[]): void {
    const previous = new Set(this.redactMarks.keys());
    this.redactMarks.clear();
    for (const mark of marks) {
      this.redactMarks.set(mark.page, [...(this.redactMarks.get(mark.page) ?? []), ...mark.rects]);
    }
    for (const page of new Set([...previous, ...this.redactMarks.keys()])) {
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
    this.setSelectedAnnotation(null);
  }

  /**
   * Scrolls an annotation into view, a little below the top, and selects it
   * as a click on it would (the comments panel, ADR 0041).
   */
  /** What `refresh` selects again (`reselectAfterRefresh`). */
  private keepAfterRefresh: SelectedAnnotation | null = null;

  showAnnotation(annotation: SelectedAnnotation): void {
    const height = this.sizes?.[annotation.page]?.height ?? 0;
    const above = 24;
    this.goToPage(annotation.page, height > 0 ? (annotation.rect.y - above) / height : 0);
    this.setSelection(null);
    this.setSelectedAnnotation(annotation);
  }

  /** The selected text's runs, page by page: what marking it covers (ADR 0017). */
  selectionMarks(): MarkedPage[] {
    return [...this.selectionRects]
      .filter(([, rects]) => rects.length > 0)
      .sort(([a], [b]) => a - b)
      .map(([page, rects]) => ({ page, rects: rects.map((rect) => ({ ...rect })) }));
  }

  /** The annotation selected by clicking it, if any. */
  currentAnnotation(): SelectedAnnotation | null {
    return this.selectedAnnotation;
  }

  /**
   * Shows where a new note's icon or text box goes (its top-left corner)
   * while its text is written, and anchors the editor there; null takes it
   * away.
   */
  showPendingNote(
    note: {
      page: number;
      x: number;
      y: number;
      kind: "note" | "textBox" | "pageText" | "block" | "fillText";
      width: number;
      height?: number;
    } | null,
  ): void {
    const previous = this.noteAnchor;
    if (note) {
      const height = note.height ?? (note.kind === "note" ? NOTE_SIZE : TEXT_BOX_LINE);
      this.noteAnchor = {
        page: note.page,
        rect: { x: note.x, y: note.y, width: note.width, height },
        pending: true,
        // New page text shows where it goes as a text box does: a dashed outline.
        kind: note.kind === "note" ? "note" : "textBox",
      };
    } else if (previous?.pending) {
      this.noteAnchor = null;
    }
    for (const page of new Set([previous?.page, this.noteAnchor?.page])) {
      const slot = page === undefined ? undefined : this.slots.get(page);
      if (slot) this.paintOverlay(slot);
    }
    this.placeNoteEditor();
  }

  /** Drops the pen's preview on a page (its stroke was not added after all). */
  clearInkPreview(page: number): void {
    if (!this.inkPreview.delete(page)) return;
    const slot = this.slots.get(page);
    if (slot) this.paintOverlay(slot);
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
    const generation = this.layerGeneration;
    return this.backend.pageText(this.documentId, page).then(
      (layer) => {
        if (generation !== this.layerGeneration) return null;
        this.layerRequests.delete(page);
        if (this.destroyed) return null;
        this.layers.set(page, layer);
        this.trimLayers();
        this.onLayerLoaded(page, layer);
        return layer;
      },
      (error: unknown) => {
        if (generation === this.layerGeneration) this.layerRequests.delete(page);
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

  /** The page's annotations, from the cache or asked for once; settles when they are known. */
  private loadAnnotations(page: number): Promise<void> {
    const pending = this.annotationRequests.get(page);
    if (this.annotations.has(page)) return Promise.resolve();
    if (pending) return pending;
    const generation = this.annotationGeneration;
    const request = this.backend.pageAnnotations(this.documentId, page).then(
      (list) => {
        if (this.destroyed || generation !== this.annotationGeneration) return;
        this.annotationRequests.delete(page);
        this.annotations.set(page, list);
        if (this.annotations.size > TEXT_LAYER_PAGES) {
          const oldest = this.annotations.keys().next().value;
          if (oldest !== undefined && oldest !== this.selectedAnnotation?.page) {
            this.annotations.delete(oldest);
          }
        }
      },
      (error: unknown) => {
        if (generation === this.annotationGeneration) this.annotationRequests.delete(page);
        console.warn("page annotations unavailable", page, error);
      },
    );
    this.annotationRequests.set(page, request);
    return request;
  }

  /** The annotation under a point, the one drawn last first. */
  private annotationAt(point: PagePoint): SelectedAnnotation | null {
    const hit = annotationAt(this.annotations.get(point.page) ?? [], point);
    if (!hit) return null;
    const x0 = Math.min(...hit.rects.map((r) => r.x));
    const y0 = Math.min(...hit.rects.map((r) => r.y));
    const x1 = Math.max(...hit.rects.map((r) => r.x + r.width));
    const y1 = Math.max(...hit.rects.map((r) => r.y + r.height));
    return {
      page: point.page,
      index: hit.index,
      kind: hit.kind,
      contents: hit.contents,
      rect: { x: x0, y: y0, width: x1 - x0, height: y1 - y0 },
    };
  }

  private setSelectedAnnotation(selected: SelectedAnnotation | null): void {
    const previous = this.selectedAnnotation;
    if (previous?.page === selected?.page && previous?.index === selected?.index) return;
    this.selectedAnnotation = selected;
    // A note opens its editor under the icon; a pending new note stays put.
    if (
      selected?.kind === "note" ||
      selected?.kind === "textBox" ||
      selected?.kind === "fillText"
    ) {
      this.noteAnchor = {
        page: selected.page,
        rect: selected.rect,
        pending: false,
        kind: selected.kind === "note" ? "note" : "textBox",
      };
    } else if (this.noteAnchor && !this.noteAnchor.pending) {
      this.noteAnchor = null;
    }
    for (const page of new Set([previous?.page, selected?.page])) {
      const slot = page === undefined ? undefined : this.slots.get(page);
      if (slot) this.paintOverlay(slot);
    }
    this.placeSelectionToolbar();
    this.placeNoteEditor();
    this.callbacks.onAnnotationSelect?.(selected);
  }

  /** The selected annotation's rectangles: its marked runs, else its box. */
  private selectedAnnotationRects(): ViewRect[] {
    const selected = this.selectedAnnotation;
    if (!selected) return [];
    const found = this.annotations.get(selected.page)?.find((a) => a.index === selected.index);
    return found && found.kind !== "ink" && found.kind !== "note" ? found.rects : [selected.rect];
  }

  /** Puts the note editor's host under the note's icon. */
  private placeNoteEditor(): void {
    const host = this.noteEditorHost;
    const anchor = this.noteAnchor;
    const box = anchor ? this.layout?.boxes[anchor.page] : undefined;
    const size = anchor ? this.sizes?.[anchor.page] : undefined;
    if (!anchor || !box || !size) {
      host.hidden = true;
      return;
    }
    const sx = box.width / size.width;
    const sy = box.height / size.height;
    host.hidden = false;
    // The page area is always left to right: inline-start is the left edge.
    host.style.insetInlineStart = `${box.x + anchor.rect.x * sx}px`;
    host.style.insetBlockStart = `${box.y + (anchor.rect.y + anchor.rect.height) * sy}px`;
  }

  /** A client position on a given page (not the nearest one), clamped to it. */
  private pointOnPage(page: number, clientX: number, clientY: number): PagePoint | null {
    const box = this.layout?.boxes[page];
    const size = this.sizes?.[page];
    if (!box || !size || box.width <= 0 || box.height <= 0) return null;
    const bounds = this.container.getBoundingClientRect();
    const x =
      ((clientX - bounds.left + this.container.scrollLeft - box.x) * size.width) / box.width;
    const y =
      ((clientY - bounds.top + this.container.scrollTop - box.y) * size.height) / box.height;
    return { page, ...clampToPage(x, y, size.width, size.height) };
  }

  /** A mouse has no pressure (it reports 0.5 while pressed, 0 otherwise): count it as 0.5. */
  private static pressureOf(event: PointerEvent): number {
    return event.pointerType === "mouse" || !(event.pressure > 0) ? 0.5 : event.pressure;
  }

  private startStroke(event: PointerEvent, point: PagePoint): void {
    this.setSelection(null);
    this.setSelectedAnnotation(null);
    const pressure = ViewerController.pressureOf(event);
    this.drawing = {
      pointerId: event.pointerId,
      page: point.page,
      points: [{ x: point.x, y: point.y, pressure }],
    };
    this.content.setPointerCapture?.(event.pointerId);
    this.paintInk(point.page);
  }

  private extendStroke(event: PointerEvent): void {
    const drawing = this.drawing;
    if (!drawing) return;
    // Every sample the pointer reported since the last event, for a smooth line.
    const samples = event.getCoalescedEvents?.() ?? [];
    for (const sample of samples.length > 0 ? samples : [event]) {
      const point = this.pointOnPage(drawing.page, sample.clientX, sample.clientY);
      const last = drawing.points.at(-1);
      if (!point || (last && last.x === point.x && last.y === point.y)) continue;
      drawing.points.push({
        x: point.x,
        y: point.y,
        pressure: ViewerController.pressureOf(sample),
      });
    }
    this.paintInk(drawing.page);
  }

  private finishStroke(event: PointerEvent): void {
    const drawing = this.drawing;
    if (!drawing) return;
    this.drawing = null;
    this.content.releasePointerCapture?.(event.pointerId);
    // Shown until the page's tiles, rendered with the new ink, replace it.
    this.inkPreview.set(drawing.page, [
      ...(this.inkPreview.get(drawing.page) ?? []),
      drawing.points,
    ]);
    this.paintInk(drawing.page);
    this.callbacks.onInk?.(drawing.page, [drawing.points]);
  }

  private startErasing(event: PointerEvent, point: PagePoint): void {
    this.setSelection(null);
    this.setSelectedAnnotation(null);
    this.erasing = { pointerId: event.pointerId, page: point.page, indices: new Set() };
    this.content.setPointerCapture?.(event.pointerId);
    void this.loadAnnotations(point.page).then(() => this.eraseAt(point));
  }

  private eraseAt(point: PagePoint): void {
    const erasing = this.erasing;
    if (!erasing || erasing.page !== point.page) return;
    let touched = false;
    for (const index of erased(this.annotations.get(point.page) ?? [], point)) {
      if (!erasing.indices.has(index)) {
        erasing.indices.add(index);
        touched = true;
      }
    }
    const slot = this.slots.get(point.page);
    if (touched && slot) this.paintOverlay(slot);
  }

  private finishErasing(event: PointerEvent): void {
    const erasing = this.erasing;
    if (!erasing) return;
    this.erasing = null;
    this.content.releasePointerCapture?.(event.pointerId);
    const slot = this.slots.get(erasing.page);
    if (slot) this.paintOverlay(slot);
    if (erasing.indices.size > 0) this.callbacks.onErase?.(erasing.page, [...erasing.indices]);
  }

  /** Redraws a page's ink preview (or the whole overlay, the first time). */
  private paintInk(page: number): void {
    const slot = this.slots.get(page);
    if (!slot) return;
    if (!slot.ink) {
      this.paintOverlay(slot);
      return;
    }
    this.drawInk(slot.ink, page);
  }

  private drawInk(svg: SVGSVGElement, page: number): void {
    const strokes = [...(this.inkPreview.get(page) ?? [])];
    if (this.drawing?.page === page) strokes.push(this.drawing.points);
    const paths: SVGPathElement[] = [];
    for (const stroke of strokes) {
      for (const run of inkRuns(stroke)) {
        const path = document.createElementNS(SVG_NS, "path");
        path.setAttribute("d", run.d);
        path.setAttribute("stroke-width", String(run.width));
        paths.push(path);
      }
    }
    svg.replaceChildren(...paths);
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
    const mode = this.mode();
    if (event.button !== 0 || mode === "none" || !this.layout) return;
    // The selection toolbar's and the note editor's own controls.
    if (
      event.target instanceof Node &&
      (this.selectionToolbarHost.contains(event.target) ||
        this.noteEditorHost.contains(event.target))
    ) {
      return;
    }
    // Clicks on the scrollbars land on the container, not the content.
    this.container.focus({ preventScroll: true });
    const point = this.pointAt(event.clientX, event.clientY);
    if (!point) return;
    event.preventDefault();
    if (mode === "pen") {
      this.startStroke(event, point);
      return;
    }
    if (mode === "eraser") {
      this.startErasing(event, point);
      return;
    }
    if (mode === "redact") {
      this.setSelection(null);
      this.setSelectedAnnotation(null);
      this.marking = { pointerId: event.pointerId, page: point.page, from: point, to: point };
      this.content.setPointerCapture?.(event.pointerId);
      return;
    }
    const clicks = this.countClick(event);
    // A plain click on an annotation selects it, unless the press turns into a text selection.
    this.presses += 1;
    this.pressedPoint = clicks === 1 && !event.shiftKey ? point : null;
    void this.loadAnnotations(point.page);
    this.setSelectedAnnotation(null);
    if (mode === "note" || mode === "textBox" || mode === "text" || mode === "place") {
      // A press places a note, a text box, new text, or a Fill & Sign item (or opens one) when it ends; it selects no text.
      this.setSelection(null);
      this.dragging = {
        pointerId: event.pointerId,
        clientX: event.clientX,
        clientY: event.clientY,
      };
      this.content.setPointerCapture?.(event.pointerId);
      return;
    }
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
    if (this.drawing && event.pointerId === this.drawing.pointerId) {
      this.extendStroke(event);
      return;
    }
    if (this.erasing && event.pointerId === this.erasing.pointerId) {
      const point = this.pointOnPage(this.erasing.page, event.clientX, event.clientY);
      if (point) this.eraseAt(point);
      return;
    }
    if (this.marking && event.pointerId === this.marking.pointerId) {
      const point = this.pointOnPage(this.marking.page, event.clientX, event.clientY);
      if (point) {
        this.marking.to = point;
        const slot = this.slots.get(this.marking.page);
        if (slot) this.paintOverlay(slot);
      }
      return;
    }
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
    if (this.drawing && event.pointerId === this.drawing.pointerId) {
      if (event.type === "pointercancel") {
        const { page } = this.drawing;
        this.drawing = null;
        this.paintInk(page);
      } else {
        this.finishStroke(event);
      }
      return;
    }
    if (this.erasing && event.pointerId === this.erasing.pointerId) {
      this.finishErasing(event);
      return;
    }
    if (this.marking && event.pointerId === this.marking.pointerId) {
      const { page, from, to } = this.marking;
      this.marking = null;
      this.content.releasePointerCapture?.(event.pointerId);
      const slot = this.slots.get(page);
      if (slot) this.paintOverlay(slot);
      if (event.type !== "pointercancel") {
        this.callbacks.onMarkArea?.(page, {
          x: Math.min(from.x, to.x),
          y: Math.min(from.y, to.y),
          width: Math.abs(to.x - from.x),
          height: Math.abs(to.y - from.y),
        });
      }
      return;
    }
    if (!this.dragging || event.pointerId !== this.dragging.pointerId) return;
    this.dragging = null;
    this.pendingAnchor = null;
    this.stopAutoScroll();
    this.content.releasePointerCapture?.(event.pointerId);
    if (this.selection && isCollapsed(this.selection, this.layerLookup)) this.setSelection(null);
    const pressed = this.pressedPoint;
    this.pressedPoint = null;
    if (!pressed || this.selection !== null) return;
    // The page's annotations may still be on their way (a click right after an edit).
    const press = this.presses;
    const mode = this.mode();
    void this.loadAnnotations(pressed.page).then(() => {
      if (this.destroyed || press !== this.presses || this.selection !== null) return;
      const hit = this.annotationAt(pressed);
      const size = this.sizes?.[pressed.page];
      if (mode === "place") {
        // On an item placed before, the press selects it; elsewhere it places another.
        if (hit && FILL_ITEMS.includes(hit.kind)) this.setSelectedAnnotation(hit);
        else this.callbacks.onPlace?.(pressed.page, pressed.x, pressed.y);
      } else if (mode !== "note" && mode !== "textBox" && mode !== "text") {
        if (hit) this.setSelectedAnnotation(hit);
      } else if (hit?.kind === mode) {
        this.setSelectedAnnotation(hit);
      } else if (mode === "text") {
        // A press on the page's own text edits that paragraph (ADR 0023); elsewhere it places new text.
        void this.backend.pageTextBlocks(this.documentId, pressed.page).then(
          (blocks) => {
            if (this.destroyed || press !== this.presses) return;
            const block = blocks.find(
              (b) =>
                pressed.x >= b.x - 2 &&
                pressed.x <= b.x + b.width + 2 &&
                pressed.y >= b.y - 2 &&
                pressed.y <= b.y + b.height + 2,
            );
            if (block?.editable) this.callbacks.onEditBlock?.(pressed.page, block);
            else if (block) this.callbacks.onBlockRefused?.(block);
            else this.placeNewText(pressed.page, pressed.x, pressed.y);
          },
          () => this.placeNewText(pressed.page, pressed.x, pressed.y),
        );
      } else if (mode === "textBox") {
        // Its top-left corner at the press, as wide as the page allows.
        if (!size) return;
        const x = Math.max(0, Math.min(pressed.x, size.width - TEXT_BOX_MIN_WIDTH - 4));
        const y = Math.max(0, Math.min(pressed.y, size.height - TEXT_BOX_LINE));
        const width = Math.min(TEXT_BOX_WIDTH, size.width - x - 4);
        this.callbacks.onPlaceTextBox?.(pressed.page, x, y, width);
      } else {
        // The icon centred on the press, kept on the page.
        if (!size) return;
        const half = NOTE_SIZE / 2;
        const { x, y } = clampToPage(
          pressed.x - half,
          pressed.y - half,
          size.width - NOTE_SIZE,
          size.height - NOTE_SIZE,
        );
        this.callbacks.onPlaceNote?.(pressed.page, x, y);
      }
    });
  };

  /** New page text: its top-left corner at the press, as wide as the page allows. */
  private placeNewText(page: number, pressedX: number, pressedY: number): void {
    const size = this.sizes?.[page];
    if (!size) return;
    const x = Math.max(0, Math.min(pressedX, size.width - TEXT_BOX_MIN_WIDTH - 4));
    const y = Math.max(0, Math.min(pressedY, size.height - TEXT_BOX_LINE));
    const width = Math.min(TEXT_BOX_WIDTH, size.width - x - 4);
    this.callbacks.onPlaceText?.(page, x, y, width);
  }

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

  /**
   * The cursor: the text cursor over text when selecting, or the Comment
   * tool's own; a page's text and annotations arrive on first hover.
   */
  private updateCursor(event: PointerEvent): void {
    const mode = this.mode();
    this.content.dataset.mode = mode;
    if (mode !== "select") {
      delete this.content.dataset.overText;
      const point = mode === "none" ? null : this.pointAt(event.clientX, event.clientY);
      if (point) void this.loadAnnotations(point.page);
      return;
    }
    const point = this.pointAt(event.clientX, event.clientY);
    const layer = point ? this.layers.get(point.page) : undefined;
    if (point && !layer) void this.loadLayer(point.page);
    if (point) void this.loadAnnotations(point.page);
    const over = point !== null && layer !== undefined && isOverText(layer, point.x, point.y);
    if (over) this.content.dataset.overText = "";
    else delete this.content.dataset.overText;
  }

  private setSelection(selection: Selection | null): void {
    // Selecting text and selecting an annotation exclude each other.
    if (selection) this.setSelectedAnnotation(null);
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
    // A note or a text box has its editor instead.
    const selected = this.selectedAnnotation;
    if (
      !rects?.length &&
      selected &&
      selected.kind !== "note" &&
      selected.kind !== "textBox" &&
      selected.kind !== "fillText"
    ) {
      first = selected.page;
      rects = this.selectedAnnotationRects();
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
    if (this.selectedAnnotation?.page === slot.page) {
      for (const rect of this.selectedAnnotationRects()) {
        const element = document.createElement("div");
        element.className = this.classNames.markupSelected;
        place(element, rect);
        slot.overlay.append(element);
      }
    }
    if (this.noteAnchor?.pending && this.noteAnchor.page === slot.page) {
      const element = document.createElement("div");
      element.className =
        this.noteAnchor.kind === "textBox"
          ? this.classNames.textBoxPending
          : this.classNames.notePending;
      place(element, this.noteAnchor.rect);
      slot.overlay.append(element);
    }
    for (const rect of this.redactMarks.get(slot.page) ?? []) {
      const element = document.createElement("div");
      element.className = this.classNames.redactMark ?? "";
      place(element, rect);
      slot.overlay.append(element);
    }
    if (this.marking?.page === slot.page) {
      const { from, to } = this.marking;
      const element = document.createElement("div");
      element.className = this.classNames.redactPreview ?? "";
      place(element, {
        x: Math.min(from.x, to.x),
        y: Math.min(from.y, to.y),
        width: Math.abs(to.x - from.x),
        height: Math.abs(to.y - from.y),
      });
      slot.overlay.append(element);
    }
    // Ink the eraser touches: its strokes, marked.
    const erasing = this.erasing?.page === slot.page ? this.erasing.indices : null;
    const preview = this.inkPreview.has(slot.page) || this.drawing?.page === slot.page;
    slot.ink = null;
    if (preview || (erasing && erasing.size > 0)) {
      const svg = document.createElementNS(SVG_NS, "svg");
      svg.setAttribute("viewBox", `0 0 ${size.width} ${size.height}`);
      svg.setAttribute("preserveAspectRatio", "none");
      svg.setAttribute("aria-hidden", "true");
      svg.classList.add(this.classNames.inkPreview);
      slot.overlay.append(svg);
      slot.ink = svg;
      this.drawInk(svg, slot.page);
      for (const annotation of this.annotations.get(slot.page) ?? []) {
        if (!erasing?.has(annotation.index)) continue;
        for (const stroke of annotation.strokes) {
          const path = document.createElementNS(SVG_NS, "path");
          path.setAttribute("d", `M${stroke.map(([x, y]) => `${x} ${y}`).join("L")}`);
          path.classList.add(this.classNames.inkErasing);
          svg.append(path);
        }
      }
    }
  }

  // ------------------------------------------------------------ events

  private readonly onScroll = () => {
    // The gesture canvas covers the viewport where the gesture began.
    this.settleGesture();
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

  /** One step of a zoom gesture: the gesture canvas shows the layout at `next`. */
  private zoomBy(next: number, at: { x: number; y: number }): void {
    if (!this.gesture) {
      const origin = { x: this.container.scrollLeft + at.x, y: this.container.scrollTop + at.y };
      this.gesture = { layoutZoom: this.zoom, origin, at, canvas: null, paper: "" };
    }
    const gesture = this.gesture;
    this.zoom = next;
    gesture.at = at;
    this.profile?.trace?.push({
      at: performance.now(),
      kind: "scale",
      ms: next / gesture.layoutZoom,
    });
    // Mount what the gesture brings into view, so its previews are on the way.
    this.update();
    this.drawGesture(gesture);
  }

  /**
   * What the page area shows in layout coordinates: the viewport, or during
   * a zoom gesture the part of the layout the gesture canvas shows.
   */
  private shownRect(): Rect {
    const view = this.viewRect();
    const gesture = this.gesture;
    if (!gesture) return view;
    const { origin } = gesture;
    const ratio = this.zoom / gesture.layoutZoom;
    return {
      x0: origin.x + (view.x0 - origin.x) / ratio,
      x1: origin.x + (view.x1 - origin.x) / ratio,
      y0: origin.y + (view.y0 - origin.y) / ratio,
      y1: origin.y + (view.y1 - origin.y) / ratio,
    };
  }

  /**
   * Draws the mounted pages the gesture shows, scaled, into the gesture
   * canvas: paper, the preview, and whatever tiles the page has (at any
   * scale). Search hits, the selection and shadows return when the gesture
   * ends.
   */
  private drawGesture(gesture: NonNullable<typeof this.gesture>): void {
    const sizes = this.sizes;
    if (!sizes) return;
    const width = this.container.clientWidth;
    const height = this.container.clientHeight;
    const dpr = this.devicePixelRatio;
    const scrollX = this.container.scrollLeft;
    const scrollY = this.container.scrollTop;
    let canvas = gesture.canvas;
    if (!canvas) {
      canvas = document.createElement("canvas");
      canvas.className = this.classNames.gesture ?? "";
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      const style = canvas.style;
      // Over the viewport: the page area does not scroll during a gesture.
      style.insetInlineStart = `${scrollX}px`;
      style.insetBlockStart = `${scrollY}px`;
      style.inlineSize = `${width}px`;
      style.blockSize = `${height}px`;
      this.container.append(canvas);
      this.content.dataset.zooming = "";
      gesture.canvas = canvas;
      gesture.paper = getComputedStyle(canvas).getPropertyValue("--color-page-paper");
    }
    const context = canvas.getContext("2d");
    if (!context) return;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, canvas.width, canvas.height);
    const { origin } = gesture;
    const ratio = this.zoom / gesture.layoutZoom;
    // Layout px → canvas px: scaled by `ratio` about `origin`, less the scroll.
    context.setTransform(
      ratio * dpr,
      0,
      0,
      ratio * dpr,
      (origin.x * (1 - ratio) - scrollX) * dpr,
      (origin.y * (1 - ratio) - scrollY) * dpr,
    );
    const shown = this.shownRect();
    context.fillStyle = gesture.paper;
    for (const slot of this.slots.values()) {
      const box = slot.box;
      const size = sizes[slot.page];
      if (!box || !size || !intersect(shown, boxRect(box))) continue;
      context.fillRect(box.x, box.y, box.width, box.height);
      const preview = slot.preview ?? slot.stalePreview;
      if (preview && preview.width > 0) {
        context.drawImage(preview, box.x, box.y, box.width, box.height);
      }
      for (const { canvas: tile, scale, tile: rect } of slot.tiles.values()) {
        if (tile.width === 0) continue;
        const pixels = pagePixelSize(size, scale);
        context.drawImage(
          tile,
          box.x + (rect.x / pixels.width) * box.width,
          box.y + (rect.y / pixels.height) * box.height,
          (rect.width / pixels.width) * box.width,
          (rect.height / pixels.height) * box.height,
        );
      }
    }
  }

  /** Ends a zoom gesture: the real layout at the gesture's zoom replaces the gesture canvas. */
  private settleGesture(): void {
    const gesture = this.gesture;
    if (!gesture) return;
    // The point shown under the pointer stays there.
    const shown = this.shownRect();
    const perPixel = (shown.x1 - shown.x0) / Math.max(1, this.container.clientWidth);
    const point = { x: shown.x0 + gesture.at.x * perPixel, y: shown.y0 + gesture.at.y * perPixel };
    const layoutAnchor = this.layout ? anchorAt(this.layout, point.x, point.y) : null;
    this.gesture = null;
    this.relayout(layoutAnchor ? { anchor: layoutAnchor, at: gesture.at } : null);
    if (gesture.canvas) releaseCanvas(gesture.canvas);
    delete this.content.dataset.zooming;
    this.update();
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
      this.zooming = false;
      this.settleGesture();
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
    profile.trace?.push({ at: started, kind: "relayout", ms });
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
    this.placeNoteEditor();
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
    const view = this.shownRect();
    const height = view.y1 - view.y0;
    const centre = { x: (view.x0 + view.x1) / 2, y: (view.y0 + view.y1) / 2 };

    // 1. Mount the pages near the viewport, unmount the rest.
    const mountBand = { y0: view.y0 - height * MOUNT_MARGIN, y1: view.y1 + height * MOUNT_MARGIN };
    const mounted = new Set<number>();
    for (const rowIndex of rowsInBand(layout.rows, mountBand.y0, mountBand.y1)) {
      for (const page of layout.rows[rowIndex]?.pages ?? []) mounted.add(page);
    }
    // During a zoom gesture pages only come: the gesture canvas draws from them.
    for (const [page, slot] of this.slots) {
      if (!mounted.has(page) && !this.gesture) {
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
    if (this.profile) {
      this.profile.mounts += 1;
      this.profile.trace?.push({ at: performance.now(), kind: "mount", page });
    }
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
      ink: null,
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
      this.selectedAnnotation?.page === page ||
      this.noteAnchor?.page === page ||
      this.inkPreview.has(page)
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
    profile.trace?.push({ at: started, kind: wanted.kind, page: wanted.page, ms });
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
    this.profile?.trace?.push({ at: performance.now(), kind: "unmount", page: slot.page });
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
    // The engine's tiles now show the ink the preview stood in for.
    if (this.inkPreview.delete(slot.page)) this.paintOverlay(slot);
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
