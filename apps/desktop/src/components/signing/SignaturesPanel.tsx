import { RefreshCw, ShieldAlert, ShieldCheck, ShieldX } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { SignatureChanges, SignatureReport } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatNumber, formatTag, type Locale } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { overallStatus, useCertSign } from "../../state/certSign";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import styles from "./SignaturesPanel.module.css";

const ICONS = { valid: ShieldCheck, unknown: ShieldAlert, invalid: ShieldX } as const;

/**
 * A document's digital signatures (ADR 0047), checked when the panel shows:
 * for each, the verdict and why, who signed (names in `<bdi>`: they may be
 * Arabic), when (a trusted time-stamp, or the signer's own clock), and what
 * the file changed after it. Revocation is not checked yet, and every
 * verdict says so.
 */
export function SignaturesPanel({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const reports = useCertSign((state) => state.reports[documentId]);
  // A document changes (an edit, a save) and is checked again when shown.
  const paint = useAppStore((state) => state.documents.find((d) => d.id === documentId)?.paint);
  const revision = useAppStore(
    (state) => state.documents.find((d) => d.id === documentId)?.revision,
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: checks again on every change of the document
  useEffect(() => {
    void runCommand("signatures.check");
  }, [documentId, paint, revision]);

  const list = reports?.list ?? [];
  const overall = overallStatus(list);
  return (
    <div className={styles.panel} data-testid="signatures-panel">
      <div className={styles.controls}>
        <p className={styles.summary} role="status">
          {reports === undefined || (reports.status === "loading" && list.length === 0)
            ? t("signaturesPanel.checking")
            : reports.status === "error"
              ? t("signaturesPanel.failed")
              : overall === null
                ? t("signaturesPanel.none")
                : t(`signaturesPanel.overall.${overall}`, {
                    formatted: formatNumber(list.length, locale),
                  })}
        </p>
        <CommandButton command="signatures.check">
          <RefreshCw />
        </CommandButton>
      </div>
      <ol className={styles.list}>
        {list.map((report, index) => (
          <SignatureCard
            // Reports have no id; their order (oldest first) is stable for a revision.
            // biome-ignore lint/suspicious/noArrayIndexKey: see above
            key={index}
            report={report}
            index={index}
            locale={locale}
          />
        ))}
      </ol>
      {list.length > 0 ? <p className={styles.note}>{t("signaturesPanel.revocation")}</p> : null}
    </div>
  );
}

function SignatureCard({
  report,
  index,
  locale,
}: {
  report: SignatureReport;
  index: number;
  locale: Locale;
}) {
  const { t } = useTranslation();
  const Icon = ICONS[report.status];
  const date = (seconds: number) =>
    new Intl.DateTimeFormat(formatTag(locale), { dateStyle: "medium", timeStyle: "short" }).format(
      new Date(seconds * 1000),
    );
  const signer = report.signer?.name ?? report.name;
  const headingId = `signature-${index}`;
  return (
    <li className={styles.card} aria-labelledby={headingId} data-status={report.status}>
      <div className={styles.verdict}>
        <Icon aria-hidden="true" className={styles.icon} />
        <span id={headingId}>
          {t(
            report.documentTimestamp
              ? `signaturesPanel.timestampStatus.${report.status}`
              : `signaturesPanel.status.${report.status}`,
          )}
        </span>
      </div>
      <dl className={styles.facts}>
        <dt>
          {t(report.documentTimestamp ? "signaturesPanel.authority" : "signaturesPanel.signer")}
        </dt>
        <dd>{signer ? <bdi>{signer}</bdi> : t("signaturesPanel.unknownSigner")}</dd>
        {report.signer && !report.documentTimestamp ? (
          <>
            <dt>{t("signaturesPanel.issuer")}</dt>
            <dd>
              {report.signer.selfSigned ? (
                t("signaturesPanel.selfSigned")
              ) : (
                <bdi>{report.signer.issuer}</bdi>
              )}
            </dd>
          </>
        ) : null}
        {report.timestamp ? (
          <>
            <dt>{t("signaturesPanel.timestamped")}</dt>
            <dd>
              {date(report.timestamp.time)}
              {report.timestamp.authority ? (
                <>
                  {" — "}
                  <bdi>{report.timestamp.authority}</bdi>
                </>
              ) : null}
            </dd>
          </>
        ) : report.claimedTime !== null ? (
          <>
            <dt>{t("signaturesPanel.signedAt")}</dt>
            <dd>
              {date(report.claimedTime)}
              <span className={styles.aside}>{t("signaturesPanel.signerClock")}</span>
            </dd>
          </>
        ) : null}
        {report.reason ? (
          <>
            <dt>{t("signaturesPanel.reason")}</dt>
            <dd>
              <bdi>{report.reason}</bdi>
            </dd>
          </>
        ) : null}
        {report.location ? (
          <>
            <dt>{t("signaturesPanel.location")}</dt>
            <dd>
              <bdi>{report.location}</bdi>
            </dd>
          </>
        ) : null}
        <dt>{t("signaturesPanel.revision")}</dt>
        <dd>
          {t("signaturesPanel.revisionOf", {
            revision: formatNumber(report.revision, locale),
            revisions: formatNumber(report.revisions, locale),
          })}
        </dd>
      </dl>
      {report.problems.length > 0 ? (
        <ul className={styles.problems}>
          {report.problems.map((problem) => (
            <li key={problem}>{t(`signaturesPanel.problem.${problem}`)}</li>
          ))}
        </ul>
      ) : null}
      {report.coversWholeFile ? (
        <p className={styles.unchanged}>{t("signaturesPanel.unchanged")}</p>
      ) : report.changes ? (
        <Changes changes={report.changes} locale={locale} />
      ) : null}
      {report.page !== null && report.view !== null ? (
        <div className={styles.row}>
          <Button
            variant="ghost"
            onPress={() => void runCommand("signatures.reveal", { page: report.page ?? 0 })}
          >
            {t("signaturesPanel.show", { page: formatNumber((report.page ?? 0) + 1, locale) })}
          </Button>
        </div>
      ) : null}
    </li>
  );
}

function Changes({ changes, locale }: { changes: SignatureChanges; locale: Locale }) {
  const { t } = useTranslation();
  const n = (value: number) => formatNumber(value, locale);
  const items: string[] = [];
  if (changes.signatures > 0) {
    items.push(t("signaturesPanel.changed.signatures", { formatted: n(changes.signatures) }));
  }
  if (changes.annotations > 0) {
    items.push(t("signaturesPanel.changed.annotations", { formatted: n(changes.annotations) }));
  }
  if (changes.formFields > 0) {
    items.push(t("signaturesPanel.changed.formFields", { formatted: n(changes.formFields) }));
  }
  if (changes.pageOrder) items.push(t("signaturesPanel.changed.pageOrder"));
  if (changes.pages.length > 0) {
    items.push(
      t("signaturesPanel.changed.pages", {
        pages: changes.pages.map((page) => n(page + 1)).join(t("signaturesPanel.listSeparator")),
      }),
    );
  }
  if (changes.other > 0) items.push(t("signaturesPanel.changed.other"));
  return (
    <div className={styles.changes}>
      <p className={styles.changesTitle}>
        {t("signaturesPanel.changedAfter", { formatted: n(changes.revisions) })}
      </p>
      {items.length > 0 ? (
        <ul className={styles.problems}>
          {items.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
