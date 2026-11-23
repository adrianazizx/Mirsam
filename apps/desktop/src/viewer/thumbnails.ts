import {
  type Backend,
  drawRendered,
  type PageSize,
  type RenderedPage,
  releaseRendered,
  type TileRequest,
} from "../backend";
import { pagePixelSize, tileId } from "./geometry";
import { firstRowBelow, type Row } from "./layout";
import { TileClient, type WantedTile } from "./tileClient";

/**
 * The thumbnail strip of the left panel: one small whole-page render per page,
 * virtualized like the page view so a 1,000-page document mounts only the
 * thumbnails near the visible part of the list.
 *
 * Renders go through the `thumbnails` lane of the render queue, which has its
 * own viewport generation: scrolling the strip never cancels the page view's
 * tiles, and scrolling the page view never cancels thumbnails.
 */

export interface ThumbnailOptions {
  backend: Backend;
  documentId: number;
  container: HTMLElement;
  /** Width of a thumbnail image in CSS px (the `--size-thumbnail` token). */
  width: number;
  /** Vertical space per item besides the image (label and padding), CSS px. */
  chrome: number;
  classNames: { list: string; item: string; image: string; label: string; current: string };
  /** Accessible name and visible label of a page ("Page 3", or its label "iv"). */
  label(page: number): { name: string; text: string };
  onSelect(page: number): void;
}

interface Item {
  page: number;
  element: HTMLButtonElement;
  image: HTMLDivElement;
  canvas: HTMLCanvasElement | null;
  /** The image from before a refresh, shown until the new one replaces it. */
  stale: HTMLCanvasElement | null;
  label: HTMLSpanElement;
}

/** Thumbnails mounted beyond the visible part of the list, in list heights. */
const MOUNT_MARGIN = 1;

export class ThumbnailStrip {
  private readonly options: ThumbnailOptions;
  private readonly list: HTMLDivElement;
  private client: TileClient;
  private readonly items = new Map<number, Item>();
  private sizes: PageSize[] | null = null;
  /** One row per page: its top and bottom in the list. */
  private rows: Row[] = [];
  private current = 0;
  /**
   * The thumbnail Tab lands on (one tab stop for the whole strip, arrows move
   * within it): the one last focused while it is mounted, else the current page's.
   */
  private tabStop: number | null = null;
  private dpr = window.devicePixelRatio || 1;
  private destroyed = false;

  constructor(options: ThumbnailOptions) {
    this.options = options;
    this.list = document.createElement("div");
    this.list.className = options.classNames.list;
    this.list.style.position = "relative";
    options.container.append(this.list);
    this.client = this.newClient();
    options.container.addEventListener("scroll", this.update, { passive: true });
    options.container.addEventListener("keydown", this.onKeyDown);
    options.container.addEventListener("focusin", this.onFocusIn);
    options.backend.pageSizes(options.documentId).then(
      (sizes) => {
        if (this.destroyed) return;
        this.sizes = sizes;
        this.layout();
        this.reveal(this.current);
        this.update();
      },
      (error: unknown) => console.warn("thumbnails: page sizes unavailable", error),
    );
  }

  private newClient(): TileClient {
    return new TileClient(
      this.options.backend,
      this.options.documentId,
      (wanted, bitmap) => this.onRendered(wanted, bitmap),
      (wanted, error) => console.warn("thumbnail render failed", wanted, error),
      undefined,
      "thumbnails",
    );
  }

  /**
   * Draws every thumbnail again (only annotations changed); each keeps its
   * old image until the new one replaces it.
   */
  refresh(): void {
    if (this.destroyed) return;
    this.client.close();
    this.client = this.newClient();
    for (const item of this.items.values()) {
      if (!item.canvas) continue;
      if (item.stale) releaseCanvas(item.stale);
      item.stale = item.canvas;
      item.canvas = null;
    }
    this.update();
  }

