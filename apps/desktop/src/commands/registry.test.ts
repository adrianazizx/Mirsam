import { beforeEach, describe, expect, it } from "vitest";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { commandForEvent, isCommandEnabled, runCommand, shortcutLabel } from "./registry";

const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

beforeEach(() => {
  useAppStore.setState({ documents: [], activeDocumentId: null, errorCode: null });
});

describe("command registry", () => {
  it("resolves shortcuts per platform", () => {
    expect(commandForEvent(key({ key: "o", metaKey: true }), true)?.id).toBe("file.open");
    expect(commandForEvent(key({ key: "o", ctrlKey: true }), false)?.id).toBe("file.open");
    // Ctrl+O is not the shortcut on macOS, and a bare "o" is never one.
    expect(commandForEvent(key({ key: "o", ctrlKey: true }), true)).toBeUndefined();
    expect(commandForEvent(key({ key: "o" }), false)).toBeUndefined();
    expect(commandForEvent(key({ key: "+", ctrlKey: true, shiftKey: true }), false)?.id).toBe(
      "view.zoomIn",
    );
    expect(commandForEvent(key({ key: "PageDown" }), false)?.id).toBe("view.nextPage");
  });

  it("labels shortcuts for each platform", () => {
    expect(shortcutLabel("file.open", true)).toBe("⌘O");
    expect(shortcutLabel("file.open", false)).toBe("Ctrl+O");
    expect(shortcutLabel("view.zoomIn", false)).toBe("Ctrl++");
    expect(shortcutLabel("app.toggleTheme")).toBeNull();
  });

  it("disables document commands until a document is open", async () => {
    expect(isCommandEnabled("view.zoomIn")).toBe(false);
    await runCommand("view.zoomIn");
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 2 });
    expect(isCommandEnabled("view.zoomIn")).toBe(true);
    expect(isCommandEnabled("view.previousPage")).toBe(false);
  });

  it("navigates pages and zooms through the store", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 2 });
    await runCommand("view.nextPage");
    expect(selectActiveDocument(useAppStore.getState())?.page).toBe(1);
    expect(isCommandEnabled("view.nextPage")).toBe(false);
    await runCommand("view.zoomIn");
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      zoomMode: "custom",
      zoom: 1.1,
    });
    await runCommand("view.fitWidth");
    expect(selectActiveDocument(useAppStore.getState())?.zoomMode).toBe("fit-width");
  });

  it("switches language", async () => {
    useAppStore.setState({ locale: "en" });
    await runCommand("app.toggleLanguage");
    expect(useAppStore.getState().locale).toBe("ar");
  });
});
