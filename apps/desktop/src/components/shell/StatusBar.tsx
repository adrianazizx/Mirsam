import { ChevronLeft, ChevronRight, MoveHorizontal, ZoomIn, ZoomOut } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { type CommandId, PAGE_NUMBER_INPUT_ID, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { type OpenDocument, selectActiveDocument, useAppStore } from "../../state/appStore";
import { parsePageNumber } from "../../viewer/pageNumber";
import { CommandButton } from "../ui/CommandButton";
import { CommandSelect } from "../ui/CommandSelect";
import styles from "./StatusBar.module.css";

const MODE_COMMANDS: Record<OpenDocument["viewMode"], CommandId> = {
  continuous: "view.modeContinuous",
  single: "view.modeSingle",
  spread: "view.modeSpread",
  book: "view.modeBook",
};

/** The current page as an editable number: type a page and press Enter to go there. */
function PageNumberField({ document }: { document: OpenDocument }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const [draft, setDraft] = useState<string | null>(null);
  const current = formatNumber(document.page + 1, locale, { useGrouping: false });

  const commit = () => {
    const page = draft === null ? null : parsePageNumber(draft);
    if (page !== null) void runCommand("view.goToPage", { page: page - 1 });
    setDraft(null);
  };

  return (
    <input
      id={PAGE_NUMBER_INPUT_ID}
      className={styles.pageInput}
      value={draft ?? current}
      inputMode="numeric"
      autoComplete="off"
      spellCheck={false}
      aria-label={t("status.pageNumber")}
      data-testid="page-number"
      onFocus={(event) => {
        setDraft(current);
        event.currentTarget.select();
      }}
      onChange={(event) => setDraft(event.currentTarget.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          commit();
          event.currentTarget.blur();
        } else if (event.key === "Escape") {
          setDraft(null);
          event.currentTarget.blur();
        }
      }}
      onBlur={() => setDraft(null)}
    />
  );
}

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
            <PageNumberField document={document} />
            <span className={styles.label}>
              {t("status.pageCount", { count: formatNumber(document.pageCount, locale) })}
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
          <div className={styles.group}>
            <CommandSelect
              label={t("status.viewMode")}
              value={MODE_COMMANDS[document.viewMode]}
              options={Object.values(MODE_COMMANDS).map((command) => ({ command }))}
              testId="view-mode"
            />
            <CommandSelect
              label={t("status.pageOrder")}
              value={
                document.pageOrder === "rtl" ? "view.pagesRightToLeft" : "view.pagesLeftToRight"
              }
              options={[
                { command: "view.pagesLeftToRight", shortLabel: t("status.orderLtr") },
                { command: "view.pagesRightToLeft", shortLabel: t("status.orderRtl") },
              ]}
              testId="page-order"
            />
          </div>
        </>
      ) : null}
    </footer>
  );
}
