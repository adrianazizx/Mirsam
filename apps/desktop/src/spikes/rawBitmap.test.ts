import { describe, expect, it } from "vitest";
import { BITMAP_HEADER_LEN, decodeBitmap, FLAG_FROM_CACHE } from "./rawBitmap";

function encode(
  width: number,
  height: number,
  micros: number,
  pixels: number[],
  flags = 0,
): ArrayBuffer {
  const buffer = new ArrayBuffer(BITMAP_HEADER_LEN + pixels.length);
  const view = new DataView(buffer);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  view.setUint32(8, micros, true);
  view.setUint32(12, flags, true);
  new Uint8Array(buffer, BITMAP_HEADER_LEN).set(pixels);
  return buffer;
}

describe("decodeBitmap", () => {
  it("reads the header and exposes the pixels without copying", () => {
    const buffer = encode(2, 1, 1500, [255, 0, 0, 255, 0, 0, 255, 255]);
    const page = decodeBitmap(buffer);
    expect(page).toMatchObject({ width: 2, height: 1, renderMicros: 1500, fromCache: false });
    expect([...page.rgba]).toEqual([255, 0, 0, 255, 0, 0, 255, 255]);
    expect(page.rgba.buffer).toBe(buffer);
  });

  it("reads the cache flag from the last header word", () => {
    expect(decodeBitmap(encode(1, 1, 0, [0, 0, 0, 0], FLAG_FROM_CACHE)).fromCache).toBe(true);
  });

  it("rejects a payload whose length disagrees with its header", () => {
    expect(() => decodeBitmap(encode(2, 2, 0, [0, 0, 0, 0]))).toThrow(/declared size/);
    expect(() => decodeBitmap(new ArrayBuffer(4))).toThrow(/header/);
    expect(() => decodeBitmap(encode(0, 0, 0, []))).toThrow(/declared size/);
  });
});
