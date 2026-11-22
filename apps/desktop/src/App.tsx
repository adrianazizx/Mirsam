import { useEffect } from "react";
import { I18nProvider } from "react-aria-components";
import { backend, type DocumentsArrived, isBackendError } from "./backend";
import {
  acceptOutcome,
  commandForEvent,
  isCommandEnabled,
  isEditableTarget,
  refreshOcrModels,
  resolvedTheme,
  runCommand,
} from "./commands/registry";
import { SignatureDialog } from "./components/forms/SignatureDialog";
import { OcrModelPrompt } from "./components/ocr/OcrModelPrompt";
import { OutputDialog } from "./components/output/OutputDialog";
import { SplitDialog } from "./components/pages/SplitDialog";
import { RedactDialog } from "./components/protect/RedactDialog";
import { SecurityDialog } from "./components/protect/SecurityDialog";
import { SettingsDialog } from "./components/settings/SettingsDialog";
import { AppShell } from "./components/shell/AppShell";
import { RecoveryDialog } from "./components/shell/RecoveryDialog";
import { UnsavedChangesDialog } from "./components/shell/UnsavedChangesDialog";
import { applyLocale, formatTag } from "./i18n";
import {
  type AppState,
  selectActiveDocument,
  setSettingsWriter,
  useAppStore,
} from "./state/appStore";
import { useOcrModels } from "./state/ocrModels";
import { getViewer } from "./viewer/registry";

/** Documents that arrived while running open as tabs; the last one is shown. */
function acceptArrivals({ documents, error }: DocumentsArrived) {
  documents.forEach((outcome, index) => {
    acceptOutcome(outcome, { activate: index === documents.length - 1 });
  });
  if (error) useAppStore.getState().setError(error);
}

/**
 * Writes the open tabs (the session restored at the next launch) now and
 * whenever they change. Not debounced: tab changes are rare, a save is one
 * small transaction, and a delay would lose the last change if the app quit
 * right after it.
 */
function saveSessionOnChange(): () => void {
  let last = "";
  const save = (state: AppState) => {
    const ids = state.documents.map((open) => open.id);
    const key = `${ids.join(",")}|${state.activeDocumentId}`;
    if (key === last) return;
    last = key;
    backend
      .saveSession(ids, state.activeDocumentId)
      .catch((error: unknown) => console.warn("could not save the session", error));
  };
  save(useAppStore.getState());
  return useAppStore.subscribe(save);
}

