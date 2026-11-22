import { describe, expect, it, vi } from "vitest";
import type { Backend, KeptRequest, Priority, RenderedPage, TileRequest } from "../backend";
import { tileId } from "./geometry";
import { TileClient, type WantedTile } from "./tileClient";

interface Call {
  generation: number;
  page: number;
  tile: TileRequest;
  resolve(page: RenderedPage): void;
  reject(error: unknown): void;
}

/** A backend whose renders settle only when the test says so. */
function fakeBackend() {
  const calls: Call[] = [];
  const updates: { generation: number; keep: KeptRequest[] }[] = [];
  const backend = {
    renderTile: (_doc: number, generation: number, page: number, tile: TileRequest) =>
      new Promise<RenderedPage>((resolve, reject) => {
        calls.push({ generation, page, tile, resolve, reject });
      }),
    updateViewport: async (_doc: number, generation: number, keep: KeptRequest[]) => {
      updates.push({ generation, keep });
    },
  } as unknown as Backend;
  return { backend, calls, updates };
}

const bitmap: RenderedPage = {
  width: 1,
  height: 1,
  renderMicros: 10,
  fromCache: false,
  rgba: new Uint8ClampedArray(4),
};

function want(page: number, x: number, urgency: Priority["urgency"] = "visible"): WantedTile {
  const tile = { scale: 1, x, y: 0, width: 1, height: 1 };
  return { id: tileId(page, tile), page, tile, priority: { urgency, distance: x } };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const CANCELLED = { code: "cancelled", detail: "" };

describe("tile client", () => {
  it("requests each wanted tile once and delivers what arrives", async () => {
    const { backend, calls, updates } = fakeBackend();
    const onTile = vi.fn();
    const client = new TileClient(backend, 1, onTile, vi.fn(), 0);
    client.update([want(0, 0), want(0, 1)], () => false);
    client.update([want(0, 0), want(0, 1)], () => false);
    expect(calls).toHaveLength(2);
    expect(updates).toHaveLength(0);
    calls[0]?.resolve(bitmap);
    await flush();
    expect(onTile).toHaveBeenCalledTimes(1);
    expect(client.pending).toBe(1);
  });

  it("skips tiles already on screen", () => {
    const { backend, calls } = fakeBackend();
    const client = new TileClient(backend, 1, vi.fn(), vi.fn(), 0);
    const drawn = want(0, 0);
    client.update([drawn, want(0, 1)], (id) => id === drawn.id);
    expect(calls.map((call) => call.tile.x)).toEqual([1]);
  });

  it("moves to a new generation keeping only what is still wanted", () => {
    const { backend, calls, updates } = fakeBackend();
    const client = new TileClient(backend, 1, vi.fn(), vi.fn(), 0);
    client.update([want(0, 0), want(0, 1)], () => false);
    // Tile 1 scrolled away; tile 2 came into view; tile 0 is now only prefetch.
    client.update([want(0, 0, "prefetch"), want(0, 2)], () => false);
    expect(updates).toEqual([
      {
        generation: 1,
        keep: [{ page: 0, tile: want(0, 0).tile, priority: { urgency: "prefetch", distance: 0 } }],
      },
    ]);
    expect(calls.at(-1)).toMatchObject({ generation: 1, tile: { x: 2 } });
    // Distance alone changing does not cost a viewport update.
    client.update(
      [{ ...want(0, 0, "prefetch"), priority: { urgency: "prefetch", distance: 9 } }, want(0, 2)],
      () => false,
    );
    expect(updates).toHaveLength(1);
  });

  it("re-requests a still-wanted tile that lost a race with a viewport update", async () => {
    const { backend, calls } = fakeBackend();
    const client = new TileClient(backend, 1, vi.fn(), vi.fn(), 0);
    client.update([want(0, 0)], () => false);
    client.update([want(0, 1)], () => false); // generation 1: tile 0 unwanted
    client.update([want(0, 0), want(0, 1)], () => false); // wanted again, still in flight
    calls[0]?.reject(CANCELLED); // the engine dropped the generation-0 request
    await flush();
    expect(calls).toHaveLength(3);
    expect(calls[2]).toMatchObject({ generation: 1, tile: { x: 0 } });
  });

  it("does not spin when the engine cancels a request of the current generation", async () => {
    const { backend, calls } = fakeBackend();
    const client = new TileClient(backend, 1, vi.fn(), vi.fn(), 0);
    client.update([want(0, 0)], () => false);
    calls[0]?.reject(CANCELLED);
    await flush();
    expect(calls).toHaveLength(1);
    expect(client.stats.cancelled).toBe(1);
    // The next update asks again.
    client.update([want(0, 0)], () => false);
    expect(calls).toHaveLength(2);
  });

  it("reports failures other than cancellation", async () => {
    const { backend, calls } = fakeBackend();
    const onFailure = vi.fn();
    const client = new TileClient(backend, 1, vi.fn(), onFailure, 0);
    client.update([want(0, 0)], () => false);
    calls[0]?.reject({ code: "invalid_render_request", detail: "" });
    await flush();
    expect(onFailure).toHaveBeenCalledTimes(1);
    expect(client.stats.failed).toBe(1);
  });

  it("starts above the generations of any earlier client of the same document", () => {
    const { backend, calls } = fakeBackend();
    const before = Date.now() * 1000;
    new TileClient(backend, 1, vi.fn(), vi.fn()).update([want(0, 0)], () => false);
    expect(calls[0]?.generation).toBeGreaterThanOrEqual(before);
    expect(Number.isSafeInteger(calls[0]?.generation)).toBe(true);
  });

  it("cancels everything on close and ignores late arrivals", async () => {
    const { backend, calls, updates } = fakeBackend();
    const onTile = vi.fn();
    const client = new TileClient(backend, 1, onTile, vi.fn(), 0);
    client.update([want(0, 0)], () => false);
    client.close();
    expect(updates).toEqual([{ generation: 1, keep: [] }]);
    calls[0]?.resolve(bitmap);
    await flush();
    expect(onTile).not.toHaveBeenCalled();
    client.update([want(0, 1)], () => false);
    expect(calls).toHaveLength(1);
  });
});
