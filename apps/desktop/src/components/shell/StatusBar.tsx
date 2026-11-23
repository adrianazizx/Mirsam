import {
  ChevronLeft,
  ChevronRight,
  MoveHorizontal,
  PanelLeft,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { type CommandId, PAGE_NUMBER_INPUT_ID, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import {
  type OpenDocument,
  type ReadingTheme,
  selectActiveDocument,
  useAppStore,
} from "../../state/appStore";
import { parsePageNumber } from "../../viewer/pageNumber";
import { SignatureBadge } from "../signing/SignatureBadge";
import { CommandButton } from "../ui/CommandButton";
import { CommandSelect } from "../ui/CommandSelect";
import styles from "./StatusBar.module.css";

const READING_COMMANDS: Record<ReadingTheme, CommandId> = {
  normal: "view.readingNormal",
  sepia: "view.readingSepia",
  night: "view.readingNight",
};

/** The page a typed value names: a page label ("iv") first, else a number. */
export function pageFromInput(value: string, labels: (string | null)[] | null): number | null {
  const trimmed = value.trim();
  const byLabel = labels?.findIndex((label) => label !== null && label === trimmed) ?? -1;
  if (byLabel >= 0) return byLabel;
  const number = parsePageNumber(trimmed);
  return number === null ? null : number - 1;
}

const MODE_COMMANDS: Record<OpenDocument["viewMode"], CommandId> = {
  continuous: "view.modeContinuous",
  single: "view.modeSingle",
  spread: "view.modeSpread",
  book: "view.modeBook",
};

/**
 * The current page as an editable field: type a page number, or the page's
 * label in a document that has labels ("iv"), and press Enter to go there.
 */
function PageNumberField({ document }: { document: OpenDocument }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const [draft, setDraft] = useState<string | null>(null);
  const label = document.labels?.[document.page] ?? null;
  const current = label ?? formatNumber(document.page + 1, locale, { useGrouping: false });

  const commit = () => {
    const page = draft === null ? null : pageFromInput(draft, document.labels);
    if (page !== null) void runCommand("view.goToPage", { page });
    setDraft(null);
  };

  return (
    <input
      id={PAGE_NUMBER_INPUT_ID}
      className={styles.pageInput}
      value={draft ?? current}
      inputMode={document.labels ? "text" : "numeric"}
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

function ReadingThemeSelect() {
  const { t } = useTranslation();
  const readingTheme = useAppStore((state) => state.readingTheme);
  return (
    <CommandSelect
      label={t("status.readingTheme")}
      value={READING_COMMANDS[readingTheme]}
      options={Object.values(READING_COMMANDS).map((command) => ({ command }))}
      testId="reading-theme"
    />
  );
}

export function StatusBar() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const document = useAppStore(selectActiveDocument);

  return (
    <footer className={styles.bar} data-testid="status-bar">
      <div className={styles.group}>
        {/* The icon draws a panel on its start side, so it mirrors with the layout. */}
        <CommandButton command="view.toggleLeftPanel" tooltipPlacement="top" mirrorInRtl>
          <PanelLeft />
        </CommandButton>
        <SignatureBadge />
      </div>
      {document ? (
        <>
          <div className={styles.group}>
            {/* Chevrons mean "towards the start/end of the document", so they mirror in RTL. */}
            <CommandButton command="view.previousPage" tooltipPlacement="top" mirrorInRtl>
              <ChevronLeft />
            </CommandButton>
            <PageNumberField document={document} />
            <span className={styles.label}>
              {document.labels?.[document.page]
                ? t("status.labelledCount", {
                    page: formatNumber(document.page + 1, locale),
                    count: formatNumber(document.pageCount, locale),
                  })
                : t("status.pageCount", { count: formatNumber(document.pageCount, locale) })}
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
            <ReadingThemeSelect />
          </div>
        </>
      ) : null}
    </footer>
  );
}
