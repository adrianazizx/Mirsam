import {
  type Backend,
  drawRendered,
  type PageSize,
  type RenderedPage,
  releaseRendered,
} from "../backend";
import { pagePixelSize, tileId } from "./geometry";
import { TileClient, type WantedTile } from "./tileClient";

/**
 * Whole-page previews for React views that mount pages themselves (the page
 * organizer's grid). An item asks for its page's image while it is mounted;
 * renders go through the `thumbnails` lane, so they never cancel the page
 * view's tiles. Recent images are kept, so scrolling back shows them at once.
 */
export class PageImages {
  private client: TileClient;
  private readonly width: number;
  private readonly dpr = window.devicePixelRatio || 1;
  private sizes: PageSize[] = [];
  /** Drawn images, least recently used first. */
  private readonly images = new Map<number, HTMLCanvasElement>();
  /** Images from before a refresh, handed out until their new ones arrive. */
  private readonly stale = new Map<number, HTMLCanvasElement>();
  private readonly listeners = new Map<number, Set<(image: HTMLCanvasElement) => void>>();
  private scheduled = false;
  private destroyed = false;

  /** `width`: the CSS width an image is shown at. */
  constructor(
    private readonly backend: Backend,
    private readonly documentId: number,
    width: number,
  ) {
    this.width = width;
    this.client = this.newClient();
  }

  private newClient(): TileClient {
    return new TileClient(
      this.backend,
      this.documentId,
      (wanted, rendered) => this.onRendered(wanted, rendered),
      (wanted, error) => console.warn("page image render failed", wanted, error),
      undefined,
      "thumbnails",
    );
  }

  /**
   * Draws every image again (only annotations changed). Items keep showing
   * the old image until their listener receives the new one.
   */
  refresh(): void {
    if (this.destroyed) return;
    this.client.close();
    this.client = this.newClient();
    for (const [page, image] of this.images) {
      const previous = this.stale.get(page);
      if (previous) release(previous);
      this.stale.set(page, image);
    }
    this.images.clear();
    this.schedule();
  }

  setSizes(sizes: PageSize[]): void {
    this.sizes = sizes;
    this.schedule();
  }

  /** Calls `listener` with the page's image, now if it is ready. Returns the unsubscribe. */
  subscribe(page: number, listener: (image: HTMLCanvasElement) => void): () => void {
    const ready = this.images.get(page);
    const stale = ready ? undefined : this.stale.get(page);
    if (stale) listener(stale);
    if (ready) {
      // Most recently used goes last.
      this.images.delete(page);
      this.images.set(page, ready);
      listener(ready);
    }
    const set = this.listeners.get(page) ?? new Set();
    set.add(listener);
    this.listeners.set(page, set);
    if (!ready) this.schedule();
    return () => {
      set.delete(listener);
      if (set.size === 0) this.listeners.delete(page);
      this.schedule();
    };
  }

  destroy(): void {
    this.destroyed = true;
    this.client.close();
    for (const image of [...this.images.values(), ...this.stale.values()]) release(image);
    this.images.clear();
    this.stale.clear();
    this.listeners.clear();
  }

  /** Asks for the images mounted items are waiting for, once per task. */
  private schedule(): void {
    if (this.scheduled || this.destroyed) return;
    this.scheduled = true;
    queueMicrotask(() => {
      this.scheduled = false;
      if (this.destroyed) return;
      const wanted: WantedTile[] = [];
      for (const page of this.listeners.keys()) {
        if (this.images.has(page)) continue;
        const tile = this.tileFor(this.sizes[page]);
        if (!tile) continue;
        wanted.push({
          id: tileId(page, tile),
          page,
          tile,
          priority: { urgency: "thumbnail", distance: page },
        });
      }
      this.client.update(wanted, () => false);
    });
  }

  private tileFor(size: PageSize | undefined) {
    if (!size || size.width <= 0) return null;
    const scale = Math.fround((this.width * Math.min(2, this.dpr)) / size.width);
    const { width, height } = pagePixelSize(size, scale);
    if (width <= 0 || height <= 0) return null;
    return { scale, x: 0, y: 0, width, height };
  }

  private onRendered(wanted: WantedTile, rendered: RenderedPage): void {
    if (this.destroyed || this.images.has(wanted.page)) {
      releaseRendered(rendered);
      return;
    }
    const image = document.createElement("canvas");
    image.width = rendered.width;
    image.height = rendered.height;
    const context = image.getContext("2d", { alpha: false });
    if (context) drawRendered(context, rendered);
    releaseRendered(rendered);
    image.setAttribute("aria-hidden", "true");
    this.images.set(wanted.page, image);
    this.evict();
    for (const listener of this.listeners.get(wanted.page) ?? []) listener(image);
    // Listeners have swapped it in: the image it replaced can go.
    const stale = this.stale.get(wanted.page);
    if (stale) {
      this.stale.delete(wanted.page);
      release(stale);
    }
  }

  /** Keeps memory bounded: drops the least recently used images no item shows. */
  private evict(): void {
    const keep = 240;
    for (const [page, image] of this.images) {
      if (this.images.size <= keep) break;
      if (this.listeners.has(page)) continue;
      release(image);
      this.images.delete(page);
    }
  }
}

function release(image: HTMLCanvasElement): void {
  image.width = 0;
  image.height = 0;
  image.remove();
}
