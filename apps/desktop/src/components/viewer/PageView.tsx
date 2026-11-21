import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { backend, isBackendError } from "../../backend";
import { formatNumber } from "../../i18n";
import { type OpenDocument, useAppStore } from "../../state/appStore";
import { CSS_PX_PER_POINT, fitWidthZoom, renderScale } from "../../state/zoom";
import styles from "./PageView.module.css";

/** Fired on `window` after a page bitmap reaches the canvas. Used by the self-test. */
export const PAGE_PAINTED_EVENT = "safha:page-painted";

export interface PagePaintedDetail {
  documentId: number;
  page: number;
  width: number;
  height: number;
  /** Engine rasterization time. */
  renderMicros: number;
  /** Request sent → pixels on the canvas, as seen by the UI. */
  roundTripMs: number;
}

/** Wait this long after a zoom/resize settles before re-rasterizing. */
const RERENDER_DELAY_MS = 80;

/**
 * Shows one page of a document on a canvas.
 *
 * Milestone 0 scope: a single whole-page bitmap. The canvas is resized in CSS
 * immediately when the zoom changes (so the old bitmap stretches as a preview)
 * and is then re-rasterized at device resolution. Tiling, continuous scroll,
 * and the render cache are Milestone 1.
 */
export function PageView({ document }: { document: OpenDocument }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const updateDocument = useAppStore((state) => state.updateDocument);
  const setError = useAppStore((state) => state.setError);

  const viewportRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const hasBitmapRef = useRef(false);
  const [availableWidth, setAvailableWidth] = useState(0);
  const [isRendering, setIsRendering] = useState(false);

  const { id, page, pageSize, zoomMode } = document;

  // Track the width available to the page (content box, so padding is excluded).
  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setAvailableWidth(Math.floor(entry.contentRect.width));
    });
    observer.observe(viewport);
    return () => observer.disconnect();
  }, []);

  // Load the size of the page in view.
  useEffect(() => {
    let cancelled = false;
    hasBitmapRef.current = false;
    backend
      .pageSize(id, page)
      .then((size) => {
        if (!cancelled) updateDocument(id, { pageSize: size });
      })
      .catch((error: unknown) => {
        if (!cancelled) setError(isBackendError(error) ? error.code : "generic");
      });
    return () => {
      cancelled = true;
    };
  }, [id, page, updateDocument, setError]);

  // The zoom to draw at. In fit-width mode it is derived from the layout and is
  // unknown until both the page size and the viewport width are.
  let zoom: number | null = document.zoom;
  if (zoomMode === "fit-width") {
    zoom = pageSize && availableWidth > 0 ? fitWidthZoom(pageSize.width, availableWidth) : null;
  }

  // Keep the store's zoom (shown in the status bar) in step with fit-width.
  useEffect(() => {
    if (zoomMode === "fit-width" && zoom !== null && Math.abs(zoom - document.zoom) > 0.0005) {
      updateDocument(id, { zoom });
    }
  }, [zoomMode, zoom, document.zoom, id, updateDocument]);

  // Rasterize.
  useEffect(() => {
    if (!pageSize || zoom === null) return;
    let cancelled = false;
    const scale = renderScale(zoom, window.devicePixelRatio || 1, pageSize);

    const render = async () => {
      setIsRendering(true);
      const requestedAt = performance.now();
      try {
        const bitmap = await backend.renderPage(id, page, scale);
        const canvas = canvasRef.current;
        if (cancelled || !canvas) return;
        canvas.width = bitmap.width;
        canvas.height = bitmap.height;
        canvas
          .getContext("2d")
          ?.putImageData(new ImageData(bitmap.rgba, bitmap.width, bitmap.height), 0, 0);
        hasBitmapRef.current = true;
        const detail: PagePaintedDetail = {
          documentId: id,
          page,
          width: bitmap.width,
          height: bitmap.height,
          renderMicros: bitmap.renderMicros,
          roundTripMs: performance.now() - requestedAt,
        };
        window.dispatchEvent(new CustomEvent(PAGE_PAINTED_EVENT, { detail }));
      } catch (error) {
        if (!cancelled) setError(isBackendError(error) ? error.code : "generic");
      } finally {
        if (!cancelled) setIsRendering(false);
      }
    };

    // First paint of a page is immediate; later zoom/resize changes are coalesced.
    const timer = window.setTimeout(render, hasBitmapRef.current ? RERENDER_DELAY_MS : 0);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [id, page, pageSize, zoom, setError]);

  const cssWidth = pageSize && zoom !== null ? pageSize.width * CSS_PX_PER_POINT * zoom : 0;
  const cssHeight = pageSize && zoom !== null ? pageSize.height * CSS_PX_PER_POINT * zoom : 0;

  return (
    <div className={styles.viewport} ref={viewportRef}>
      <div className={styles.sheet} style={{ inlineSize: cssWidth, blockSize: cssHeight }}>
        <canvas
          ref={canvasRef}
          className={styles.canvas}
          role="img"
          aria-label={t("viewer.canvasLabel", {
            page: formatNumber(page + 1, locale),
            name: document.name,
          })}
          aria-busy={isRendering}
          data-testid="page-canvas"
        />
      </div>
      <span className="visually-hidden" aria-live="polite">
        {isRendering ? t("viewer.rendering") : ""}
      </span>
    </div>
  );
}
