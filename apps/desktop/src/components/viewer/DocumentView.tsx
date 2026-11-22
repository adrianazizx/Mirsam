import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { backend, isBackendError } from "../../backend";
import { isMac } from "../../commands/platform";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { ViewerController, type ViewerSpacing } from "../../viewer/controller";
import { registerViewer, unregisterViewer } from "../../viewer/registry";
import styles from "./DocumentView.module.css";

/** Reads the viewer's spacing tokens (tokens.css); 0 where CSS is unavailable (tests). */
function readSpacing(element: HTMLElement): ViewerSpacing {
  const style = getComputedStyle(element);
  const px = (name: string) => {
    const value = Number.parseFloat(style.getPropertyValue(name));
    return Number.isFinite(value) ? value : 0;
  };
  return {
    rowGap: px("--size-page-gap"),
    padding: px("--size-viewer-padding"),
    spreadGap: px("--size-spread-gap"),
  };
}

/**
 * The scrolling page area for one document. React owns only this container;
 * the pages, previews, and tiles inside it are managed by `ViewerController`
 * so scrolling never waits on a React render.
 */
export function DocumentView({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const containerRef = useRef<HTMLElement>(null);
  const viewerRef = useRef<ViewerController | null>(null);
  // The view settings only: page changes (reported by the viewer itself) do not re-render this.
  const { name, zoomMode, zoom, viewMode, pageOrder } = useAppStore(
    useShallow((state) => {
      const open = state.documents.find((candidate) => candidate.id === id);
      return {
        name: open?.name ?? "",
        zoomMode: open?.zoomMode ?? "fit-width",
        zoom: open?.zoom ?? 1,
        viewMode: open?.viewMode ?? "continuous",
        pageOrder: open?.pageOrder ?? "ltr",
      };
    }),
  );

  const pageLabel = useRef<(page: number) => string>(() => "");
  pageLabel.current = (page) =>
    t("viewer.canvasLabel", { page: formatNumber(page + 1, locale), name });

  useEffect(() => {
    const container = containerRef.current;
    const current = useAppStore.getState().documents.find((open) => open.id === id);
    if (!container || !current) return;
    const update = useAppStore.getState().updateDocument;
    const viewer = new ViewerController({
      backend,
      documentId: id,
      container,
      classNames: {
        page: styles.page ?? "",
        tiles: styles.tiles ?? "",
        tile: styles.tile ?? "",
        preview: styles.preview ?? "",
      },
      initial: {
        zoomMode: current.zoomMode,
        zoom: current.zoom,
        viewMode: current.viewMode,
        pageOrder: current.pageOrder,
        page: current.page,
      },
      spacing: readSpacing(container),
      isMac,
      pageLabel: (page) => pageLabel.current(page),
      callbacks: {
        onCurrentPage: (page, size) => update(id, { page, pageSize: size }),
        onZoom: (next, mode) => update(id, { zoom: next, zoomMode: mode }),
        onFatal: (error) =>
          useAppStore.getState().setError(isBackendError(error) ? error.code : "generic"),
      },
    });
    viewerRef.current = viewer;
    registerViewer(id, viewer);
    return () => {
      unregisterViewer(id, viewer);
      viewer.destroy();
      viewerRef.current = null;
    };
  }, [id]);

  useEffect(() => {
    viewerRef.current?.setView({ zoomMode, zoom, viewMode, pageOrder });
  }, [zoomMode, zoom, viewMode, pageOrder]);

  // Page labels follow the UI language.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run when the label inputs change
  useEffect(() => {
    viewerRef.current?.refreshLabels();
  }, [locale, name]);

  return (
    <section
      ref={containerRef}
      className={styles.viewport}
      // biome-ignore lint/a11y/noNoninteractiveTabindex: a scrollable region must be reachable from the keyboard (WCAG 2.1.1)
      tabIndex={0}
      aria-label={t("viewer.pagesLabel", { name })}
      data-testid="viewer"
    />
  );
}
