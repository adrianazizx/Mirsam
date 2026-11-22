import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { formatBytes, isolate } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { bytesToDownload, missingRequired, receivedOf, useOcrModels } from "../../state/ocrModels";
import { Button } from "../ui/Button";
import styles from "./OcrModelPrompt.module.css";

/**
 * Asked the first time OCR is used without its model (spec §8: the download is
 * explained before it starts): what the model is, that it runs locally, its
 * size, and whether this machine will run it slowly.
 */
export function OcrModelPrompt() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const open = useOcrModels((state) => state.prompt);
  const models = useOcrModels((state) => state.models);
  const missing = missingRequired(models);
  const primary = missing.find((pkg) => pkg.role === "primary") ?? missing[0];
  if (!open || !models || !primary) return null;
  const later = () => void runCommand("ocrModels.dismissPrompt");
  const received = missing.reduce(
    (sum, pkg) => sum + (pkg.state.kind === "partial" ? receivedOf(pkg) : 0),
    0,
  );

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) later();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="ocr-prompt">
          <Heading slot="title" className={styles.title}>
            {t("ocrPrompt.title")}
          </Heading>
          <p className={styles.message}>{t("ocrPrompt.body", { name: isolate(primary.name) })}</p>
          <p className={styles.message}>
            {t("ocrPrompt.size", { size: formatBytes(bytesToDownload(missing), locale) })}
            {received > 0
              ? ` ${t("ocrPrompt.resumed", { received: formatBytes(received, locale) })}`
              : null}
          </p>
          {models.hardware.runtimeGpu === null ? (
            <p className={styles.warning}>{t("ocrPrompt.slow")}</p>
          ) : null}
          <div className={styles.footer}>
            <Button variant="ghost" onPress={later}>
              {t("ocrPrompt.later")}
            </Button>
            <Button
              variant="primary"
              autoFocus
              onPress={() => void runCommand("ocrModels.downloadRequired")}
            >
              {t("ocrPrompt.download")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
