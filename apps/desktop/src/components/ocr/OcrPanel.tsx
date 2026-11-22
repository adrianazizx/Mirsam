import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { isCommandEnabled, refreshOcrModels, runCommand } from "../../commands/registry";
import { formatBytes } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { bytesToDownload, missingRequired, useOcrModels } from "../../state/ocrModels";
import { Button } from "../ui/Button";
import styles from "./OcrPanel.module.css";

/**
 * The OCR tool's panel in the inspector: whether the model is ready, the way
 * to recognize text, and the way to the models in Settings.
 */
export function OcrPanel() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const canRecognize = useAppStore((state) => isCommandEnabled("ocr.recognize", state));
  const models = useOcrModels((state) => state.models);
  const jobs = useOcrModels((state) => state.jobs);

  useEffect(() => {
    if (useOcrModels.getState().models) return;
    refreshOcrModels().catch((error: unknown) => console.warn("OCR models unavailable", error));
  }, []);

  const missing = missingRequired(models);
  const job = missing.map((pkg) => jobs[pkg.id]).find((running) => running !== undefined);
  const size = (bytes: number) => formatBytes(bytes, locale);
  let status: string;
  if (!models) status = t("ocrModels.loading");
  else if (!models.available) status = t("ocrModels.unavailable");
  else if (job)
    status = t(`ocrModels.status.${job.phase}`, {
      received: size(job.received),
      total: size(job.total),
    });
  else if (missing.length > 0)
    status = t("ocrPanel.missing", { size: size(bytesToDownload(missing)) });
  else status = t("ocrPanel.ready");

  return (
    <div className={styles.panel} data-testid="ocr-panel">
      <p className={styles.status} role="status">
        {status}
      </p>
      <div className={styles.actions}>
        <Button
          variant="primary"
          isDisabled={!canRecognize}
          onPress={() => void runCommand("ocr.recognize")}
        >
          {t("ocrPanel.recognize")}
        </Button>
        <Button onPress={() => void runCommand("app.settings")}>{t("ocrPanel.manage")}</Button>
      </div>
    </div>
  );
}
