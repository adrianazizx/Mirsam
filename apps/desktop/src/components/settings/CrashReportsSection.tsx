import { useEffect } from "react";
import { Label, Radio, RadioGroup, Text } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { refreshCrashReports, useCrashReports } from "../../state/crashReports";
import { CrashReportCard } from "../shell/CrashReportCard";
import styles from "./SettingsDialog.module.css";

/**
 * Settings → Crash reports (ADR 0061): ask after a crash (the default) or
 * keep none, and every report kept, each shown as it would be sent.
 */
export function CrashReportsSection() {
  const { t } = useTranslation();
  const setting = useCrashReports((state) => state.setting);
  const kept = useCrashReports((state) => state.kept);

  useEffect(() => {
    refreshCrashReports().catch((error: unknown) =>
      console.warn("crash reports unavailable", error),
    );
  }, []);

  return (
    <>
      <RadioGroup
        className={styles.group}
        value={setting}
        onChange={(value) => void runCommand(value === "off" ? "crash.off" : "crash.ask")}
      >
        <Label className={styles.legend}>{t("settings.crash.choice")}</Label>
        <Radio value="ask" className={styles.radio}>
          <span className={styles.dot} aria-hidden="true" />
          {t("settings.crash.ask")}
        </Radio>
        <Radio value="off" className={styles.radio}>
          <span className={styles.dot} aria-hidden="true" />
          {t("settings.crash.off")}
        </Radio>
        <Text slot="description" className={styles.hint}>
          {setting === "off" ? t("settings.crash.offHint") : t("settings.crash.askHint")}
        </Text>
      </RadioGroup>
      {kept.length === 0 ? (
        <p className={styles.hint}>{t("settings.crash.none")}</p>
      ) : (
        <ul className={styles.reports} aria-label={t("settings.crash.kept")}>
          {kept.map((report) => (
            <CrashReportCard key={report.id} report={report} />
          ))}
        </ul>
      )}
    </>
  );
}
