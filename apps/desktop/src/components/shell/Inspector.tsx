import { useTranslation } from "react-i18next";
import { formatNumber } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { FormsPanel } from "../forms/FormsPanel";
import { SignPanel } from "../forms/SignPanel";
import { OcrPanel } from "../ocr/OcrPanel";
import { ProtectPanel } from "../protect/ProtectPanel";
import styles from "./Inspector.module.css";

const MM_PER_POINT = 25.4 / 72;
/** Tools whose own panel the inspector shows. */
const TOOL_PANELS: string[] = ["protect", "forms", "sign"];

/**
 * Contextual properties panel. With nothing selected it shows document info
 * (docs/SPEC.md section 4); tool- and selection-specific panels arrive with
 * their features.
 */
export function Inspector() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const document = useAppStore(selectActiveDocument);
  const tool = useAppStore((state) => state.activeTool);
  const mm = (points: number) =>
    formatNumber(points * MM_PER_POINT, locale, { maximumFractionDigits: 0 });

  return (
    <aside className={styles.inspector} aria-label={t("inspector.title")} data-testid="inspector">
      <h2 className={styles.title}>
        {t(
          tool === "ocr" || (document && TOOL_PANELS.includes(tool))
            ? `tools.${tool}`
            : "inspector.title",
        )}
      </h2>
      {tool === "ocr" ? (
        <OcrPanel />
      ) : document && tool === "protect" ? (
        <ProtectPanel key={document.id} document={document} />
      ) : document && tool === "forms" ? (
        <FormsPanel key={document.id} documentId={document.id} />
      ) : document && tool === "sign" ? (
        <SignPanel key={document.id} document={document} />
      ) : document ? (
        <dl className={styles.list}>
          <dt>{t("inspector.name")}</dt>
          <dd className={styles.selectable}>
            <bdi>{document.name}</bdi>
          </dd>
          <dt>{t("inspector.pages")}</dt>
          <dd>{formatNumber(document.pageCount, locale)}</dd>
          <dt>{t("inspector.pageSize")}</dt>
          <dd>
            {document.pageSize
              ? t("inspector.pageSizeValue", {
                  width: mm(document.pageSize.width),
                  height: mm(document.pageSize.height),
                })
              : "—"}
          </dd>
        </dl>
      ) : (
        <p className={styles.empty}>{t("inspector.empty")}</p>
      )}
    </aside>
  );
}
