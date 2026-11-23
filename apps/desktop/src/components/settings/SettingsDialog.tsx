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
import { formatNumber } from "../../i18n";
import { UI_SCALES, type UiScale, useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { CrashReportsSection } from "./CrashReportsSection";
import { OcrModelsSection } from "./OcrModelsSection";
import styles from "./SettingsDialog.module.css";

/**
 * Settings (Cmd/Ctrl+,): the interface size, the Arabic interface's digits,
 * the OCR models (ADR 0034), and crash reports (ADR 0061). Language and theme
 * stay on their own commands.
 */
export function SettingsDialog() {
  const { t } = useTranslation();
  const open = useAppStore((state) => state.settingsOpen);
  const uiDigits = useAppStore((state) => state.uiDigits);
  const uiScale = useAppStore((state) => state.uiScale);
  const setUiScale = useAppStore((state) => state.setUiScale);
  const locale = useAppStore((state) => state.locale);
  const displayHeading = useId();
  const languageHeading = useId();
  const ocrHeading = useId();
  const crashHeading = useId();
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
            <section className={styles.section} aria-labelledby={displayHeading}>
              <h3 id={displayHeading} className={styles.sectionTitle}>
                {t("settings.display")}
              </h3>
              <RadioGroup
                className={styles.group}
                orientation="horizontal"
                value={String(uiScale)}
                onChange={(value) => {
                  const scale = UI_SCALES.find((candidate) => String(candidate) === value);
                  if (scale !== undefined) setUiScale(scale as UiScale);
                }}
              >
                <Label className={styles.legend}>{t("settings.uiScale")}</Label>
                <div className={styles.choices}>
                  {UI_SCALES.map((scale) => (
                    <Radio key={scale} value={String(scale)} className={styles.radio}>
                      <span className={styles.dot} aria-hidden="true" />
                      {t("settings.uiScaleOption", {
                        percent: formatNumber(scale * 100, locale),
                      })}
                    </Radio>
                  ))}
                </div>
                <Text slot="description" className={styles.hint}>
                  {t("settings.uiScaleHint")}
                </Text>
              </RadioGroup>
            </section>
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
            <section className={styles.section} aria-labelledby={crashHeading}>
              <h3 id={crashHeading} className={styles.sectionTitle}>
                {t("settings.crash.heading")}
              </h3>
              <CrashReportsSection />
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
