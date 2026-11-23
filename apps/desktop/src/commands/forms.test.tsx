import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { backend } from "../backend";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { ALL_PERMISSIONS } from "../state/security";
import { fillDate, isCommandEnabled, runCommand } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());
const annotations = () => backend.pageAnnotations(1, 0);
const fields = async () => (await backend.formFields(1)).fields;
const field = async (name: string) => {
  const found = (await fields()).find((candidate) => candidate.name === name);
  if (!found) throw new Error(`no field ${name}`);
  return found;
};

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
    fillSignature: null,
    fillColor: "black",
    signatures: null,
    signatureDialog: null,
  });
  // The mock keeps its edits per document id: start each test from the file as opened.
  while ((await backend.undoEdit(1).catch(() => null))?.canUndo) {
    /* undo everything */
  }
  for (const kept of await backend.signatures()) await backend.deleteSignature(kept.id);
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
});

describe("form filling (ADR 0045)", () => {
  it("sets text, check boxes, radio buttons, and choices, each one undo step", async () => {
    const name = await field("tenant.name");
    await runCommand("form.setValue", { field: name.id, values: ["محمد العلي"] });
    expect((await field("tenant.name")).values).toEqual(["محمد العلي"]);
    expect(active()?.modified).toBe(true);
    expect(active()?.canUndo).toBe(true);

    await runCommand("form.setValue", { field: (await field("agree")).id, values: ["Yes"] });
    await runCommand("form.setValue", { field: (await field("payment")).id, values: ["transfer"] });
    await runCommand("form.setValue", {
      field: (await field("languages")).id,
      values: ["ar", "en"],
    });
    expect((await field("payment")).values).toEqual(["transfer"]);
    expect((await field("languages")).values).toEqual(["ar", "en"]);

    await runCommand("edit.undo");
    expect((await field("languages")).values).toEqual([]);
    expect((await field("tenant.name")).values).toEqual(["محمد العلي"]);
  });

  it("refuses what the field does not take, and says so", async () => {
    await runCommand("form.setValue", { field: (await field("city")).id, values: ["mecca"] });
    expect(useAppStore.getState().errorCode).toBe("invalid_edit");
    useAppStore.getState().setError(null);
    await runCommand("form.setValue", { field: (await field("contractNo")).id, values: ["x"] });
    expect(useAppStore.getState().errorCode).toBe("invalid_edit");
  });
});

describe("a file's permissions (ADR 0043)", () => {
  it("switch off filling and Fill & Sign unless opened with the owner password", () => {
    const security = {
      encrypted: true,
      cipher: "aes256" as const,
      revision: 6,
      owner: false,
      permissions: { ...ALL_PERMISSIONS, fillForms: false, annotate: false },
    };
    useAppStore.getState().removeDocument(1);
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 3, security });
    for (const command of ["form.setValue", "fill.check", "fill.place", "fill.text"] as const) {
      expect(isCommandEnabled(command), command).toBe(false);
    }
    useAppStore.getState().removeDocument(1);
    useAppStore
      .getState()
      .addDocument({ id: 1, name: "a.pdf", pageCount: 3, security: { ...security, owner: true } });
    expect(isCommandEnabled("form.setValue")).toBe(true);
    expect(isCommandEnabled("fill.check")).toBe(true);
  });
});

