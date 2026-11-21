import { useEffect } from "react";
import { I18nProvider } from "react-aria-components";
import { backend, isBackendError } from "./backend";
import { commandForEvent, resolvedTheme, runCommand } from "./commands/registry";
import { AppShell } from "./components/shell/AppShell";
import { applyLocale, formatTag } from "./i18n";
import { useAppStore } from "./state/appStore";

export function App() {
  const locale = useAppStore((state) => state.locale);
  const theme = useAppStore((state) => state.theme);

  // Language and direction live on the root element so every component, portal
  // (tooltips), and CSS logical property follows them.
  useEffect(() => {
    applyLocale(locale);
  }, [locale]);

  // Resolve "system" to a concrete theme and follow OS changes.
  useEffect(() => {
    const apply = () => {
      document.documentElement.dataset.theme = resolvedTheme(theme);
    };
    apply();
    if (theme !== "system") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  // Global keyboard shortcuts run registry commands.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const command = commandForEvent(event);
      if (!command) return;
      event.preventDefault();
      void runCommand(command.id);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Open the document the app was launched with ("Open with", file association).
  useEffect(() => {
    backend
      .takeStartupDocument()
      .then((info) => {
        if (info) useAppStore.getState().addDocument(info);
      })
      .catch((error: unknown) => {
        useAppStore.getState().setError(isBackendError(error) ? error.code : "generic");
      });
  }, []);

  return (
    <I18nProvider locale={formatTag(locale)}>
      <AppShell />
    </I18nProvider>
  );
}
