import { TriangleAlert } from "lucide-react";
import { Trans, useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./BlockEditBar.module.css";

/**
 * Sits above a block of page text being edited in place (ADR 0024): which
 * font the text is set in, flagged when it is not the PDF's own (spec §7.4),
 * and Done and Cancel. Font names come from the document, so they are
 * isolated from the interface's direction.
 */
export function BlockEditBar({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const edit = useAppStore((state) =>
    state.blockEdit?.documentId === documentId ? state.blockEdit : null,
  );
  if (!edit) return null;
  const { font } = edit;
  const substitute = font.kind === "substitute";
  const key = substitute
    ? font.original
      ? "blockEdit.substitute"
      : "blockEdit.substituteUnnamed"
    : font.kind === "sameFamily"
      ? "blockEdit.sameFamily"
      : "blockEdit.exact";
  return (
    <div className={styles.band}>
      <div
        className={styles.bar}
        role="toolbar"
        aria-label={t("blockEdit.label")}
        dir={directionOf(locale)}
        data-testid="block-edit-bar"
      >
        <span
          className={substitute ? styles.substitute : styles.font}
          data-testid="block-edit-font"
          data-kind={font.kind}
        >
          {substitute ? <TriangleAlert aria-hidden="true" className={styles.icon} /> : null}
          <span>
            <Trans
              i18nKey={key}
              values={{ font: font.used, original: font.original }}
              components={{ bdi: <bdi /> }}
            />
          </span>
        </span>
        {edit.mixed ? (
          <span className={styles.substitute} data-testid="block-edit-mixed">
            <TriangleAlert aria-hidden="true" className={styles.icon} />
            {t("blockEdit.mixed")}
          </span>
        ) : null}
        <Button variant="ghost" onPress={() => void runCommand("text.cancelBlock")}>
          {t("commands.text.cancelBlock")}
        </Button>
        <Button variant="primary" onPress={() => void runCommand("text.commitBlock")}>
          {t("commands.text.commitBlock")}
        </Button>
      </div>
    </div>
  );
}