describe("Fill & Sign (ADR 0045)", () => {
  it("places marks where pressed, and keeps the tool to tick several boxes", async () => {
    await runCommand("fill.check");
    expect(useAppStore.getState().activeTool).toBe("sign");
    expect(useAppStore.getState().fillTool).toBe("check");
    await runCommand("fill.place", { page: 0, x: 107, y: 107 });
    await runCommand("fill.place", { page: 0, x: 207, y: 107 });
    const placed = await annotations();
    expect(placed.map((item) => item.kind)).toEqual(["mark", "mark"]);
    // Centred on the press.
    expect(placed[0]?.rects[0]).toEqual({ x: 100, y: 100, width: 14, height: 14 });
    expect(useAppStore.getState().fillTool).toBe("check");
  });

  it("writes text after a press, and today's date at once", async () => {
    await runCommand("fill.text");
    await runCommand("fill.place", { page: 0, x: 50, y: 60 });
    expect(useAppStore.getState().pendingNote?.kind).toBe("fillText");
    await runCommand("note.save", { text: "سارة الحربي" });
    await runCommand("fill.date");
    await runCommand("fill.place", { page: 0, x: 50, y: 90 });
    const placed = await annotations();
    expect(placed.map((item) => [item.kind, item.contents])).toEqual([
      ["fillText", "سارة الحربي"],
      ["fillText", fillDate("en")],
    ]);
    // The date is placed once; the tool goes back to selecting.
    expect(useAppStore.getState().fillTool).toBeNull();
  });

  it("keeps signatures, places them in proportion, and resizes them", async () => {
    expect(isCommandEnabled("fill.signature")).toBe(false);
    await runCommand("signature.new");
    expect(useAppStore.getState().signatureDialog).toEqual({ initials: false });
    await runCommand("signature.saveTyped", { text: "Sarah", family: "naskh" });
    expect(useAppStore.getState().signatureDialog).toBeNull();
    await waitFor(() => expect(useAppStore.getState().signatures).toHaveLength(1));
    // A new signature is what the tool places next.
    expect(useAppStore.getState().fillTool).toBe("signature");
    const kept = useAppStore.getState().signatures?.[0];
    expect(useAppStore.getState().fillSignature).toBe(kept?.id);

    await runCommand("fill.place", { page: 0, x: 300, y: 400 });
    const [signature] = await annotations();
    expect(signature?.kind).toBe("signature");
    const rect = signature?.rects[0];
    expect(rect?.width).toBe(150);
    expect(rect && kept ? rect.width / rect.height : 0).toBeCloseTo(kept?.aspect ?? 0);
    // Centred on the press.
    expect(rect ? rect.x + rect.width / 2 : 0).toBe(300);

    if (!rect) throw new Error("the signature has no box");
    useAppStore.getState().updateDocument(1, {
      selectedAnnotation: { page: 0, index: 0, kind: "signature", contents: null, rect },
    });
    expect(isCommandEnabled("fill.larger")).toBe(true);
    await runCommand("fill.larger");
    const [larger] = await annotations();
    expect(larger?.rects[0]?.width).toBeCloseTo(187.5);
    expect(larger?.rects[0]?.x).toBe(rect?.x);

    await runCommand("signature.delete", { signature: kept?.id });
    expect(useAppStore.getState().signatures).toEqual([]);
    expect(useAppStore.getState().fillTool).toBeNull();
  });

  it("moves an item to where it is dragged, and nudges it with the arrow keys", async () => {
    await runCommand("fill.check");
    await runCommand("fill.place", { page: 0, x: 107, y: 107 });
    const [placed] = await annotations();
    const rect = placed?.rects[0];
    if (!placed || !rect) throw new Error("no mark");
    const item = { page: 0, index: placed.index, kind: placed.kind, contents: null, rect };

    await runCommand("fill.move", { annotation: item, rect: { ...rect, x: 300, y: 250 } });
    expect((await annotations())[0]?.rects[0]).toEqual({ x: 300, y: 250, width: 14, height: 14 });
    expect(active()?.canUndo).toBe(true);

    // Arrow keys move the selected item while the page view has the focus.
    useAppStore.getState().updateDocument(1, {
      selectedAnnotation: { ...item, rect: { ...rect, x: 300, y: 250 } },
    });
    expect(isCommandEnabled("fill.moveRight")).toBe(true);
    await runCommand("fill.moveRight");
    useAppStore.getState().updateDocument(1, {
      selectedAnnotation: { ...item, rect: { ...rect, x: 301, y: 250 } },
    });
    await runCommand("fill.moveDownMore");
    expect((await annotations())[0]?.rects[0]).toMatchObject({ x: 301, y: 260 });

    // In a text field they keep their own meaning.
    useAppStore.getState().updateDocument(1, { selectedAnnotation: { ...item, rect } });
    const field = document.createElement("input");
    document.body.append(field);
    field.focus();
    expect(isCommandEnabled("fill.moveRight")).toBe(false);
    field.remove();
  });

  it("places text, dates and marks in the chosen colour, and recolours the selected one", async () => {
    const addMark = vi.spyOn(backend, "addFillMark");
    const addText = vi.spyOn(backend, "addFillText");
    const recolour = vi.spyOn(backend, "setAnnotationColor");
    const red: [number, number, number] = [0.72, 0.11, 0.11];

    // Black until another colour is chosen; with nothing selected, choosing recolours nothing.
    await runCommand("fill.check");
    await runCommand("fill.place", { page: 0, x: 107, y: 107 });
    expect(addMark.mock.lastCall?.[6]).toEqual([0, 0, 0]);
    useAppStore.getState().updateDocument(1, { selectedAnnotation: null });
    await runCommand("fill.color", { colorId: "red" });
    expect(useAppStore.getState().fillColor).toBe("red");
    expect(recolour).not.toHaveBeenCalled();

    await runCommand("fill.place", { page: 0, x: 207, y: 107 });
    expect(addMark.mock.lastCall?.[6]).toEqual(red);
    await runCommand("fill.date");
    await runCommand("fill.place", { page: 0, x: 50, y: 200 });
    expect(addText.mock.lastCall?.[6]).toEqual(red);
    await runCommand("fill.text");
    await runCommand("fill.place", { page: 0, x: 50, y: 260 });
    await runCommand("note.save", { text: "نعم" });
    expect(addText.mock.lastCall?.[6]).toEqual(red);

    // Selected, the first (black) tick is recoloured: one undo step.
    const [first] = await annotations();
    const rect = first?.rects[0];
    if (!first || !rect) throw new Error("no mark");
    useAppStore.getState().updateDocument(1, {
      selectedAnnotation: { page: 0, index: first.index, kind: "mark", contents: null, rect },
    });
    await runCommand("fill.color", { colorId: "blue" });
    expect(recolour).toHaveBeenLastCalledWith(1, 0, first.index, [0.05, 0.28, 0.63]);
    expect(useAppStore.getState().fillColor).toBe("blue");

    // A signature keeps its own colour.
    recolour.mockClear();
    useAppStore.getState().updateDocument(1, {
      selectedAnnotation: { page: 0, index: first.index, kind: "signature", contents: null, rect },
    });
    await runCommand("fill.color", { colorId: "green" });
    expect(recolour).not.toHaveBeenCalled();
  });

  it("moves an item to another page, by a drag or to the next and previous page", async () => {
    await runCommand("fill.check");
    await runCommand("fill.place", { page: 0, x: 107, y: 107 });
    const [placed] = await annotations();
    const rect = placed?.rects[0];
    if (!placed || !rect) throw new Error("no mark");
    const item = { page: 0, index: placed.index, kind: placed.kind, contents: null, rect };

    // Dragged onto page 2: it leaves page 1 for page 2, one undo step.
    await runCommand("fill.move", { annotation: item, rect: { ...rect, y: 40 }, page: 1 });
    expect(await annotations()).toEqual([]);
    const [moved] = await backend.pageAnnotations(1, 1);
    expect(moved?.kind).toBe("mark");
    expect(moved?.rects[0]).toEqual({ ...rect, y: 40 });
    expect(active()?.canUndo).toBe(true);

    // Selected there, it goes back a page, where it was on its page.
    useAppStore.getState().updateDocument(1, {
      selectedAnnotation: { ...item, page: 1, index: moved?.index ?? 0, rect: { ...rect, y: 40 } },
    });
    expect(isCommandEnabled("fill.toPreviousPage")).toBe(true);
    await runCommand("fill.toPreviousPage");
    expect(await backend.pageAnnotations(1, 1)).toEqual([]);
    expect((await annotations())[0]?.rects[0]).toEqual({ ...rect, y: 40 });
    expect(active()?.page).toBe(0);

    // There is no page before the first.
    useAppStore.getState().updateDocument(1, { selectedAnnotation: { ...item, page: 0 } });
    expect(isCommandEnabled("fill.toPreviousPage")).toBe(false);
    expect(isCommandEnabled("fill.toNextPage")).toBe(true);
  });

  it("keeps a drawn signature as initials", async () => {
    await runCommand("signature.newInitials");
    await runCommand("signature.saveDrawn", {
      strokes: [
        [
          { x: 10, y: 10, pressure: 0.5 },
          { x: 60, y: 30, pressure: 0.5 },
        ],
      ],
    });
    await waitFor(() => expect(useAppStore.getState().signatures).toHaveLength(1));
    const kept = useAppStore.getState().signatures?.[0];
    expect(kept?.initials).toBe(true);
    expect(kept?.kind).toBe("drawn");
  });
});

