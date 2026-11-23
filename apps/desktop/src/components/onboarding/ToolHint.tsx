import { Lightbulb } from "lucide-react";
import { useEffect, useId } from "react";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { announce } from "../../state/announcer";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { hintDue, useOnboarding } from "../../state/onboarding";
import { Button } from "../ui/Button";
import styles from "./ToolHint.module.css";

/**
 * A tool's one-time hint (M7): the first time OCR, Sign or Edit is opened with
 * a document on screen, a short note says what the tool does. It floats over
 * the page area's inline-end corner, near the tool's panel, so the page does
 * not move; it never takes focus, and is announced politely instead. It goes
 * on "Got it" or when another tool is chosen, and is not shown again.
 */
export function ToolHint() {
  const { t } = useTranslation();
  const tool = useAppStore((state) => state.activeTool);
  const hasDocument = useAppStore((state) => selectActiveDocument(state) !== null);
  const loaded = useOnboarding((state) => state.loaded);
  const guideOpen = useOnboarding((state) => state.guideOpen);
  const hintsSeen = useOnboarding((state) => state.hintsSeen);
  const hint = useOnboarding((state) => state.hint);
  const titleId = useId();

  // A tool opened for the first time shows its hint; another tool hides it.
  useEffect(() => {
    const onboarding = useOnboarding.getState();
    if (onboarding.hint !== null && onboarding.hint !== tool) onboarding.dismissHint();
    const due = hintDue(tool, hasDocument, { loaded, guideOpen, hintsSeen });
    if (due === null) return;
    onboarding.showHint(due);
    announce(`${t(`onboarding.hints.${due}.title`)}. ${t(`onboarding.hints.${due}.body`)}`);
  }, [tool, hasDocument, loaded, guideOpen, hintsSeen, t]);

  if (hint === null || hint !== tool || !hasDocument) return null;
  return (
    <section className={styles.hint} aria-labelledby={titleId} data-testid="tool-hint">
      <Lightbulb className={styles.icon} aria-hidden="true" />
      <div className={styles.content}>
        <h2 id={titleId} className={styles.title}>
          {t(`onboarding.hints.${hint}.title`)}
        </h2>
        <p className={styles.body}>{t(`onboarding.hints.${hint}.body`)}</p>
        <div className={styles.actions}>
          <Button variant="primary" onPress={() => void runCommand("onboarding.dismissHint")}>
            {t("onboarding.gotIt")}
          </Button>
          {hint === "ocr" ? (
            <Button onPress={() => void runCommand("app.settings")}>
              {t("onboarding.hints.ocr.models")}
            </Button>
          ) : null}
        </div>
      </div>
    </section>
  );
}
