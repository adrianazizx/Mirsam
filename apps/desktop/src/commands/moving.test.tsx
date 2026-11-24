import { act, fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type AnnotationKind, backend, type ViewRect } from "../backend";
import { NoteEditor } from "../components/comments/NoteEditor";
import { applyLocale, i18n } from "../i18n";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { isCommandEnabled, runCommand } from "./registry";

/**
 * Moving what a user placed (ADR 0045's amendment): text boxes, notes and
 * drawings move as Fill & Sign items do — dragged (`fill.move`) or nudged by
 * the arrow keys — each move one undo step, before and after a save.
 */

const active = () => selectActiveDocument(useAppStore.getState());
const annotations = () => backend.pageAnnotations(1, 0);
const boxOf = async (kind: AnnotationKind) => {
  const found = (await annotations()).find((annotation) => annotation.kind === kind);
  const rect = found?.rects[0];
  if (!found || !rect) throw new Error(`no ${kind}`);
  return { found, rect };
};
const select = (kind: AnnotationKind, index: number, rect: ViewRect, contents: string | null) =>
  useAppStore.getState().updateDocument(1, {
    selectedAnnotation: { page: 0, index, kind, contents, rect },
  });

afterEach(() => {
  vi.restoreAllMocks();
});

beforeEach(async () => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    activeTool: "select",
    pendingNote: null,
    fillTool: null,
  });
  while ((await backend.undoEdit(1).catch(() => null))?.canUndo) {
    /* undo everything */
  }
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
});

describe.each(["en", "ar"] as const)("moving placed items in %s", (locale) => {
  beforeEach(() => {
    useAppStore.setState({ locale });
    applyLocale(locale);
  });

  it.each([
    ["textBox", "مربع نص — a text box"],
    ["note", "ملاحظة — a note"],
    ["ink", null],
  ] as const)(
    "moves a %s where it is dragged and by the arrow keys, before and after a save",
    async (kind, text) => {
      if (kind === "textBox") await backend.addTextBox(1, 0, 60, 500, 160, text ?? "");
      else if (kind === "note") await backend.addNote(1, 0, 60, 500, text ?? "");
      else
        await backend.addInk(1, 0, [
          [
            { x: 60, y: 500, pressure: 0.5 },
            { x: 140, y: 520, pressure: 0.5 },
          ],
        ]);
      const { found, rect } = await boxOf(kind);
      const item = { page: 0, index: found.index, kind, contents: text, rect };

      // Dragged: one undo step, the same size.
      await runCommand("fill.move", { annotation: item, rect: { ...rect, x: 300, y: 250 } });
      const dragged = (await boxOf(kind)).rect;
      expect(dragged).toMatchObject({ x: 300, y: 250, width: rect.width, height: rect.height });
      if (kind === "ink") {
        const strokes = (await boxOf(kind)).found.strokes;
        expect(strokes[0]?.[0]?.[0]).toBeCloseTo(300 + (60 - rect.x));
      }
      expect(active()?.canUndo).toBe(true);

      // Saved, then nudged with the arrow keys (the page view has the focus): right is right
      // in either language, as the page is drawn.
      await runCommand("file.save");
      expect(active()?.modified).toBe(false);
      select(kind, found.index, dragged, text);
      expect(isCommandEnabled("fill.moveRight")).toBe(true);
      await runCommand("fill.moveRight");
      select(kind, found.index, { ...dragged, x: 301 }, text);
      await runCommand("fill.moveDownMore");
      expect((await boxOf(kind)).rect).toMatchObject({ x: 301, y: 260 });
      expect(active()?.modified).toBe(true);

      // Kept on the page: nudged off the top, it stops at the edge.
      select(kind, found.index, { ...dragged, x: 301, y: 4 }, text);
      await runCommand("fill.moveUpMore");
      expect((await boxOf(kind)).rect.y).toBe(0);

      // Each nudge was one step; undo goes back to where it was saved.
      await runCommand("edit.undo");
      await runCommand("edit.undo");
      await runCommand("edit.undo");
      expect((await boxOf(kind)).rect).toMatchObject({ x: 300, y: 250 });
      expect(active()?.modified).toBe(false);
    },
  );

  it("adds up arrow keys pressed faster than the file is written", async () => {
    await backend.addTextBox(1, 0, 60, 500, 160, "مربع");
    const box = await boxOf("textBox");
    select("textBox", box.found.index, box.rect, "مربع");
    // A held key: three nudges before the first is written.
    await Promise.all([
      runCommand("fill.moveRightMore"),
      runCommand("fill.moveRightMore"),
      runCommand("fill.moveRightMore"),
    ]);
    expect((await boxOf("textBox")).rect.x).toBe(box.rect.x + 30);
  });

  it("widens a text box, scales a drawing, and keeps a note's icon its size", async () => {
    await backend.addTextBox(1, 0, 60, 500, 160, "نص طويل يلتف على عدة أسطر");
    await backend.addNote(1, 0, 300, 500, "ملاحظة");
    const box = await boxOf("textBox");
    await runCommand("fill.move", {
      annotation: {
        page: 0,
        index: box.found.index,
        kind: "textBox",
        contents: null,
        rect: box.rect,
      },
      rect: { ...box.rect, width: 80 },
    });
    const narrow = (await boxOf("textBox")).rect;
    expect(narrow.width).toBe(80);
    expect(narrow.height).toBeGreaterThan(box.rect.height);

    const note = await boxOf("note");
    await runCommand("fill.move", {
      annotation: {
        page: 0,
        index: note.found.index,
        kind: "note",
        contents: null,
        rect: note.rect,
      },
      rect: { ...note.rect, width: 40, height: 40 },
    });
    expect(useAppStore.getState().errorCode).toBe("invalid_edit");
    expect((await boxOf("note")).rect).toEqual(note.rect);
  });

  it("moves a selected note or text box only to the page it is on", async () => {
    await backend.addTextBox(1, 0, 60, 500, 160, "مربع");
    const box = await boxOf("textBox");
    const move = vi.spyOn(backend, "moveAnnotationToPage");
    await runCommand("fill.move", {
      annotation: {
        page: 0,
        index: box.found.index,
        kind: "textBox",
        contents: null,
        rect: box.rect,
      },
      rect: box.rect,
      page: 1,
    });
    expect(move).not.toHaveBeenCalled();
    select("textBox", box.found.index, box.rect, "مربع");
    expect(isCommandEnabled("fill.toNextPage")).toBe(false);
  });

  it("leaves a placed text box's editor for the page on Escape, the box still selected", async () => {
    await backend.addTextBox(1, 0, 60, 500, 160, "مربع");
    const box = await boxOf("textBox");
    select("textBox", box.found.index, box.rect, "مربع");
    render(
      <I18nextProvider i18n={i18n}>
        <NoteEditor documentId={1} />
      </I18nextProvider>,
    );
    const field = screen.getByRole("textbox");
    expect(field).toHaveProperty("value", "مربع");
    fireEvent.change(field, { target: { value: "مربع معدل" } });
    const toPage = vi.spyOn(backend, "setAnnotationContents");
    await act(async () => {
      fireEvent.keyDown(field, { key: "Escape" });
    });
    // What was typed is dropped, nothing is written, and the box stays selected.
    expect(field).toHaveProperty("value", "مربع");
    expect(toPage).not.toHaveBeenCalled();
    expect(active()?.selectedAnnotation?.kind).toBe("textBox");
    // A press elsewhere saves the (unchanged) text: no edit.
    fireEvent.pointerDown(document.body);
    expect(toPage).not.toHaveBeenCalled();
  });
});
