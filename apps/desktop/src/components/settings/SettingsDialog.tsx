import { useId } from "react";
import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { OcrModelsSection } from "./OcrModelsSection";
import styles from "./SettingsDialog.module.css";

/**
 * Settings (Cmd/Ctrl+,). For now one section: the OCR models (ADR 0034).
 * Language and theme stay on their own commands.
 */
export function SettingsDialog() {
  const { t } = useTranslation();
  const open = useAppStore((state) => state.settingsOpen);
  const ocrHeading = useId();
  if (!open) return null;
  const close = () => void runCommand("app.closeSettings");

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) close();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="settings">
          <Heading slot="title" className={styles.title}>
            {t("settings.title")}
          </Heading>
          <section className={styles.body} aria-labelledby={ocrHeading}>
            <h3 id={ocrHeading} className={styles.sectionTitle}>
              {t("ocrModels.heading")}
            </h3>
            <OcrModelsSection />
          </section>
          <div className={styles.footer}>
            <Button variant="primary" onPress={close}>
              {t("settings.done")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