  destroy(): void {
    this.destroyed = true;
    this.client.close();
    this.options.container.removeEventListener("scroll", this.update);
    this.options.container.removeEventListener("keydown", this.onKeyDown);
    this.options.container.removeEventListener("focusin", this.onFocusIn);
    for (const item of this.items.values()) release(item);
    this.items.clear();
    this.list.remove();
  }

  /** Marks the page the reader is on and keeps it in view. */
  setCurrent(page: number): void {
    const previous = this.items.get(this.current);
    if (previous) this.markCurrent(previous, false);
    this.current = page;
    const item = this.items.get(page);
    if (item) this.markCurrent(item, true);
    // Away from the strip, Tab brings the reader back to the page they are on.
    if (!this.options.container.contains(document.activeElement)) this.tabStop = null;
    this.reveal(page);
    this.refreshTabStop();
  }

  /** Re-reads every mounted label (language change, labels loaded). */
  refreshLabels(): void {
    for (const item of this.items.values()) this.applyLabel(item);
  }

  private imageHeight(size: PageSize): number {
    return Math.round((this.options.width * size.height) / size.width);
  }

  private layout(): void {
    const sizes = this.sizes ?? [];
    let top = 0;
    this.rows = sizes.map((size, page) => {
      const height = this.imageHeight(size) + this.options.chrome;
      const row = { top, bottom: top + height, pages: [page] };
      top += height;
      return row;
    });
    this.list.style.blockSize = `${top}px`;
  }

  private reveal(page: number): void {
    const row = this.rows[page];
    const container = this.options.container;
    if (!row) return;
    const view0 = container.scrollTop;
    const view1 = view0 + container.clientHeight;
    if (row.top < view0 || row.bottom > view1) {
      container.scrollTop = row.top - (container.clientHeight - (row.bottom - row.top)) / 2;
    }
  }

  private readonly update = () => {
    const sizes = this.sizes;
    if (!sizes || this.destroyed) return;
    const container = this.options.container;
    const view0 = container.scrollTop;
    const view1 = view0 + container.clientHeight;
    const margin = container.clientHeight * MOUNT_MARGIN;
    const wanted: WantedTile[] = [];
    const keep = new Set<number>();
    for (
      let index = firstRowBelow(this.rows, view0 - margin);
      index < this.rows.length;
      index += 1
    ) {
      const row = this.rows[index];
      if (!row || row.top > view1 + margin) break;
      keep.add(index);
      const item = this.items.get(index) ?? this.mount(index, row);
      if (item.canvas) continue;
      const tile = this.tileFor(sizes[index]);
      if (!tile) continue;
      const visible = row.bottom > view0 && row.top < view1;
      wanted.push({
        id: tileId(index, tile),
        page: index,
        tile,
        priority: {
          urgency: visible ? "thumbnail" : "prefetch",
          distance: Math.abs((row.top + row.bottom) / 2 - (view0 + view1) / 2),
        },
      });
    }
    for (const [page, item] of this.items) {
      if (!keep.has(page) && item.element !== document.activeElement) {
        release(item);
        this.items.delete(page);
      }
    }
    this.client.update(wanted, () => false);
    this.refreshTabStop();
  };

  /** Gives exactly one mounted thumbnail a place in the tab order. */
  private refreshTabStop(): void {
    let target: number | null = null;
    if (this.tabStop !== null && this.items.has(this.tabStop)) target = this.tabStop;
    else if (this.items.has(this.current)) target = this.current;
    else for (const page of this.items.keys()) if (target === null || page < target) target = page;
    for (const [page, item] of this.items) {
      const tabIndex = page === target ? 0 : -1;
      // Called on every scroll: write only what changes.
      if (item.element.tabIndex !== tabIndex) item.element.tabIndex = tabIndex;
    }
  }

  private tileFor(size: PageSize | undefined): TileRequest | null {
    if (!size || size.width <= 0) return null;
    const scale = Math.fround((this.options.width * Math.min(2, this.dpr)) / size.width);
    const { width, height } = pagePixelSize(size, scale);
    if (width <= 0 || height <= 0) return null;
    return { scale, x: 0, y: 0, width, height };
  }