export function App() {
  const locale = useAppStore((state) => state.locale);
  const theme = useAppStore((state) => state.theme);
  const readingTheme = useAppStore((state) => state.readingTheme);

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

  // Reading themes tint pages and thumbnails (CSS keys off the root).
  useEffect(() => {
    document.documentElement.dataset.readingTheme = readingTheme;
  }, [readingTheme]);

  // Global keyboard shortcuts run registry commands. A disabled command leaves
  // the key alone, so Cmd+C still copies in a text field.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const command = commandForEvent(event);
      if (!command || !isCommandEnabled(command.id)) return;
      event.preventDefault();
      void runCommand(command.id);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // A setting changed in another window (language, theme, reading theme).
  useEffect(() => {
    let stop = () => {};
    let live = true;
    backend
      .onSettingChanged((key, value) => useAppStore.getState().applySetting(key, value))
      .then((unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      })
      .catch((error: unknown) => console.warn("cannot follow other windows' settings", error));
    return () => {
      live = false;
      stop();
    };
  }, []);

  // OCR model downloads run in the shell; every window follows their progress
  // (ADR 0034). An outcome reloads the listing, which is the truth.
  useEffect(() => {
    let stop = () => {};
    let live = true;
    backend
      .onOcrModelProgress((event) => {
        useOcrModels.getState().applyEvent(event);
        if (event.kind !== "progress") {
          refreshOcrModels().catch((error: unknown) =>
            console.warn("OCR models unavailable", error),
          );
        }
      })
      .then((unlisten) => {
        if (live) stop = unlisten;
        else unlisten();
      })
      .catch((error: unknown) => console.warn("cannot follow OCR model downloads", error));
    return () => {
      live = false;
      stop();
    };
  }, []);

  // Copy from the Edit menu (or any copy not started by our shortcut) takes
  // the page view's selection, in logical order.
  useEffect(() => {
    const onCopy = (event: ClipboardEvent) => {
      if (event.defaultPrevented || isEditableTarget(event.target)) return;
      const active = selectActiveDocument(useAppStore.getState());
      const text = active?.hasSelection ? getViewer(active.id)?.selectedText() : "";
      if (!text) return;
      event.clipboardData?.setData("text/plain", text);
      event.preventDefault();
    };
    document.addEventListener("copy", onCopy);
    return () => document.removeEventListener("copy", onCopy);
  }, []);

  // Startup: stored settings, then the previous session's tabs and the
  // documents the app was launched with ("Open with", file association). Only
  // then are tab changes written back as the session: an empty window must not
  // overwrite the session it is about to restore.
  //
  // Documents that arrive later (another copy of the app handed them over,
  // ADR 0046) are listened for first: until the shell has given out the
  // startup documents it adds arrivals to them, and afterwards it sends them.
  useEffect(() => {
    setSettingsWriter((key, value) => {
      backend
        .setSetting(key, value)
        .catch((error: unknown) => console.warn("could not save a setting", key, error));
    });
    backend.settings().then(
      (settings) => useAppStore.getState().hydrate(settings),
      (error: unknown) => console.warn("settings unavailable", error),
    );
    let unsubscribe = () => {};
    let stopArrivals = () => {};
    let cancelled = false;
    // A document that arrives while the startup documents are on their way
    // is the one the user just asked for: it stays in front.
    let starting = true;
    let arrivedWhileStarting = false;
    backend
      .onDocumentsArrived((arrived) => {
        if (starting && arrived.documents.length > 0) arrivedWhileStarting = true;
        acceptArrivals(arrived);
      })
      .then(
        (unlisten) => {
          if (cancelled) unlisten();
          else stopArrivals = unlisten;
        },
        (error: unknown) => console.warn("cannot receive documents from elsewhere", error),
      )
      .then(() => backend.startup())
      .then(({ documents, active, recoveries }) => {
        starting = false;
        for (const outcome of documents) acceptOutcome(outcome, { activate: false });
        // Unsaved changes an earlier session left (ADR 0040).
        if (recoveries?.length) useAppStore.getState().setRecoveries(recoveries);
        const store = useAppStore.getState();
        const first = store.documents[0];
        const target = active ?? (store.activeDocumentId === null ? first?.id : undefined);
        if (!arrivedWhileStarting && target !== undefined && target !== null) {
          store.activateDocument(target);
        }
      })
      .catch((error: unknown) => {
        starting = false;
        useAppStore.getState().setError(isBackendError(error) ? error.code : "generic");
      })
      .finally(() => {
        if (!cancelled) unsubscribe = saveSessionOnChange();
      });
    return () => {
      cancelled = true;
      unsubscribe();
      stopArrivals();
    };
  }, []);

  // The shell asks before a window or the app closes over unsaved changes (ADR 0015).
  useEffect(() => {
    const stops: (() => void)[] = [];
    let live = true;
    const keep = (stop: () => void) => {
      if (live) stops.push(stop);
      else stop();
    };
    const ask = (kind: "window" | "quit") => () => useAppStore.getState().setClosePrompt({ kind });
    backend
      .onCloseRequested(ask("window"))
      .then(keep)
      .catch((error: unknown) => console.warn("cannot ask before closing", error));
    backend
      .onQuitRequested(ask("quit"))
      .then(keep)
      .catch((error: unknown) => console.warn("cannot ask before quitting", error));
    return () => {
      live = false;
      for (const stop of stops) stop();
    };
  }, []);

  return (
    <I18nProvider locale={formatTag(locale)}>
      <AppShell />
      <UnsavedChangesDialog />
      <RecoveryDialog />
      <SplitDialog />
      <SignatureDialog />
      <RedactDialog />
      <OutputDialog />
      <SecurityDialog />
      <SettingsDialog />
      <OcrModelPrompt />
    </I18nProvider>
  );
}
