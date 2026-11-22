import {
  ArrowLeft,
  ArrowRight,
  Copy,
  FileInput,
  FileOutput,
  FilePlus2,
  RotateCcw,
  RotateCw,
  Scissors,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  GridLayout,
  GridList,
  GridListItem,
  type Key,
  Size,
  useDragAndDrop,
  useLocale,
  Virtualizer,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { backend, type PageSize } from "../../backend";
import { dropTarget, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { PageImages } from "../../viewer/pageImages";
import { CommandButton } from "../ui/CommandButton";
import styles from "./PageGrid.module.css";

/** A design token in CSS px, read from the root (`tokens.css`). */
function tokenPx(name: string, fallback: number): number {
  const value = Number.parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue(name),
  );
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

/** Width of a page image in the grid (`--size-page-grid-image`). */
const IMAGE_WIDTH = tokenPx("--size-page-grid-image", 168);
/** Room around an image: padding and outline on each side, and the label below. */
const ITEM_CHROME = {
  inline: 2 * tokenPx("--space-3", 12),
  block: 2 * tokenPx("--space-3", 12) + tokenPx("--space-6", 24),
};
const GAP = tokenPx("--size-page-grid-gap", 16);

/**
 * The page organizer (spec section 5.4): every page of the document in a
 * grid. Select pages (click, Shift+click, Cmd/Ctrl+click, arrows), then
 * rotate, delete, duplicate, insert, or move them; drag them to reorder, by
 * pointer or keyboard. Opening a page (double-click, Enter) shows it in the
 * page view. Only the pages near the visible part are mounted.
 */
export function PageGrid({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const { direction } = useLocale();
  const locale = useAppStore((state) => state.locale);
  const { name, revision, paint, selected, labels } = useAppStore(
    useShallow((state) => {
      const open = state.documents.find((candidate) => candidate.id === id);
      return {
        name: open?.name ?? "",
        revision: open?.revision ?? 0,
        paint: open?.paint ?? 0,
        selected: open?.selectedPages ?? [],
        labels: open?.labels ?? null,
      };
    }),
  );
  const [sizes, setSizes] = useState<PageSize[] | null>(null);
  const imagesRef = useRef<PageImages | null>(null);

  // A new revision (an edit, undo, or redo) starts over from the document's pages.
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new revision reloads the pages
  useEffect(() => {
    const images = new PageImages(backend, id, IMAGE_WIDTH);
    imagesRef.current = images;
    let live = true;
    setSizes(null);
    backend.pageSizes(id).then(
      (next) => {
        if (!live) return;
        images.setSizes(next);
        setSizes(next);
      },
      (error: unknown) => console.warn("page organizer: page sizes unavailable", error),
    );
    return () => {
      live = false;
      images.destroy();
      if (imagesRef.current === images) imagesRef.current = null;
    };
  }, [id, revision]);

  // Only annotations changed: the same pages, drawn again.
  const paintShown = useRef(paint);
  useEffect(() => {
    if (paint === paintShown.current) return;
    paintShown.current = paint;
    imagesRef.current?.refresh();
  }, [paint]);

  const items = useMemo(() => (sizes ?? []).map((size, page) => ({ id: page, size })), [sizes]);
  const pageName = (page: number) => {
    const number = formatNumber(page + 1, locale);
    const label = labels?.[page];
    return label
      ? t("thumbnails.labelledName", { label, page: number })
      : t("thumbnails.name", { page: number });
  };

  const { dragAndDropHooks } = useDragAndDrop({
    getItems: (keys) => [...keys].map((key) => ({ "text/plain": pageName(Number(key)) })),
    onReorder(event) {
      const pages = [...event.keys].map(Number).sort((a, b) => a - b);
      const to = dropTarget(pages, Number(event.target.key), event.target.dropPosition === "after");
      void runCommand("pages.move", { pages, to });
    },
  });

  const layoutOptions = useMemo(
    () => ({
      minItemSize: new Size(IMAGE_WIDTH + ITEM_CHROME.inline, IMAGE_WIDTH + ITEM_CHROME.block),
      maxItemSize: new Size(IMAGE_WIDTH + ITEM_CHROME.inline, 2 * IMAGE_WIDTH + ITEM_CHROME.block),
      minSpace: new Size(GAP, GAP),
      preserveAspectRatio: false,
      direction,
    }),
    [direction],
  );

  return (
    <section className={styles.organizer} aria-label={t("pages.organizer", { name })}>
      <div className={styles.toolbar} role="toolbar" aria-label={t("pages.actions")}>
        <CommandButton command="pages.rotateCounterclockwise">
          <RotateCcw aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.rotateClockwise">
          <RotateCw aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.duplicate">
          <Copy aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.insertBlank">
          <FilePlus2 aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.moveEarlier" mirrorInRtl>
          <ArrowLeft aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.moveLater" mirrorInRtl>
          <ArrowRight aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.delete">
          <Trash2 aria-hidden="true" />
        </CommandButton>
        <span className={styles.divider} aria-hidden="true" />
        <CommandButton command="pages.insertFromFile">
          <FileInput aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.extract">
          <FileOutput aria-hidden="true" />
        </CommandButton>
        <CommandButton command="pages.split">
          <Scissors aria-hidden="true" />
        </CommandButton>
        <span className={styles.count} aria-live="polite">
          {selected.length > 0
            ? t("pages.selected", { count: formatNumber(selected.length, locale) })
            : ""}
        </span>
      </div>
      <Virtualizer layout={GridLayout} layoutOptions={layoutOptions}>
        <GridList
          aria-label={t("pages.grid", { name })}
          className={styles.grid}
          layout="grid"
          items={items}
          selectionMode="multiple"
          selectionBehavior="replace"
          selectedKeys={new Set<Key>(selected)}
          onSelectionChange={(keys) => {
            const all = keys === "all" ? items.map((item) => item.id) : [...keys].map(Number);
            useAppStore.getState().setSelectedPages(id, all);
          }}
          onAction={(key) => void runCommand("pages.open", { page: Number(key) })}
          dragAndDropHooks={dragAndDropHooks}
        >
          {(item) => (
            <GridListItem id={item.id} textValue={pageName(item.id)} className={styles.item}>
              <PageImage images={imagesRef} page={item.id} size={item.size} />
              <span className={styles.label}>
                {labels?.[item.id] ?? formatNumber(item.id + 1, locale)}
              </span>
            </GridListItem>
          )}
        </GridList>
      </Virtualizer>
    </section>
  );
}

/** A page's preview, drawn when its render arrives. */
function PageImage({
  images,
  page,
  size,
}: {
  images: React.RefObject<PageImages | null>;
  page: number;
  size: PageSize;
}) {
  const boxRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const box = boxRef.current;
    const source = images.current;
    if (!box || !source) return;
    return source.subscribe(page, (image) => {
      if (image.parentElement !== box) box.replaceChildren(image);
    });
  }, [images, page]);
  const height = Math.round((IMAGE_WIDTH * size.height) / Math.max(1, size.width));
  return (
    <div
      ref={boxRef}
      className={styles.image}
      style={{ inlineSize: IMAGE_WIDTH, blockSize: Math.min(height, 2 * IMAGE_WIDTH) }}
    />
  );
}