  private mount(page: number, row: Row): Item {
    const { classNames } = this.options;
    const element = document.createElement("button");
    element.type = "button";
    element.tabIndex = -1;
    element.className = classNames.item;
    element.dataset.page = String(page);
    element.style.position = "absolute";
    element.style.insetBlockStart = `${row.top}px`;
    element.style.blockSize = `${row.bottom - row.top}px`;
    element.addEventListener("click", () => this.options.onSelect(page));
    const image = document.createElement("div");
    image.className = classNames.image;
    const size = this.sizes?.[page];
    if (size) {
      image.style.inlineSize = `${this.options.width}px`;
      image.style.blockSize = `${this.imageHeight(size)}px`;
    }
    const label = document.createElement("span");
    label.className = classNames.label;
    element.append(image, label);
    const item: Item = { page, element, image, canvas: null, stale: null, label };
    this.applyLabel(item);
    if (page === this.current) this.markCurrent(item, true);
    this.items.set(page, item);
    this.list.append(element);
    return item;
  }

  private applyLabel(item: Item): void {
    const { name, text } = this.options.label(item.page);
    item.element.setAttribute("aria-label", name);
    item.label.textContent = text;
  }

  private markCurrent(item: Item, current: boolean): void {
    item.element.classList.toggle(this.options.classNames.current, current);
    if (current) item.element.setAttribute("aria-current", "page");
    else item.element.removeAttribute("aria-current");
  }

  private onRendered(wanted: WantedTile, bitmap: RenderedPage): void {
    const item = this.items.get(wanted.page);
    if (!item || item.canvas) {
      releaseRendered(bitmap);
      return;
    }
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d", { alpha: false });
    if (context) drawRendered(context, bitmap);
    releaseRendered(bitmap);
    canvas.setAttribute("aria-hidden", "true");
    if (item.stale) {
      item.stale.replaceWith(canvas);
      releaseCanvas(item.stale);
      item.stale = null;
    } else {
      item.image.append(canvas);
    }
    item.canvas = canvas;
  }

  private readonly onFocusIn = (event: FocusEvent) => {
    const target = event.target;
    if (!(target instanceof HTMLElement) || target.dataset.page === undefined) return;
    this.tabStop = Number(target.dataset.page);
    this.refreshTabStop();
  };

  /**
   * Up and down arrows move between thumbnails, Page Up and Page Down by a
   * screenful, Home and End to the first and last (only nearby ones are
   * mounted). The strip is a vertical list, so nothing mirrors in RTL.
   */
  private readonly onKeyDown = (event: KeyboardEvent) => {
    const focused = document.activeElement;
    if (!(focused instanceof HTMLElement) || focused.dataset.page === undefined) return;
    const from = Number(focused.dataset.page);
    const last = this.rows.length - 1;
    const row = this.rows[from];
    const screenful = row
      ? Math.max(1, Math.floor(this.options.container.clientHeight / (row.bottom - row.top)))
      : 1;
    const steps: Record<string, number> = {
      ArrowDown: from + 1,
      ArrowUp: from - 1,
      PageDown: Math.min(last, from + screenful),
      PageUp: Math.max(0, from - screenful),
      Home: 0,
      End: last,
    };
    const page = steps[event.key];
    if (page === undefined) return;
    event.preventDefault();
    const target = this.rows[page];
    if (!target) return;
    this.tabStop = page;
    this.reveal(page);
    this.update();
    (this.items.get(page) ?? this.mount(page, target)).element.focus();
  };
}

function release(item: Item): void {
  if (item.canvas) releaseCanvas(item.canvas);
  if (item.stale) releaseCanvas(item.stale);
  item.element.remove();
}

function releaseCanvas(canvas: HTMLCanvasElement): void {
  canvas.width = 0;
  canvas.height = 0;
  canvas.remove();
}