describe("the Forms and Sign panels", () => {
  it.each([
    ["en", "Forms", "tenant.name", "Fields: 8"],
    ["ar", "النماذج", "tenant.name", "الحقول: ٨"],
  ] as const)("list the fields and fill them in %s", async (locale, heading, name, count) => {
    render(<App />);
    act(() => {
      useAppStore.getState().setLocale(locale);
      useAppStore.getState().setActiveTool("forms");
    });
    expect(await screen.findByRole("heading", { name: heading })).toBeTruthy();
    const panel = await screen.findByTestId("forms-panel");
    expect(panel.textContent).toContain(count);
    // A required field's label says so after its name.
    const input = within(panel).getByLabelText(name, { exact: false }) as HTMLInputElement;
    expect(input.getAttribute("dir")).toBe("auto");
    fireEvent.change(input, { target: { value: "نورة" } });
    fireEvent.blur(input);
    await waitFor(async () => expect((await field("tenant.name")).values).toEqual(["نورة"]));
    // The read-only field cannot be typed into.
    expect((within(panel).getByLabelText("contractNo") as HTMLInputElement).readOnly).toBe(true);
    expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");
  });

  it.each([
    ["en", "Sign", "New signature…", "Click each box to tick."],
    ["ar", "توقيع", "توقيع جديد…", "انقر على كل مربع لوضع علامة صح."],
  ] as const)("offer the tools and a new signature in %s", async (locale, heading, add, hint) => {
    render(<App />);
    act(() => {
      useAppStore.getState().setLocale(locale);
    });
    await act(async () => runCommand("fill.check"));
    expect(await screen.findByRole("heading", { name: heading })).toBeTruthy();
    expect((await screen.findByTestId("fill-hint")).textContent).toBe(hint);
    const panel = within(screen.getByTestId("sign-panel"));
    // The colours, named in the UI language; black is chosen.
    const swatches = within(screen.getByTestId("fill-colors")).getAllByRole("radio");
    expect(swatches).toHaveLength(4);
    expect((swatches[0] as HTMLInputElement).checked).toBe(true);
    fireEvent.click(panel.getAllByRole("button", { name: add })[0] as HTMLElement);
    const dialog = await screen.findByTestId("signature-dialog");
    expect(within(dialog).getByTestId("signature-pad")).toBeTruthy();
    await act(async () => runCommand("signature.cancel"));
    await waitFor(() => expect(screen.queryByTestId("signature-dialog")).toBeNull());
  });
});
