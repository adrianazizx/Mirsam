import { useId } from "react";
import {
  Dialog,
  Heading,
  Label,
  Modal,
  ModalOverlay,
  Radio,
  RadioGroup,
  Text,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { OcrModelsSection } from "./OcrModelsSection";
import styles from "./SettingsDialog.module.css";

/**
 * Settings (Cmd/Ctrl+,): the Arabic interface's digits, and the OCR models
 * (ADR 0034). Language and theme stay on their own commands.
 */
export function SettingsDialog() {
  const { t } = useTranslation();
  const open = useAppStore((state) => state.settingsOpen);
  const uiDigits = useAppStore((state) => state.uiDigits);
  const languageHeading = useId();
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
          <div className={styles.body}>
            <section className={styles.section} aria-labelledby={languageHeading}>
              <h3 id={languageHeading} className={styles.sectionTitle}>
                {t("settings.language")}
              </h3>
              <RadioGroup
                className={styles.group}
                value={uiDigits}
                onChange={(value) =>
                  void runCommand(
                    value === "western" ? "app.digitsWestern" : "app.digitsArabicIndic",
                  )
                }
              >
                <Label className={styles.legend}>{t("settings.digits")}</Label>
                <Radio value="arabicIndic" className={styles.radio}>
                  <span className={styles.dot} aria-hidden="true" />
                  {t("settings.digitsArabicIndic")}
                </Radio>
                <Radio value="western" className={styles.radio}>
                  <span className={styles.dot} aria-hidden="true" />
                  {t("settings.digitsWestern")}
                </Radio>
                <Text slot="description" className={styles.hint}>
                  {t("settings.digitsHint")}
                </Text>
              </RadioGroup>
            </section>
            <section className={styles.section} aria-labelledby={ocrHeading}>
              <h3 id={ocrHeading} className={styles.sectionTitle}>
                {t("ocrModels.heading")}
              </h3>
              <OcrModelsSection />
            </section>
          </div>
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
