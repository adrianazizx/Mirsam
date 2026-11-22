import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import type { ViewerController } from "../viewer/controller";
import { registerViewer, unregisterViewer } from "../viewer/registry";
import {
  commandForEvent,
  dropTarget,
  isCommandEnabled,
  moveTarget,
  runCommand,
  shortcutLabel,
  targetPages,
} from "./registry";

const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

beforeEach(() => {
  useAppStore.setState({
    activeTool: "select",
    commentTool: "note",
    pendingNote: null,
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

  it("works out where moved pages land", () => {
    // One step: a block moves past its neighbour; a scattered selection gathers.
    expect(moveTarget([3, 4], 10, -1)).toBe(2);
    expect(moveTarget([0, 1], 10, -1)).toBeNull();
    expect(moveTarget([0, 5], 10, -1)).toBe(0);
    expect(moveTarget([3, 4], 10, 1)).toBe(4);
    expect(moveTarget([8, 9], 10, 1)).toBeNull();
    expect(moveTarget([2, 9], 10, 1)).toBe(3);
    // Dropping: how many unmoved pages precede the drop point.
    expect(dropTarget([0, 1], 3, true)).toBe(2); // after page 4 → pages 3, 4 come first
    expect(dropTarget([5], 0, false)).toBe(0);
    expect(dropTarget([2, 6], 4, false)).toBe(3);
  });

  it("acts on the organizer's selection, and keeps the moved pages selected", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    useAppStore.getState().updateDocument(1, { page: 7 });
    useAppStore.getState().setSelectedPages(1, [5, 3]);
    // Outside the organizer the current page is the target.
    expect(targetPages()).toEqual([7]);
    useAppStore.getState().setActiveTool("pages");
    expect(targetPages()).toEqual([3, 5]);

    await runCommand("pages.moveEarlier");
    // The mock keeps its edits between tests: count relative to here.
    const before = selectActiveDocument(useAppStore.getState())?.pageCount ?? 0;
    expect(selectActiveDocument(useAppStore.getState())?.selectedPages).toEqual([2, 3]);
    await runCommand("pages.duplicate");
    const doc = selectActiveDocument(useAppStore.getState());
    expect(doc?.pageCount).toBe(before + 2);
    expect(doc?.selectedPages).toEqual([3, 5]);
    await runCommand("pages.move", { pages: [3, 5], to: 0 });
    expect(selectActiveDocument(useAppStore.getState())?.selectedPages).toEqual([0, 1]);
    expect(isCommandEnabled("pages.moveEarlier")).toBe(false);
    for (let i = 0; i < 2; i += 1) await runCommand("edit.undo");
    expect(selectActiveDocument(useAppStore.getState())?.pageCount).toBe(before);
  });

  it("inserts another file's pages, extracts, and splits", async () => {
    // The mock keeps its edits between tests: start from its current page count.
    const before = (await backend.pageSizes(1)).length;
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: before });
    useAppStore.getState().setActiveTool("pages");
    useAppStore.getState().setSelectedPages(1, [4]);

    // The mock picks a three-page file; it goes in after the selection, selected.
    await runCommand("pages.insertFromFile");
    let doc = selectActiveDocument(useAppStore.getState());
    expect(doc?.pageCount).toBe(before + 3);
    expect(doc?.selectedPages).toEqual([5, 6, 7]);

    await runCommand("pages.extract");
    expect(useAppStore.getState().notice).toEqual({
      key: "extracted",
      values: { name: "Extract.pdf" },
    });

    await runCommand("pages.split");
    expect(useAppStore.getState().splitPrompt).toBe(true);
    await runCommand("pages.splitInto", { size: 100 });
    expect(useAppStore.getState().splitPrompt).toBe(false);
    expect(useAppStore.getState().notice).toEqual({
      key: "split",
      values: { count: String(Math.ceil((before + 3) / 100)) },
    });
    await runCommand("edit.undo");
    doc = selectActiveDocument(useAppStore.getState());
    expect(doc?.pageCount).toBe(before);
  });

  it("marks the selected text, and deletes markup clicked on, painting pages again", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    expect(isCommandEnabled("markup.highlight")).toBe(false);
    const rect = { x: 10, y: 10, width: 50, height: 12 };
    const clearSelection = vi.fn();
    const viewer = {
      selectionMarks: () => [{ page: 2, rects: [rect] }],
      clearSelection,
    } as unknown as ViewerController;
    registerViewer(1, viewer);
    useAppStore.getState().updateDocument(1, { hasSelection: true });
    // The mock keeps its edits between tests: count relative to here.
    const before = (await backend.pageAnnotations(1, 2)).length;

    await runCommand("markup.highlight");
    expect(clearSelection).toHaveBeenCalled();
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      paint: 1,
      revision: 0,
      modified: true,
      canUndo: true,
    });
    const marked = await backend.pageAnnotations(1, 2);
    expect(marked.at(-1)).toEqual({
      index: before,
      kind: "highlight",
      rects: [rect],
      contents: null,
      strokes: [],
    });

    // Clicking it selects it; the delete key removes it.
    useAppStore.getState().updateDocument(1, {
      selectedAnnotation: { page: 2, index: before, kind: "highlight", contents: null, rect },
    });
    expect(commandForEvent(key({ key: "Backspace" }), true)?.id).toBe("annotation.delete");
    expect(commandForEvent(key({ key: "Delete" }), false)?.id).toBe("annotation.delete");
    expect(shortcutLabel("annotation.delete", true)).toBe("⌫");
    expect(shortcutLabel("annotation.delete", false)).toBe("Del");
    expect(isCommandEnabled("edit.deselect")).toBe(true);
    await runCommand("annotation.delete");
    expect(await backend.pageAnnotations(1, 2)).toHaveLength(before);
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      paint: 2,
      selectedAnnotation: null,
    });

    // Undo is an annotation change too: the pages are painted, not rebuilt.
    await runCommand("edit.undo");
    expect(await backend.pageAnnotations(1, 2)).toHaveLength(before + 1);
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({ paint: 3, revision: 0 });
    await runCommand("edit.undo");
    expect(await backend.pageAnnotations(1, 2)).toHaveLength(before);
    unregisterViewer(1, viewer);
  });

  it("writes notes: placed, saved with their text, edited, and cancelled", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    const clearSelection = vi.fn();
    const viewer = { clearSelection } as unknown as ViewerController;
    registerViewer(1, viewer);
    await runCommand("comment.note");
    expect(useAppStore.getState()).toMatchObject({ activeTool: "comment", commentTool: "note" });
    const before = (await backend.pageAnnotations(1, 3)).length;

    // An empty new note is no note.
    await runCommand("note.place", { page: 3, x: 40, y: 60 });
    expect(useAppStore.getState().pendingNote).toEqual({ documentId: 1, page: 3, x: 40, y: 60 });
    await runCommand("note.save", { text: "  " });
    expect(useAppStore.getState().pendingNote).toBeNull();
    expect(await backend.pageAnnotations(1, 3)).toHaveLength(before);

    await runCommand("note.place", { page: 3, x: 40, y: 60 });
    await runCommand("note.save", { text: "راجع البند ٤" });
    const added = (await backend.pageAnnotations(1, 3)).at(-1);
    expect(added).toMatchObject({ kind: "note", contents: "راجع البند ٤" });
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({
      paint: 1,
      modified: true,
    });

    // The selected note: saving its text unchanged edits nothing; a new text is one step.
    const rect = { x: 40, y: 60, width: 20, height: 20 };
    const select = () =>
      useAppStore.getState().updateDocument(1, {
        selectedAnnotation: {
          page: 3,
          index: added?.index ?? 0,
          kind: "note",
          contents: "راجع البند ٤",
          rect,
        },
      });
    select();
    await runCommand("note.save", { text: "راجع البند ٤" });
    expect(clearSelection).toHaveBeenCalled();
    expect(selectActiveDocument(useAppStore.getState())?.paint).toBe(1);
    select();
    await runCommand("note.save", { text: "check clause 4" });
    expect((await backend.pageAnnotations(1, 3)).at(-1)?.contents).toBe("check clause 4");
    // Cancel leaves it as it is.
    select();
    await runCommand("note.cancel");
    expect((await backend.pageAnnotations(1, 3)).at(-1)?.contents).toBe("check clause 4");

    for (let i = 0; i < 2; i += 1) await runCommand("edit.undo");
    expect(await backend.pageAnnotations(1, 3)).toHaveLength(before);
    unregisterViewer(1, viewer);
  });

  it("adds ink and erases it, each in one step", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    await runCommand("comment.pen");
    expect(useAppStore.getState().commentTool).toBe("pen");
    const before = (await backend.pageAnnotations(1, 5)).length;
    const stroke = [
      { x: 10, y: 10, pressure: 0.5 },
      { x: 60, y: 20, pressure: 0.8 },
    ];
    await runCommand("ink.add", { page: 5, strokes: [stroke] });
    const ink = (await backend.pageAnnotations(1, 5)).at(-1);
    expect(ink).toMatchObject({
      kind: "ink",
      strokes: [
        [
          [10, 10],
          [60, 20],
        ],
      ],
    });
    await runCommand("comment.eraser");
    await runCommand("ink.erase", { page: 5, indices: [ink?.index ?? 0] });
    expect(await backend.pageAnnotations(1, 5)).toHaveLength(before);
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({ paint: 2, revision: 0 });
    for (let i = 0; i < 2; i += 1) await runCommand("edit.undo");
    expect(await backend.pageAnnotations(1, 5)).toHaveLength(before);
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
