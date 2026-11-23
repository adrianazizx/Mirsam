import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { backend } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { tokenPx } from "../../styles/tokens";
import { ThumbnailStrip } from "../../viewer/thumbnails";
import styles from "./ThumbnailsPanel.module.css";

/** Page thumbnails; the current page is marked and kept in view. */
export function ThumbnailsPanel({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const page = useAppStore((state) => state.documents.find((open) => open.id === id)?.page ?? 0);
  // A new revision (an edit, undo, or redo) rebuilds the strip from the document's pages.
  const revision = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.revision ?? 0,
  );
  // Only annotations changed: the same strip draws its thumbnails again.
  const paint = useAppStore((state) => state.documents.find((open) => open.id === id)?.paint ?? 0);
  const labels = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.labels ?? null,
  );
  const containerRef = useRef<HTMLDivElement>(null);
  const stripRef = useRef<ThumbnailStrip | null>(null);

  const label = useRef<(page: number) => { name: string; text: string }>(() => ({
    name: "",
    text: "",
  }));
  label.current = (index) => {
    const number = formatNumber(index + 1, locale);
    const pageLabel = labels?.[index];
    return {
      name: pageLabel
        ? t("thumbnails.labelledName", { label: pageLabel, page: number })
        : t("thumbnails.name", { page: number }),
      text: pageLabel ?? number,
    };
  };

  // A new revision, or another interface size, rebuilds the strip.
  const uiScale = useAppStore((state) => state.uiScale);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new revision or interface size rebuilds the strip
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const strip = new ThumbnailStrip({
      backend,
      documentId: id,
      container,
      width: tokenPx("--size-thumbnail", 128, container),
      // The label under each image grows with the interface size.
      chrome: tokenPx("--space-8", 32, container) + tokenPx("--space-2", 8, container),
      classNames: {
        list: styles.list ?? "",
        item: styles.item ?? "",
        image: styles.image ?? "",
        label: styles.label ?? "",
        current: styles.current ?? "",
      },
      label: (index) => label.current(index),
      onSelect: (target) => void runCommand("view.goToPage", { page: target }),
    });
    strip.setCurrent(useAppStore.getState().documents.find((open) => open.id === id)?.page ?? 0);
    stripRef.current = strip;
    return () => {
      strip.destroy();
      stripRef.current = null;
    };
  }, [id, revision, uiScale]);

  useEffect(() => {
    stripRef.current?.setCurrent(page);
  }, [page]);

  const paintShown = useRef(paint);
  useEffect(() => {
    if (paint === paintShown.current) return;
    paintShown.current = paint;
    stripRef.current?.refresh();
  }, [paint]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run when the label inputs change
  useEffect(() => {
    stripRef.current?.refreshLabels();
  }, [locale, labels]);

  return (
    <nav className={styles.scroller} aria-label={t("panel.thumbnails")}>
      <div ref={containerRef} className={styles.container} data-testid="thumbnails" />
    </nav>
  );
}
