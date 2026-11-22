import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./RedactDialog.module.css";

/**
 * Asks before applying redactions (ADR 0042): they remove content from the
 * file. Undo covers them until the document is saved; the dialog says so.
 */
export function RedactDialog() {
  const { t } = useTranslation();
  const open = useAppStore((state) => state.redactPrompt);
  const locale = useAppStore((state) => state.locale);
  const marks = useAppStore((state) => selectActiveDocument(state)?.redaction.marks.length ?? 0);
  if (!open || marks === 0) return null;
  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) void runCommand("redact.cancel");
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="redact-dialog">
          <Heading slot="title" className={styles.title}>
            {t("redactDialog.title")}
          </Heading>
          <p className={styles.body}>
            {t("redactDialog.body", { marks: formatNumber(marks, locale) })}
          </p>
          <div className={styles.actions}>
            <Button variant="primary" onPress={() => void runCommand("redact.confirm")}>
              {t("redactDialog.confirm")}
            </Button>
            <Button variant="ghost" autoFocus onPress={() => void runCommand("redact.cancel")}>
              {t("redactDialog.cancel")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
