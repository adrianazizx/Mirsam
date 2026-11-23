import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { backend, isBackendError } from "../../backend";
import { pageFields } from "../../commands/formDesign";
import { isMac } from "../../commands/platform";
import { runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { useOcrJobs } from "../../state/ocrJobs";
import {
  type FieldBox,
  ViewerController,
  type ViewerMode,
  type ViewerSpacing,
} from "../../viewer/controller";
import type { ReadableNote } from "../../viewer/readableText";
import { registerViewer, unregisterViewer } from "../../viewer/registry";
import { NoteEditor } from "../comments/NoteEditor";
import styles from "./DocumentView.module.css";
import { LinkTip } from "./LinkTip";
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

/** Annotations whose text the note editor changes, instead of the selection toolbar. */
const EDITED_IN_PLACE = ["note", "textBox", "fillText"] as const;

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
  const [noteHost, setNoteHost] = useState<HTMLElement | null>(null);
  const [linkTipHost, setLinkTipHost] = useState<HTMLElement | null>(null);
  // The view settings only: page changes (reported by the viewer itself) do not re-render this.
  const { name, zoomMode, zoom, viewMode, pageOrder, hasSelection, selectedKind } = useAppStore(
    useShallow((state) => {
      const open = state.documents.find((candidate) => candidate.id === id);
      return {
        name: open?.name ?? "",
        zoomMode: open?.zoomMode ?? "fit-width",
        zoom: open?.zoom ?? 1,
        viewMode: open?.viewMode ?? "continuous",
        pageOrder: open?.pageOrder ?? "ltr",
        hasSelection: open?.hasSelection ?? false,
        selectedKind: open?.selectedAnnotation?.kind ?? null,
      };
    }),
  );
  // A new revision (an edit, undo, or redo) rebuilds the view from its pages.
  const revision = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.revision ?? 0,
  );
  // Only annotations changed (ADR 0017): the same view paints its pages again.
  const paint = useAppStore((state) => state.documents.find((open) => open.id === id)?.paint ?? 0);
  const pendingNote = useAppStore((state) =>
    state.pendingNote?.documentId === id ? state.pendingNote : null,
  );
  const hits = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.search.hits ?? null,
  );
  const activeHit = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.search.active ?? -1,
  );

  const redactMarks = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.redaction.marks ?? null,
  );
  // The Edit tool's object and link modes (ADR 0051).
  const toolMode = useAppStore((state) => `${state.activeTool}:${state.editMode}`);
  const selectedObject = useAppStore((state) =>
    state.selectedObject?.documentId === id ? state.selectedObject : null,
  );
  const objectCrop = useAppStore((state) =>
    state.selectedObject?.documentId === id ? state.objectCrop : null,
  );
  const linkDraft = useAppStore((state) =>
    state.linkDraft?.documentId === id ? state.linkDraft : null,
  );
  const showsLinkTip = useAppStore((state) => state.linkTip?.documentId === id);
  // The form designer's fields and suggestions (ADR 0047), only while it is open on this document.
  const formDesign = useAppStore((state) =>
    state.activeTool === "forms" &&
    state.formsMode === "design" &&
    state.formDesign?.documentId === id
      ? state.formDesign
      : null,
  );

  const pageLabel = useRef<(page: number) => string>(() => "");
  pageLabel.current = (page) =>
    t("viewer.canvasLabel", { page: formatNumber(page + 1, locale), name });
  const readableNote = useRef<(kind: ReadableNote) => string>(() => "");
  readableNote.current = (kind) => t(kind === "noText" ? "a11y.noPageText" : "a11y.suspectText");

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
        reviewMark: styles.reviewMark ?? "",
        regionMark: styles.regionMark ?? "",
        selection: styles.selection ?? "",
        markupSelected: styles.markupSelected ?? "",
        inkPreview: styles.inkPreview ?? "",
        inkErasing: styles.inkErasing ?? "",
        notePending: styles.notePending ?? "",
        textBoxPending: styles.textBoxPending ?? "",
        noteEditor: styles.noteEditor ?? "",
        selectionToolbar: styles.selectionToolbar ?? "",
        redactMark: styles.redactMark ?? "",
        redactPreview: styles.redactPreview ?? "",
        gesture: styles.gesture ?? "",
        fillMoving: styles.fillMoving ?? "",
        fillHandle: styles.fillHandle ?? "",
        objectHover: styles.objectHover ?? "",
        objectSelected: styles.objectSelected ?? "",
        objectCrop: styles.objectCrop ?? "",
        linkArea: styles.linkArea ?? "",
        linkSelected: styles.linkSelected ?? "",
        linkHover: styles.linkHover ?? "",
        linkDraft: styles.linkDraft ?? "",
        linkTip: styles.linkTip ?? "",
        fieldBox: styles.fieldBox ?? "",
        fieldBoxSelected: styles.fieldBoxSelected ?? "",
        fieldTab: styles.fieldTab ?? "",
        fieldDraft: styles.fieldDraft ?? "",
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
      readableNote: (kind) => readableNote.current(kind),
      mode: (): ViewerMode => {
        const { activeTool, commentTool, fillTool, editMode, formsMode, designTool } =
          useAppStore.getState();
        if (activeTool === "forms" && formsMode === "design") return designTool ? "draw" : "design";
        if (activeTool === "select" || activeTool === "forms") return "select";
        if (activeTool === "sign") return fillTool ? "place" : "select";
        if (activeTool === "protect") return "redact";
        if (activeTool === "ocr" && useOcrJobs.getState().drawingRegion) return "region";
        if (activeTool === "edit") return editMode === "text" ? "text" : editMode;
        return activeTool === "comment" ? commentTool : "none";
      },
      callbacks: {
        onCurrentPage: (page, size) => update(id, { page, pageSize: size }),
        onZoom: (next, mode) => update(id, { zoom: next, zoomMode: mode }),
        onFatal: (error) =>
          useAppStore.getState().setError(isBackendError(error) ? error.code : "generic"),
        onSelectionChange: (has) => update(id, { hasSelection: has }),
        onAnnotationSelect: (selected) => update(id, { selectedAnnotation: selected }),
        onInk: (page, strokes) => void runCommand("ink.add", { page, strokes }),
        onErase: (page, indices) => void runCommand("ink.erase", { page, indices }),
        onPlaceNote: (page, x, y) => void runCommand("note.place", { page, x, y }),
        onPlaceTextBox: (page, x, y, width) =>
          void runCommand("textBox.place", { page, x, y, width }),
        onMarkArea: (page, rect) => void runCommand("redact.addArea", { page, rect }),
        onRegion: (page, rect) => void runCommand("ocr.setRegion", { page, rect }),
        onDrawField: (page, rect) => void runCommand("formDesign.draw", { page, rect }),
        onSelectField: (field, widget) =>
          void runCommand("formDesign.selectField", { field: field ?? undefined, index: widget }),
        onMoveField: (field, widget, page, rect) =>
          void runCommand("formDesign.move", { field, index: widget, page, rect }),
        onPlace: (page, x, y) => void runCommand("fill.place", { page, x, y }),
        onMoveItem: (annotation, rect, page) =>
          void runCommand("fill.move", { annotation, rect, page }),
        onPlaceText: (page, x, y, width) => void runCommand("text.place", { page, x, y, width }),
        onEditBlock: (page, block) => void runCommand("text.editBlock", { page, block }),
        onBlockRefused: (block) =>
          useAppStore
            .getState()
            .setNotice({ key: `textNotEditable.${block.reason ?? "unsupported"}` }),
        onObjectSelect: (page, object) => void runCommand("object.select", { page, object }),
        onObjectReshape: (page, object, rect) =>
          void runCommand("object.reshape", { page, object, rect }),
        onCropChange: (rect) => void runCommand("object.setCrop", { rect }),
        onLinkSelect: (page, link) => void runCommand("link.select", { page, link }),
        onLinkDraw: (page, rect) => void runCommand("link.draw", { page, rect }),
        onLinkReshape: (page, link, rect) => void runCommand("link.reshape", { page, link, rect }),
        onLinkHover: (page, link) => void runCommand("link.hover", { page, link }),
        onLinkPress: (page, link) => void runCommand("link.follow", { page, link }),
      },
    });
    viewerRef.current = viewer;
    registerViewer(id, viewer);
    setToolbarHost(viewer.selectionToolbarHost);
    setNoteHost(viewer.noteEditorHost);
    setLinkTipHost(viewer.linkTipHost);

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
      setNoteHost(null);
      setLinkTipHost(null);
    };
  }, [id, revision]);

  const paintShown = useRef(paint);
  useEffect(() => {
    if (paint === paintShown.current) return;
    paintShown.current = paint;
    viewerRef.current?.refresh();
  }, [paint]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new viewer shows it again
  useEffect(() => {
    viewerRef.current?.showPendingNote(pendingNote);
  }, [pendingNote, noteHost]);

  useEffect(() => {
    viewerRef.current?.setView({ zoomMode, zoom, viewMode, pageOrder });
  }, [zoomMode, zoom, viewMode, pageOrder]);

  useEffect(() => {
    viewerRef.current?.setSearchHits(hits ?? [], activeHit);
  }, [hits, activeHit]);

  // The OCR word under review (ADR 0035).
  const reviewFocus = useOcrJobs((state) => (state.focus?.documentId === id ? state.focus : null));
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new viewer shows it again
  useEffect(() => {
    const rect = reviewFocus?.rect;
    viewerRef.current?.setReviewMark(reviewFocus && rect ? { page: reviewFocus.page, rect } : null);
  }, [reviewFocus, revision]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new viewer shows them again
  useEffect(() => {
    viewerRef.current?.setRedactionMarks(redactMarks ?? []);
  }, [redactMarks, revision]);

  // The area drawn to be read again (ADR 0038).
  const region = useOcrJobs((state) => (state.region?.documentId === id ? state.region : null));
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new viewer shows it again
  useEffect(() => {
    viewerRef.current?.setRegionMark(region ? { page: region.page, rect: region.rect } : null);
  }, [region, revision]);

  // The tool or the Edit tool's mode changed: the object and link marks are painted again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run when the tool changes
  useEffect(() => {
    viewerRef.current?.repaintOverlays();
  }, [toolMode]);

  // The store's object selection, crop, and link being edited, shown by the viewer.
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new viewer shows them again
  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    if (selectedObject) viewer.selectObject(selectedObject.page, selectedObject.object.index);
    else viewer.selectObject(0, null);
  }, [selectedObject, revision]);

  useEffect(() => {
    viewerRef.current?.setObjectCrop(objectCrop);
  }, [objectCrop]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new viewer shows them again
  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer) return;
    viewer.selectLink(linkDraft?.page ?? 0, linkDraft?.index ?? null);
    viewer.setLinkDraft(
      linkDraft && linkDraft.index === null
        ? { page: linkDraft.page, rects: linkDraft.rects }
        : null,
    );
  }, [linkDraft, revision]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new viewer shows them again
  useEffect(() => {
    const fields = formDesign?.fields ?? [];
    const boxes: FieldBox[] = [];
    const pages = new Set(
      fields.flatMap((field) =>
        field.widgets.flatMap((widget) => (widget.page === null ? [] : [widget.page])),
      ),
    );
    // Each widget numbered by its field's place in its page's tab order (a radio group's buttons share one).
    for (const page of pages) {
      pageFields(fields, page).forEach((field, order) => {
        let first = true;
        field.widgets.forEach((widget, index) => {
          if (widget.page !== page || !widget.view) return;
          // The number once per field: on its first widget here.
          boxes.push({
            field: field.id,
            widget: index,
            page,
            rect: widget.view,
            tab: first ? order + 1 : 0,
          });
          first = false;
        });
      });
    }
    const detected = formDesign?.detected;
    const suggestions = (detected?.fields ?? []).flatMap((field) =>
      field.include && field.view ? [{ page: detected?.page ?? 0, rect: field.view }] : [],
    );
    viewerRef.current?.setFieldBoxes(boxes, formDesign?.selected ?? null, suggestions);
  }, [formDesign, revision]);

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
      // F6 to the page area lands here (commands/regions.ts).
      data-region-focus=""
    >
      {toolbarHost &&
      (hasSelection ||
        (selectedKind !== null && !(EDITED_IN_PLACE as readonly string[]).includes(selectedKind)))
        ? createPortal(<SelectionToolbar documentId={id} />, toolbarHost)
        : null}
      {noteHost &&
      (pendingNote || (EDITED_IN_PLACE as readonly string[]).includes(selectedKind ?? ""))
        ? createPortal(<NoteEditor documentId={id} />, noteHost)
        : null}
      {linkTipHost && showsLinkTip ? createPortal(<LinkTip documentId={id} />, linkTipHost) : null}
    </section>
  );
}
