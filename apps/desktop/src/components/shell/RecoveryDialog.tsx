import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { formatTag } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./RecoveryDialog.module.css";

/**
 * Offers the unsaved changes an earlier session left (ADR 0040), once, at
 * the app's start. Each document is restored or discarded on its own;
 * "Decide later" keeps them all for the next start. Every answer is a command.
 */
export function RecoveryDialog() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const recoveries = useAppStore((state) => state.recoveries);
  if (recoveries.length === 0) return null;
  const later = () => void runCommand("recovery.later");
  const time = new Intl.DateTimeFormat(formatTag(locale), {
    dateStyle: "medium",
    timeStyle: "short",
  });

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(open) => {
        if (!open) later();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="recovery">
          <Heading slot="title" className={styles.title}>
            {t("recovery.title")}
          </Heading>
          <p className={styles.message}>{t("recovery.message")}</p>
          <ul className={styles.list}>
            {recoveries.map((entry, index) => (
              <li key={entry.token} className={styles.entry}>
                <div className={styles.details}>
                  {/* A file name can be in either script regardless of UI language. */}
                  <bdi className={styles.name}>{entry.name}</bdi>
                  <span className={styles.time}>
                    {t("recovery.savedAt", { time: time.format(entry.savedAt * 1000) })}
                  </span>
                  {entry.originalChanged && (
                    <span className={styles.changed}>{t("recovery.changed")}</span>
                  )}
                </div>
                <div className={styles.actions}>
                  <Button
                    variant="primary"
                    autoFocus={index === 0}
                    onPress={() => void runCommand("recovery.restore", { token: entry.token })}
                  >
                    {t("recovery.restore")}
                  </Button>
                  <Button
                    onPress={() => void runCommand("recovery.discard", { token: entry.token })}
                  >
                    {t("recovery.discard")}
                  </Button>
                </div>
              </li>
            ))}
          </ul>
          <div className={styles.footer}>
            <Button variant="ghost" onPress={later}>
              {t("recovery.later")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
