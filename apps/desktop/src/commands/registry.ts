import { backend, isBackendError } from "../backend";
import { type AppState, selectActiveDocument, useAppStore } from "../state/appStore";
import { stepZoom } from "../state/zoom";
import { isMac } from "./platform";

/**
 * Command registry: every user action is a command (docs/SPEC.md section 9).
 * Buttons, keyboard shortcuts, and — from Milestone 1 — the command palette all
 * run the same entries, so an action has one implementation, one title, and one
 * shortcut everywhere.
 */

export interface Shortcut {
  /** `KeyboardEvent.key`, lower-cased for letters. */
  key: string;
  /** Cmd on macOS, Ctrl elsewhere. */
  mod?: boolean;
  shift?: boolean;
}

export interface Command {
  /** Translation key suffix under `commands.`. */
  id: CommandId;
  shortcut?: Shortcut;
  isEnabled?(state: AppState): boolean;
  run(): void | Promise<void>;
}

export type CommandId =
  | "file.open"
  | "file.close"
  | "view.zoomIn"
  | "view.zoomOut"
  | "view.actualSize"
  | "view.fitWidth"
  | "view.nextPage"
  | "view.previousPage"
  | "app.toggleTheme"
  | "app.toggleLanguage"
  | "app.toggleGallery";

const store = () => useAppStore.getState();
const hasDocument = (state: AppState) => selectActiveDocument(state) !== null;

function reportError(error: unknown) {
  const known = ["password_required", "invalid_pdf", "engine_unavailable"];
  const code = isBackendError(error) && known.includes(error.code) ? error.code : "generic";
  console.error("command failed", error);
  store().setError(code);
}

/** The theme currently on screen, with "system" resolved. */
export function resolvedTheme(theme: AppState["theme"]): "light" | "dark" {
  if (theme !== "system") return theme;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

const COMMANDS: Command[] = [
  {
    id: "file.open",
    shortcut: { key: "o", mod: true },
    async run() {
      const info = await backend.openDocumentDialog();
      if (info) store().addDocument(info);
    },
  },
  {
    id: "file.close",
    shortcut: { key: "w", mod: true },
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      store().removeDocument(active.id);
      await backend.closeDocument(active.id);
    },
  },
  {
    id: "view.zoomIn",
    shortcut: { key: "=", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateDocument(active.id, { zoomMode: "custom", zoom: stepZoom(active.zoom, 1) });
      }
    },
  },
  {
    id: "view.zoomOut",
    shortcut: { key: "-", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateDocument(active.id, { zoomMode: "custom", zoom: stepZoom(active.zoom, -1) });
      }
    },
  },
  {
    id: "view.actualSize",
    shortcut: { key: "1", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { zoomMode: "custom", zoom: 1 });
    },
  },
  {
    id: "view.fitWidth",
    shortcut: { key: "2", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { zoomMode: "fit-width" });
    },
  },
  {
    id: "view.nextPage",
    shortcut: { key: "PageDown" },
    isEnabled: (state) => {
      const active = selectActiveDocument(state);
      return active !== null && active.page < active.pageCount - 1;
    },
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { page: active.page + 1, pageSize: null });
    },
  },
  {
    id: "view.previousPage",
    shortcut: { key: "PageUp" },
    isEnabled: (state) => {
      const active = selectActiveDocument(state);
      return active !== null && active.page > 0;
    },
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { page: active.page - 1, pageSize: null });
    },
  },
  {
    id: "app.toggleTheme",
    run() {
      store().setTheme(resolvedTheme(store().theme) === "dark" ? "light" : "dark");
    },
  },
  {
    id: "app.toggleLanguage",
    run() {
      store().setLocale(store().locale === "ar" ? "en" : "ar");
    },
  },
  {
    id: "app.toggleGallery",
    shortcut: { key: "g", mod: true, shift: true },
    run() {
      store().setGalleryOpen(!store().galleryOpen);
    },
  },
];

const BY_ID = new Map(COMMANDS.map((command) => [command.id, command]));

export function getCommand(id: CommandId): Command {
  const command = BY_ID.get(id);
  if (!command) throw new Error(`unknown command: ${id}`);
  return command;
}

export function isCommandEnabled(id: CommandId, state: AppState = store()): boolean {
  return getCommand(id).isEnabled?.(state) ?? true;
}

/** Runs a command if it is enabled. Failures surface as a localized error banner. */
export async function runCommand(id: CommandId): Promise<void> {
  if (!isCommandEnabled(id)) return;
  try {
    await getCommand(id).run();
  } catch (error) {
    reportError(error);
  }
}

export function matchesShortcut(shortcut: Shortcut, event: KeyboardEvent, mac = isMac): boolean {
  const mod = mac ? event.metaKey : event.ctrlKey;
  const otherMod = mac ? event.ctrlKey : event.metaKey;
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  // "+" is Shift+"=" on most layouts; accept either for zoom in.
  const keyMatches = key === shortcut.key || (shortcut.key === "=" && key === "+");
  const shiftMatches = shortcut.key === "=" || event.shiftKey === Boolean(shortcut.shift);
  return keyMatches && mod === Boolean(shortcut.mod) && !otherMod && !event.altKey && shiftMatches;
}

/** The command bound to this key event, if any. */
export function commandForEvent(event: KeyboardEvent, mac = isMac): Command | undefined {
  return COMMANDS.find(
    (command) => command.shortcut && matchesShortcut(command.shortcut, event, mac),
  );
}

/** Human-readable shortcut, e.g. "⌘O" on macOS and "Ctrl+O" elsewhere. */
export function shortcutLabel(id: CommandId, mac = isMac): string | null {
  const shortcut = getCommand(id).shortcut;
  if (!shortcut) return null;
  const names: Record<string, string> = { "=": "+", PageDown: "PgDn", PageUp: "PgUp" };
  const key = names[shortcut.key] ?? shortcut.key.toUpperCase();
  if (mac) return `${shortcut.shift ? "⇧" : ""}${shortcut.mod ? "⌘" : ""}${key}`;
  return [shortcut.mod ? "Ctrl" : null, shortcut.shift ? "Shift" : null, key]
    .filter(Boolean)
    .join("+");
}
