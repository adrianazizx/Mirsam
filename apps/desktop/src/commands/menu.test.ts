import { act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import menuRs from "../../src-tauri/src/menu.rs?raw";
import { backend } from "../backend";
import { i18n } from "../i18n";
import { useAppStore } from "../state/appStore";
import {
  MENU_COMMANDS,
  MENU_TITLES,
  menuAccelerator,
  menuText,
  runMenuCommand,
  syncMenuOnChange,
} from "./menu";
import { getCommand, shortcutOf } from "./registry";

const app = () => useAppStore.getState();
const item = (id: string) => menuText(app()).items.find((candidate) => candidate.id === id);

beforeEach(() => {
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    closePrompt: null,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("the macOS menu bar (ADR 0074)", () => {
  it("runs the same commands the shell's menu has, in its order", () => {
    const block = /pub const COMMANDS: \[&str; \d+\] = \[([^\]]*)\]/.exec(menuRs)?.[1] ?? "";
    expect([...block.matchAll(/"([^"]+)"/g)].map((match) => match[1])).toEqual([...MENU_COMMANDS]);
  });

  it("shows each command's Mac shortcut", () => {
    const accelerator = (id: (typeof MENU_COMMANDS)[number]) =>
      menuAccelerator(shortcutOf(getCommand(id), true));
    expect(accelerator("file.save")).toBe("CmdOrCtrl+S");
    expect(accelerator("file.saveAs")).toBe("CmdOrCtrl+Shift+S");
    expect(accelerator("file.print")).toBe("CmdOrCtrl+P");
    expect(accelerator("file.close")).toBe("CmdOrCtrl+W");
    expect(accelerator("app.settings")).toBe("CmdOrCtrl+,");
    expect(accelerator("help.guide")).toBeNull();
    expect(menuAccelerator({ key: "Tab", ctrl: true, shift: true })).toBe("Ctrl+Shift+Tab");
    expect(menuAccelerator({ key: "=", mod: true, alt: true })).toBe("CmdOrCtrl+Alt+=");
  });

  it("names every submenu and system item in both languages", () => {
    for (const locale of ["en", "ar"] as const) {
      for (const key of MENU_TITLES) {
        expect(i18n.exists(`menu.${key}`, { lng: locale }), `${locale} menu.${key}`).toBe(true);
      }
      for (const id of [...MENU_COMMANDS, "window.close"]) {
        expect(i18n.exists(`commands.${id}`, { lng: locale }), `${locale} ${id}`).toBe(true);
      }
    }
  });

  it("greys out Save, Save as and Print until a document is open, and Save until it changes", () => {
    expect(item("file.save")?.enabled).toBe(false);
    expect(item("file.saveAs")?.enabled).toBe(false);
    expect(item("file.print")?.enabled).toBe(false);
    expect(item("file.open")).toMatchObject({ title: "Open PDF…", enabled: true });

    act(() => app().addDocument({ id: 1, name: "a.pdf", pageCount: 2 }));
    expect(item("file.save")?.enabled).toBe(false);
    expect(item("file.saveAs")).toMatchObject({ title: "Save as…", enabled: true });
    expect(item("file.print")).toMatchObject({ title: "Print…", enabled: true });

    act(() => app().addDocument({ id: 2, name: "b.pdf", pageCount: 2, modified: true }));
    expect(item("file.save")).toMatchObject({
      title: "Save",
      enabled: true,
      accelerator: "CmdOrCtrl+S",
    });
  });

  it("offers Close for the document in front, or for the window once it shows none", () => {
    expect(item("file.close")).toMatchObject({ title: "Close window", enabled: true });
    act(() => app().addDocument({ id: 1, name: "a.pdf", pageCount: 2 }));
    expect(item("file.close")).toMatchObject({ title: "Close document", enabled: true });
  });

  it("follows the interface's language", () => {
    act(() => app().setLocale("ar"));
    const text = menuText(app());
    expect(text.titles.file).toBe("ملف");
    expect(text.titles.quit).toBe("إنهاء مرسام");
    expect(item("file.save")?.title).toBe("حفظ");
    expect(item("file.saveAs")?.title).toBe("حفظ باسم…");
    expect(item("file.print")?.title).toBe("طباعة…");
    expect(item("file.close")?.title).toBe("إغلاق النافذة");
  });

  it("sends the menu to the shell when what it shows changes, and only then", () => {
    const setMenu = vi.spyOn(backend, "setMenu");
    const stop = syncMenuOnChange();
    expect(setMenu).toHaveBeenCalledTimes(1);

    act(() => app().addDocument({ id: 1, name: "a.pdf", pageCount: 2 }));
    expect(setMenu).toHaveBeenCalledTimes(2);
    expect(setMenu.mock.lastCall?.[0].items.find((i) => i.id === "file.print")?.enabled).toBe(true);

    // Nothing the menu shows: no message.
    act(() => app().setError("generic"));
    expect(setMenu).toHaveBeenCalledTimes(2);

    act(() => app().setLocale("ar"));
    expect(setMenu).toHaveBeenCalledTimes(3);
    expect(setMenu.mock.lastCall?.[0].titles.file).toBe("ملف");

    stop();
    act(() => app().setLocale("en"));
    expect(setMenu).toHaveBeenCalledTimes(3);
  });

  it("runs Save, Save as and Print on the document in front", async () => {
    const save = vi.spyOn(backend, "saveDocument");
    const saveAs = vi.spyOn(backend, "saveDocumentAs").mockResolvedValue(null);
    const print = vi.spyOn(backend, "printDocument");
    act(() => app().addDocument({ id: 7, name: "a.pdf", pageCount: 2, modified: true }));

    await act(() => runMenuCommand("file.save"));
    expect(save).toHaveBeenCalledWith(7);
    await act(() => runMenuCommand("file.saveAs"));
    expect(saveAs).toHaveBeenCalledWith(7);
    await act(() => runMenuCommand("file.print"));
    expect(print).toHaveBeenCalledWith(7);
  });

  it("closes the document in front, then the window", async () => {
    const closeDocument = vi.spyOn(backend, "closeDocument");
    const closeWindow = vi.spyOn(backend, "closeWindow");
    act(() => app().addDocument({ id: 3, name: "a.pdf", pageCount: 2 }));

    await act(() => runMenuCommand("file.close"));
    expect(closeDocument).toHaveBeenCalledWith(3);
    expect(closeWindow).not.toHaveBeenCalled();

    await act(() => runMenuCommand("file.close"));
    expect(closeWindow).toHaveBeenCalledTimes(1);
  });

  it("ignores anything but its own commands, and commands that cannot run", async () => {
    const closeWindow = vi.spyOn(backend, "closeWindow");
    const print = vi.spyOn(backend, "printDocument");
    await act(() => runMenuCommand("window.close"));
    await act(() => runMenuCommand("not.a.command"));
    await act(() => runMenuCommand("file.print"));
    expect(closeWindow).not.toHaveBeenCalled();
    expect(print).not.toHaveBeenCalled();
  });
});
