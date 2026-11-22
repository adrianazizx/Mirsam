import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { backend, isBackendError } from "../../backend";
import { isMac } from "../../commands/platform";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { ViewerController, type ViewerSpacing } from "../../viewer/controller";
import { registerViewer, unregisterViewer } from "../../viewer/registry";
import styles from "./DocumentView.module.css";
import { SelectionToolbar } from "./SelectionToolbar";

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

/** The remembered position is written this long after the reader stops moving. */
const SAVE_DELAY_MS = 800;
/** Documents whose page labels were asked for (once per open). */
/** `id@revision` of each document revision whose page labels were requested. */
const labelsRequested = new Set<string>();

/**
 * The scrolling page area for one document. React owns only this container;
 * the pages, previews, tiles, and highlights inside it are managed by
 * `ViewerController` so scrolling never waits on a React render.
 */
export function DocumentView({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const containerRef = useRef<HTMLElement>(null);
  const viewerRef = useRef<ViewerController | null>(null);
  const [toolbarHost, setToolbarHost] = useState<HTMLElement | null>(null);
  // The view settings only: page changes (reported by the viewer itself) do not re-render this.
  const { name, zoomMode, zoom, viewMode, pageOrder, hasSelection } = useAppStore(
    useShallow((state) => {
      const open = state.documents.find((candidate) => candidate.id === id);
      return {
        name: open?.name ?? "",
        zoomMode: open?.zoomMode ?? "fit-width",
        zoom: open?.zoom ?? 1,
        viewMode: open?.viewMode ?? "continuous",
        pageOrder: open?.pageOrder ?? "ltr",
        hasSelection: open?.hasSelection ?? false,
      };
    }),
  );
  // A new revision (an edit, undo, or redo) rebuilds the view from its pages.
  const revision = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.revision ?? 0,
  );
  const hits = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.search.hits ?? null,
  );
  const activeHit = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.search.active ?? -1,
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
        overlay: styles.overlay ?? "",
        searchHit: styles.searchHit ?? "",
        searchHitActive: styles.searchHitActive ?? "",
        selection: styles.selection ?? "",
        selectionToolbar: styles.selectionToolbar ?? "",
      },
      initial: {
        zoomMode: current.zoomMode,
        zoom: current.zoom,
        viewMode: current.viewMode,
        pageOrder: current.pageOrder,
        page: current.page,
        offset: current.pageOffset,
      },
      spacing: readSpacing(container),
      isMac,
      pageLabel: (page) => pageLabel.current(page),
      canSelect: () => useAppStore.getState().activeTool === "select",
      callbacks: {
        onCurrentPage: (page, size) => update(id, { page, pageSize: size }),
        onZoom: (next, mode) => update(id, { zoom: next, zoomMode: mode }),
        onFatal: (error) =>
          useAppStore.getState().setError(isBackendError(error) ? error.code : "generic"),
        onSelectionChange: (has) => update(id, { hasSelection: has }),
      },
    });
    viewerRef.current = viewer;
    registerViewer(id, viewer);
    setToolbarHost(viewer.selectionToolbarHost);

    // Remember where the reader is, a moment after they stop moving.
    let timer: number | null = null;
    const save = () => {
      timer = null;
      const open = useAppStore.getState().documents.find((candidate) => candidate.id === id);
      if (!open) return;
      const offset = viewer.currentOffset();
      useAppStore.getState().updateDocument(id, { pageOffset: offset });
      backend
        .saveViewState(id, {
          page: open.page,
          zoomMode: open.zoomMode,
          zoom: open.zoom,
          pageOffset: offset,
        })
        .catch((error: unknown) => console.warn("could not save the position", error));
    };
    const onScroll = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(save, SAVE_DELAY_MS);
    };
    container.addEventListener("scroll", onScroll, { passive: true });

    const labelKey = `${id}@${revision}`;
    if (current.labels === null && !labelsRequested.has(labelKey)) {
      labelsRequested.add(labelKey);
      backend.pageLabels(id).then(
        (labels) => {
          if (labels) useAppStore.getState().updateDocument(id, { labels });
        },
        (error: unknown) => console.warn("page labels unavailable", error),
      );
    }

    return () => {
      container.removeEventListener("scroll", onScroll);
      if (timer !== null) {
        window.clearTimeout(timer);
        save();
      }
      unregisterViewer(id, viewer);
      viewer.destroy();
      viewerRef.current = null;
      setToolbarHost(null);
    };
  }, [id, revision]);

  useEffect(() => {
    viewerRef.current?.setView({ zoomMode, zoom, viewMode, pageOrder });
  }, [zoomMode, zoom, viewMode, pageOrder]);

  useEffect(() => {
    viewerRef.current?.setSearchHits(hits ?? [], activeHit);
  }, [hits, activeHit]);

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
    >
      {toolbarHost && hasSelection ? createPortal(<SelectionToolbar />, toolbarHost) : null}
    </section>
  );
}
