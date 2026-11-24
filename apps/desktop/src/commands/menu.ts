import { backend, type MenuText } from "../backend";
import { i18n } from "../i18n";
import { type AppState, useAppStore } from "../state/appStore";
import {
  type CommandId,
  getCommand,
  isCommandEnabled,
  runCommand,
  type Shortcut,
  shortcutOf,
} from "./registry";

/**
 * The macOS menu bar (ADR 0074). The Rust shell builds it; this window says
 * what it shows (titles in the interface's language, shortcuts, what can run
 * now) and runs the registry command an item asks for.
 */

/** The registry commands the menu bar runs, in the shell's order (`menu.rs` `COMMANDS`). */
export const MENU_COMMANDS = [
  "app.settings",
  "window.new",
  "file.open",
  "file.close",
  "file.save",
  "file.saveAs",
  "file.print",
  "help.guide",
  "app.shortcuts",
] as const satisfies readonly CommandId[];

export type MenuCommandId = (typeof MENU_COMMANDS)[number];

/** Titles of the submenus and of the system's own items, under `menu.` in the locales. */
export const MENU_TITLES = [
  "file",
  "edit",
  "view",
  "window",
  "help",
  "about",
  "services",
  "hide",
  "hideOthers",
  "showAll",
  "quit",
  "undo",
  "redo",
  "cut",
  "copy",
  "paste",
  "selectAll",
  "fullScreen",
  "minimize",
  "zoom",
] as const;

/** A shortcut as a menu accelerator: "CmdOrCtrl+Shift+S". */
export function menuAccelerator(shortcut: Shortcut | undefined): string | null {
  if (!shortcut) return null;
  const key = shortcut.key.length === 1 ? shortcut.key.toUpperCase() : shortcut.key;
  return [
    shortcut.mod ? "CmdOrCtrl" : null,
    shortcut.ctrl ? "Ctrl" : null,
    shortcut.alt ? "Alt" : null,
    shortcut.shift ? "Shift" : null,
    key,
  ]
    .filter(Boolean)
    .join("+");
}

/**
 * Close is always there: it closes the document in front, or the window once
 * it shows none (as the system's Close Window did), so ⌘W still closes an
 * empty window.
 */
function closesDocument(state: AppState): boolean {
  return isCommandEnabled("file.close", state);
}

/** The menu bar as this window shows it now. */
export function menuText(state: AppState): MenuText {
  const t = (key: string) => i18n.t(key, { lng: state.locale });
  return {
    titles: Object.fromEntries(MENU_TITLES.map((key) => [key, t(`menu.${key}`)])),
    items: MENU_COMMANDS.map((id) => {
      const closesWindow = id === "file.close" && !closesDocument(state);
      return {
        id,
        title: t(`commands.${closesWindow ? "window.close" : id}`),
        enabled: id === "file.close" || isCommandEnabled(id, state),
        // The menu bar is macOS's: its shortcuts are the Mac ones.
        accelerator: menuAccelerator(shortcutOf(getCommand(id), true)),
      };
    }),
  };
}

/** What decides the menu: rebuilt and sent only when this changes. */
function menuKey(state: AppState): string {
  const enabled = MENU_COMMANDS.map((id) => (isCommandEnabled(id, state) ? 1 : 0)).join("");
  return `${state.locale}|${enabled}`;
}

/** Keeps the shell's menu bar in step with this window. Returns a function that stops. */
export function syncMenuOnChange(): () => void {
  let last = "";
  const send = (state: AppState) => {
    const key = menuKey(state);
    if (key === last) return;
    last = key;
    backend
      .setMenu(menuText(state))
      .catch((error: unknown) => console.warn("could not update the menu bar", error));
  };
  send(useAppStore.getState());
  return useAppStore.subscribe(send);
}

/** Runs the command a menu bar item asks for, as its shortcut would. */
export function runMenuCommand(id: string): Promise<void> {
  const command = MENU_COMMANDS.find((known) => known === id);
  if (!command) return Promise.resolve();
  if (command === "file.close" && !closesDocument(useAppStore.getState())) {
    return runCommand("window.close");
  }
  return runCommand(command);
}
