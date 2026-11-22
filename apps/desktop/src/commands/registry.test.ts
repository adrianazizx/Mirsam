import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { commandForEvent, isCommandEnabled, runCommand, shortcutLabel } from "./registry";

const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

beforeEach(() => {
  useAppStore.setState({
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    closePrompt: null,
  });
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

  it("shows its own message when printing fails", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 2 });
    const print = vi
      .spyOn(backend, "printDocument")
      .mockRejectedValue({ code: "print_failed", detail: "no printer" });
    vi.spyOn(console, "error").mockImplementation(() => {});
    await runCommand("file.print");
    expect(print).toHaveBeenCalledWith(1);
    expect(useAppStore.getState().errorCode).toBe("print_failed");
    vi.restoreAllMocks();
  });

  it("edits pages, then undoes and redoes, starting the views over each time", async () => {
    // The mock backend's sample document (id 1) has 240 pages.
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    useAppStore.getState().updateDocument(1, { page: 4 });
    useAppStore.getState().updateSearch(1, { query: "x", status: "done" });
    expect(isCommandEnabled("edit.undo")).toBe(false);

    await runCommand("pages.duplicate");
    let doc = selectActiveDocument(useAppStore.getState());
    expect(doc).toMatchObject({
      pageCount: 241,
      revision: 1,
      canUndo: true,
      modified: true,
      page: 4,
    });
    expect(doc?.search.status).toBe("idle");
    expect(isCommandEnabled("edit.undo")).toBe(true);

    await runCommand("edit.undo");
    doc = selectActiveDocument(useAppStore.getState());
    expect(doc).toMatchObject({ pageCount: 240, revision: 2, canUndo: false, canRedo: true });
    expect(doc?.modified).toBe(false);

    await runCommand("edit.redo");
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      pageCount: 241,
      canRedo: false,
    });
    await runCommand("edit.undo");
    expect(shortcutLabel("edit.redo", true)).toBe("⇧⌘Z");
  });

  it("saves only when there is something to save, and undo past a save is a change again", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    expect(isCommandEnabled("file.save")).toBe(false);
    expect(isCommandEnabled("file.saveAs")).toBe(true);

    await runCommand("pages.rotateClockwise");
    expect(isCommandEnabled("file.save")).toBe(true);
    await runCommand("file.save");
    expect(selectActiveDocument(useAppStore.getState())?.modified).toBe(false);
    expect(isCommandEnabled("file.save")).toBe(false);

    await runCommand("edit.undo");
    expect(selectActiveDocument(useAppStore.getState())?.modified).toBe(true);
    await runCommand("file.saveAs");
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      name: "Copy.pdf",
      modified: false,
    });
    expect(shortcutLabel("file.saveAs", true)).toBe("⇧⌘S");
  });

  it("asks before closing a tab with unsaved changes", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    useAppStore.getState().addDocument({ id: 2, name: "b.pdf", pageCount: 12 });
    useAppStore.getState().activateDocument(1);
    await runCommand("pages.duplicate");

    await runCommand("file.close");
    expect(useAppStore.getState().closePrompt).toEqual({
      kind: "tab",
      documentId: 1,
      name: "a.pdf",
    });
    expect(useAppStore.getState().documents.map((open) => open.id)).toEqual([1, 2]);
    await runCommand("app.cancelClose");
    expect(useAppStore.getState().closePrompt).toBeNull();

    // An unmodified tab closes at once; "Don't save" closes the modified one.
    await runCommand("file.close", { documentId: 2 });
    await runCommand("file.close", { documentId: 1, force: true });
    expect(useAppStore.getState().documents).toEqual([]);
    expect(useAppStore.getState().closePrompt).toBeNull();
  });

  it("keeps the last page: deleting needs two", () => {
    useAppStore.getState().addDocument({ id: 7, name: "one.pdf", pageCount: 1 });
    expect(isCommandEnabled("pages.delete")).toBe(false);
    expect(isCommandEnabled("pages.rotateClockwise")).toBe(true);
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

  it("goes to a typed page, clamped to the document", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 10 });
    await runCommand("view.goToPage", { page: 6 });
    expect(selectActiveDocument(useAppStore.getState())?.page).toBe(6);
    await runCommand("view.goToPage", { page: 400 });
    expect(selectActiveDocument(useAppStore.getState())?.page).toBe(9);
    expect(shortcutLabel("view.goToPage", true)).toBe("⇧⌘N");
    expect(commandForEvent(key({ key: "N", ctrlKey: true, shiftKey: true }), false)?.id).toBe(
      "view.goToPage",
    );
  });

  it("switches view mode and spread page order", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 10 });
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      viewMode: "continuous",
      pageOrder: "ltr",
    });
    await runCommand("view.modeBook");
    await runCommand("view.pagesRightToLeft");
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      viewMode: "book",
      pageOrder: "rtl",
    });
    await runCommand("view.modeSingle");
    await runCommand("view.pagesLeftToRight");
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      viewMode: "single",
      pageOrder: "ltr",
    });
  });

  it("switches language", async () => {
    useAppStore.setState({ locale: "en" });
    await runCommand("app.toggleLanguage");
    expect(useAppStore.getState().locale).toBe("ar");
  });
});
