import { ShieldAlert, ShieldCheck, ShieldEllipsis, ShieldX } from "lucide-react";
import { useEffect } from "react";
import { Button as AriaButton } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { overallStatus, useCertSign } from "../../state/certSign";
import { Tooltip } from "../ui/Tooltip";
import styles from "./SignatureBadge.module.css";

const ICONS = {
  valid: ShieldCheck,
  unknown: ShieldAlert,
  invalid: ShieldX,
  checking: ShieldEllipsis,
} as const;

/**
 * A signed document's verdict in the status bar (ADR 0048). Its signatures
 * are checked as soon as it is the active document, so a reader sees at once
 * whether they hold, without opening the panel; pressing it opens the panel.
 */
export function SignatureBadge() {
  const { t } = useTranslation();
  const document = useAppStore(selectActiveDocument);
  const id = document?.id ?? null;
  const signed = document?.signed ?? false;
  const reports = useCertSign((state) => (id === null ? undefined : state.reports[id]));

  // Once per signed document; the panel checks again after changes.
  useEffect(() => {
    if (id !== null && signed && !useCertSign.getState().reports[id]) {
      void runCommand("signatures.check");
    }
  }, [id, signed]);

  if (id === null || !signed) return null;
  const status =
    reports === undefined || reports.status === "loading"
      ? "checking"
      : (overallStatus(reports.list) ?? "checking");
  if (reports?.status === "error" || (reports?.status === "done" && reports.list.length === 0)) {
    return null;
  }
  const Icon = ICONS[status];
  const label = t(`signaturesPanel.badge.${status}`);
  return (
    <Tooltip label={t("commands.view.showSignatures")} placement="top">
      <AriaButton
        className={styles.badge}
        data-status={status}
        data-testid="signature-badge"
        onPress={() => void runCommand("view.showSignatures")}
      >
        <Icon aria-hidden="true" className={styles.icon} />
        <span>{label}</span>
      </AriaButton>
    </Tooltip>
  );
}
