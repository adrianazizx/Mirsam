import type { RenderedPage } from "./types";

/** Bytes of little-endian header before the pixels: width, height, render µs, reserved. */
export const BITMAP_HEADER_LEN = 16;

/**
 * Decodes a `render_page` response without copying the pixels: the returned
 * `rgba` is a view over the response buffer.
 */
export function decodeBitmap(buffer: ArrayBuffer): RenderedPage {
  if (buffer.byteLength < BITMAP_HEADER_LEN) {
    throw new Error("bitmap response is shorter than its header");
  }
  const header = new DataView(buffer, 0, BITMAP_HEADER_LEN);
  const width = header.getUint32(0, true);
  const height = header.getUint32(4, true);
  const renderMicros = header.getUint32(8, true);
  const pixelBytes = width * height * 4;
  if (width === 0 || height === 0 || buffer.byteLength !== BITMAP_HEADER_LEN + pixelBytes) {
    throw new Error("bitmap response does not match its declared size");
  }
  return {
    width,
    height,
    renderMicros,
    rgba: new Uint8ClampedArray(buffer, BITMAP_HEADER_LEN, pixelBytes),
  };
}
