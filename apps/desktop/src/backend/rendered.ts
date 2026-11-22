import type { RenderedPage } from "./types";

/** Draws a rendered rectangle with its top-left corner at (x, y). */
export function drawRendered(
  context: CanvasRenderingContext2D,
  page: RenderedPage,
  x = 0,
  y = 0,
): void {
  if ("data" in page.image) context.putImageData(page.image, x, y);
  else context.drawImage(page.image, x, y);
}

/**
 * Frees a rendered rectangle's pixels now rather than at the next garbage
 * collection: an `ImageBitmap` is closed, an image element let go of its source.
 */
export function releaseRendered(page: RenderedPage): void {
  const image = page.image;
  if ("close" in image) image.close();
  else if ("src" in image) image.src = "";
}

/** A canvas holding a rendered rectangle at its own size. */
export function renderedCanvas(page: RenderedPage): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = page.width;
  canvas.height = page.height;
  // Pages are opaque; an opaque backing store composites faster.
  const context = canvas.getContext("2d", { alpha: false });
  if (context) drawRendered(context, page);
  return canvas;
}
