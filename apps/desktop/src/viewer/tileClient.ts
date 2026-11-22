import {
  type Backend,
  isCancelled,
  type KeptRequest,
  type Lane,
  type Priority,
  type RenderedPage,
  type TileRequest,
} from "../backend";

/** One render the viewer wants, identified by `id` (see `geometry.tileId`). */
export interface WantedTile {
  id: string;
  page: number;
  tile: TileRequest;
  priority: Priority;
}

export interface TileClientStats {
  requested: number;
  received: number;
  fromCache: number;
  cancelled: number;
  failed: number;
  viewportUpdates: number;
}

interface InFlight {
  wanted: WantedTile;
  token: number;
  /**
   * Left out of a viewport update: the engine has cancelled it unless it was
   * already rendering. Either way its answer is on its way; it is not
   * reconsidered (or re-sent) until that answer arrives.
   */
  released: boolean;
}

/**
 * Keeps the engine's render queue in step with what the viewer wants.
 *
 * Each `update` names everything currently wanted. New ids are requested;
 * ids already in flight are left alone. When something in flight is no longer
 * wanted (or changed urgency), the client advances its viewport generation and
 * tells the engine which queued requests to keep; the engine cancels the rest.
 *
 * IPC calls can be handled out of order, so a request can be cancelled by a
 * viewport update it was meant to survive. A cancellation of an id that is
 * still wanted is therefore simply re-requested at the current generation.
 */
/**
 * First generation of a new client. The engine keeps each document's newest
 * generation for as long as the document is open, across viewer instances
 * (switching tabs) and webview reloads, and cancels anything older. Counting
 * from the clock (microsecond-scaled milliseconds, still below 2^53) keeps a
 * new client's generations above every earlier client's.
 */
export function firstGeneration(): number {
  return Date.now() * 1000;
}

export class TileClient {
  private generation: number;
  private nextToken = 0;
  private readonly inFlight = new Map<string, InFlight>();
  private wanted = new Map<string, WantedTile>();
  private closed = false;
  readonly stats: TileClientStats = {
    requested: 0,
    received: 0,
    fromCache: 0,
    cancelled: 0,
    failed: 0,
    viewportUpdates: 0,
  };

  constructor(
    private readonly backend: Backend,
    private readonly documentId: number,
    /** A requested tile arrived (possibly after it stopped being wanted). */
    private readonly onTile: (wanted: WantedTile, bitmap: RenderedPage) => void,
    /** A render failed for a reason other than cancellation. */
    private readonly onFailure: (wanted: WantedTile, error: unknown) => void,
    generation = firstGeneration(),
    /** Render-queue lane: the page view and the thumbnail strip each have their own. */
    private readonly lane: Lane = "main",
  ) {
    this.generation = generation;
  }

  /** `skip(id)` is true for tiles the viewer already has on screen. */
  update(list: WantedTile[], skip: (id: string) => boolean): void {
    if (this.closed) return;
    this.wanted = new Map(list.map((wanted) => [wanted.id, wanted]));

    let stale = false;
    for (const [id, flight] of this.inFlight) {
      if (flight.released) continue;
      const now = this.wanted.get(id);
      if (!now || now.priority.urgency !== flight.wanted.priority.urgency) {
        stale = true;
        break;
      }
    }
    if (stale) {
      this.generation += 1;
      const keep: KeptRequest[] = [];
      for (const [id, flight] of this.inFlight) {
        if (flight.released) continue;
        const now = this.wanted.get(id);
        if (!now) {
          flight.released = true;
          continue;
        }
        flight.wanted = now;
        keep.push({ page: now.page, tile: now.tile, priority: now.priority });
      }
      this.stats.viewportUpdates += 1;
      this.backend
        .updateViewport(this.documentId, this.generation, keep, this.lane)
        .catch((error: unknown) => console.warn("viewport update failed", error));
    }

    for (const wanted of list) {
      if (!this.inFlight.has(wanted.id) && !skip(wanted.id)) this.request(wanted);
    }
  }

  isInFlight(id: string): boolean {
    return this.inFlight.has(id);
  }

  get pending(): number {
    return this.inFlight.size;
  }

  get currentGeneration(): number {
    return this.generation;
  }

  /** Cancels everything queued and ignores whatever still arrives. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.wanted.clear();
    if (this.inFlight.size > 0) {
      this.generation += 1;
      this.backend.updateViewport(this.documentId, this.generation, [], this.lane).catch(() => {
        // The document may already be closed; nothing left to cancel then.
      });
    }
    this.inFlight.clear();
  }

  private request(wanted: WantedTile): void {
    const token = ++this.nextToken;
    const generation = this.generation;
    this.inFlight.set(wanted.id, { wanted, token, released: false });
    this.stats.requested += 1;
    const settle = () => {
      if (this.inFlight.get(wanted.id)?.token === token) this.inFlight.delete(wanted.id);
    };
    this.backend
      .renderTile(this.documentId, generation, wanted.page, wanted.tile, wanted.priority, this.lane)
      .then(
        (bitmap) => {
          settle();
          if (this.closed) return;
          this.stats.received += 1;
          if (bitmap.fromCache) this.stats.fromCache += 1;
          this.onTile(wanted, bitmap);
        },
        (error: unknown) => {
          settle();
          if (this.closed) return;
          if (isCancelled(error)) {
            this.stats.cancelled += 1;
            // Only a request from an older generation can have lost a race with
            // a viewport update; anything else (a full queue, shutdown) waits
            // for the next `update` rather than retrying in a loop.
            const still = this.wanted.get(wanted.id);
            if (still && generation < this.generation && !this.inFlight.has(wanted.id)) {
              this.request(still);
            }
            return;
          }
          this.stats.failed += 1;
          this.onFailure(wanted, error);
        },
      );
  }
}
