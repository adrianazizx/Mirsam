import { Component, type ErrorInfo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { reportUiError } from "../../state/crashReports";
import { Button } from "../ui/Button";
import styles from "./CrashBoundary.module.css";

interface State {
  failed: boolean;
}

/**
 * The last line of defence for the window (ADR 0061): an error while
 * rendering keeps a crash report and shows a way back (reload the window)
 * instead of a blank window. The shell holds the documents, so they come
 * back with the reload.
 */
export class CrashBoundary extends Component<{ children: ReactNode }, State> {
  override state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  override componentDidCatch(error: unknown, info: ErrorInfo) {
    // The component stack names components (code), never data.
    void reportUiError(error, info.componentStack ?? "");
  }

  override render() {
    return this.state.failed ? <CrashFallback /> : this.props.children;
  }
}

function CrashFallback() {
  const { t, i18n } = useTranslation();
  // Its own direction: the error may have come before the app set the root's.
  return (
    <main
      className={styles.fallback}
      data-testid="crash-fallback"
      lang={i18n.language}
      dir={i18n.dir(i18n.language)}
    >
      <h1 className={styles.title}>{t("crashFallback.title")}</h1>
      <p className={styles.message}>{t("crashFallback.message")}</p>
      <p className={styles.message}>{t("crashFallback.reported")}</p>
      <Button variant="primary" autoFocus onPress={() => void runCommand("app.reloadWindow")}>
        {t("crashFallback.reload")}
      </Button>
    </main>
  );
}
