import { ChevronLeft, ChevronRight, MoveHorizontal, ZoomIn, ZoomOut } from "lucide-react";
import { useTranslation } from "react-i18next";
import { formatNumber } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { CommandButton } from "../ui/CommandButton";
import styles from "./StatusBar.module.css";

export function StatusBar() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const document = useAppStore(selectActiveDocument);

  return (
    <footer className={styles.bar} data-testid="status-bar">
      {document ? (
        <>
          <div className={styles.group}>
            {/* Chevrons mean "towards the start/end of the document", so they mirror in RTL. */}
            <CommandButton command="view.previousPage" tooltipPlacement="top" mirrorInRtl>
              <ChevronLeft />
            </CommandButton>
            <span className={styles.label} aria-live="polite">
              {t("status.pageOf", {
                page: formatNumber(document.page + 1, locale),
                count: formatNumber(document.pageCount, locale),
              })}
            </span>
            <CommandButton command="view.nextPage" tooltipPlacement="top" mirrorInRtl>
              <ChevronRight />
            </CommandButton>
          </div>
          <div className={styles.group}>
            <CommandButton command="view.zoomOut" tooltipPlacement="top">
              <ZoomOut />
            </CommandButton>
            <span className={`${styles.label} ${styles.zoom}`}>
              {t("status.zoom", { percent: formatNumber(Math.round(document.zoom * 100), locale) })}
            </span>
            <CommandButton command="view.zoomIn" tooltipPlacement="top">
              <ZoomIn />
            </CommandButton>
            <CommandButton command="view.fitWidth" tooltipPlacement="top">
              <MoveHorizontal />
            </CommandButton>
          </div>
        </>
      ) : null}
    </footer>
  );
}
