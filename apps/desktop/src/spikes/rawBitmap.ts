/**
 * The Milestone 0 tile-transport spike's raw wire format (ADR 0004): a 16-byte
 * header, then RGBA. The product no longer uses it; its tiles arrive as BMP
 * images decoded by the webview (ADR 0011).
 */
export interface RawBitmap {
  width: number;
  height: number;
  renderMicros: number;
  fromCache: boolean;
  /** Straight RGBA, row-major, exactly width * height * 4 bytes. */
  rgba: Uint8ClampedArray<ArrayBuffer>;
}

/** Bytes of little-endian header before the pixels: width, height, render µs, flags. */
export const BITMAP_HEADER_LEN = 16;

/** Header flag bit: served from the engine's tile cache. */
export const FLAG_FROM_CACHE = 1;

/**
 * Decodes a bitmap response without copying the pixels: the returned `rgba`
 * is a view over the response buffer.
 */
export function decodeBitmap(buffer: ArrayBuffer): RawBitmap {
  if (buffer.byteLength < BITMAP_HEADER_LEN) {
    throw new Error("bitmap response is shorter than its header");
  }
  const header = new DataView(buffer, 0, BITMAP_HEADER_LEN);
  const width = header.getUint32(0, true);
  const height = header.getUint32(4, true);
  const renderMicros = header.getUint32(8, true);
  const flags = header.getUint32(12, true);
  const pixelBytes = width * height * 4;
  if (width === 0 || height === 0 || buffer.byteLength !== BITMAP_HEADER_LEN + pixelBytes) {
    throw new Error("bitmap response does not match its declared size");
  }
  return {
    width,
    height,
    renderMicros,
    fromCache: (flags & FLAG_FROM_CACHE) !== 0,
    rgba: new Uint8ClampedArray(buffer, BITMAP_HEADER_LEN, pixelBytes),
  };
}
