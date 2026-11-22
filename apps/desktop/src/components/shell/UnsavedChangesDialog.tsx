import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./UnsavedChangesDialog.module.css";

/**
 * Asks before unsaved changes are lost (ADR 0015): closing a modified tab,
 * or a window or the app while documents hold changes. Every answer is a
 * command.
 */
export function UnsavedChangesDialog() {
  const { t } = useTranslation();
  const prompt = useAppStore((state) => state.closePrompt);
  if (!prompt) return null;
  const cancel = () => void runCommand("app.cancelClose");

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(open) => {
        if (!open) cancel();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog role="alertdialog" className={styles.dialog}>
          <Heading slot="title" className={styles.title}>
            {t("unsaved.title")}
          </Heading>
          {prompt.kind === "tab" ? (
            <p className={styles.message}>
              {t("unsaved.tab")}
              {/* A file name can be in either script regardless of UI language. */}
              <bdi className={styles.name}>{prompt.name}</bdi>
            </p>
          ) : (
            <p className={styles.message}>
              {t(prompt.kind === "window" ? "unsaved.window" : "unsaved.quit")}
            </p>
          )}
          <div className={styles.actions}>
            {prompt.kind === "tab" && (
              <>
                <Button
                  variant="primary"
                  autoFocus
                  onPress={() =>
                    void runCommand("file.saveAndClose", { documentId: prompt.documentId })
                  }
                >
                  {t("unsaved.save")}
                </Button>
                <Button
                  onPress={() =>
                    void runCommand("file.close", { documentId: prompt.documentId, force: true })
                  }
                >
                  {t("unsaved.discard")}
                </Button>
              </>
            )}
            {prompt.kind === "window" && (
              <Button onPress={() => void runCommand("window.closeDiscarding")}>
                {t("unsaved.closeAnyway")}
              </Button>
            )}
            {prompt.kind === "quit" && (
              <Button onPress={() => void runCommand("app.quitDiscarding")}>
                {t("unsaved.quitAnyway")}
              </Button>
            )}
            <Button variant="ghost" autoFocus={prompt.kind !== "tab"} onPress={cancel}>
              {t("unsaved.cancel")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
