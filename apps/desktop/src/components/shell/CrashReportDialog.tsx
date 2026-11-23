import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { useCrashReports } from "../../state/crashReports";
import { Button } from "../ui/Button";
import { CrashReportCard } from "./CrashReportCard";
import styles from "./CrashReportDialog.module.css";

/**
 * Asks, once after a crash, whether to send its report (ADR 0061). Each
 * report is shown exactly as it would be sent, and can be copied, saved,
 * filed as a GitHub issue, or deleted. "Not now" keeps the reports in
 * Settings; "Don't keep crash reports" turns them off.
 */
export function CrashReportDialog() {
  const { t } = useTranslation();
  const reports = useCrashReports((state) => state.prompt);
  if (reports.length === 0) return null;
  const notNow = () => void runCommand("crash.dismiss");

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(open) => {
        if (!open) notNow();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="crash-reports">
          <Heading slot="title" className={styles.title}>
            {t("crash.title")}
          </Heading>
          <p className={styles.message}>{t("crash.message")}</p>
          <p className={styles.message}>{t("crash.nothingSent")}</p>
          <ul className={styles.list}>
            {reports.map((report, index) => (
              <CrashReportCard key={report.id} report={report} expanded={index === 0} />
            ))}
          </ul>
          <p className={styles.message}>{t("crash.notNowHint")}</p>
          <div className={styles.footer}>
            <Button variant="ghost" onPress={() => void runCommand("crash.off")}>
              {t("crash.turnOff")}
            </Button>
            <Button variant="primary" autoFocus onPress={notNow}>
              {t("crash.notNow")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
