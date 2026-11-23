import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CrashReportInfo } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatTag } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { useCrashReports } from "../../state/crashReports";
import { Button } from "../ui/Button";
import styles from "./CrashReportCard.module.css";

/**
 * One kept crash report (ADR 0061): what kind, when, the start of its
 * message, and the whole text exactly as it would be sent. Copy, save, a
 * GitHub issue, and delete are commands.
 */
export function CrashReportCard({
  report,
  expanded = false,
}: {
  report: CrashReportInfo;
  /** Shows the report's text at first (the prompt after a crash). */
  expanded?: boolean;
}) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const github = useCrashReports((state) => state.github);
  const status = useCrashReports((state) => state.status[report.id]);
  const [open, setOpen] = useState(expanded);
  const textId = useId();
  // Each report has the same buttons: they say which report (its kind and time).
  const headerId = useId();
  const time = new Intl.DateTimeFormat(formatTag(locale), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(report.created * 1000);
  const arg = { reportId: report.id };

  return (
    <li className={styles.card} data-testid="crash-report">
      <div className={styles.header} id={headerId}>
        <span className={styles.kind}>{t(`crash.kind.${report.kind}`)}</span>
        <span className={styles.time}>{time}</span>
      </div>
      {/* The message is technical English, whatever the UI's language. */}
      <p className={styles.summary} dir="ltr" lang="en">
        {report.summary}
      </p>
      <Button
        variant="ghost"
        aria-expanded={open}
        aria-controls={open ? textId : undefined}
        aria-describedby={headerId}
        onPress={() => setOpen(!open)}
      >
        {open ? t("crash.hide") : t("crash.show")}
      </Button>
      {open && (
        <section
          id={textId}
          className={styles.text}
          aria-label={t("crash.reportText")}
          data-testid="crash-report-text"
          // biome-ignore lint/a11y/noNoninteractiveTabindex: it scrolls, so the keyboard must reach it
          tabIndex={0}
        >
          <pre className={styles.pre} dir="ltr" lang="en">
            {report.text}
          </pre>
        </section>
      )}
      <div className={styles.actions}>
        <Button aria-describedby={headerId} onPress={() => void runCommand("crash.copy", arg)}>
          {t("crash.copy")}
        </Button>
        <Button aria-describedby={headerId} onPress={() => void runCommand("crash.save", arg)}>
          {t("crash.save")}
        </Button>
        {github && (
          <Button
            aria-describedby={headerId}
            onPress={() => void runCommand("crash.openIssue", arg)}
          >
            {t("crash.openIssue")}
          </Button>
        )}
        <Button
          variant="ghost"
          aria-describedby={headerId}
          onPress={() => void runCommand("crash.delete", arg)}
        >
          {t("crash.delete")}
        </Button>
      </div>
      <p className={styles.status} role="status">
        {status ? t(`crash.status.${status}`) : ""}
      </p>
    </li>
  );
}
