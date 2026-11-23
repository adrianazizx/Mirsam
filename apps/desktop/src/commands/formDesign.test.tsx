import { act, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { backend, type FormField } from "../backend";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { ALL_PERMISSIONS } from "../state/security";
import { drawnRect, moved, rowOrder } from "./formDesign";
import { isCommandEnabled, runCommand } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());
const fields = async () => (await backend.formFields(1)).fields;
const field = async (name: string) => {
  const found = (await fields()).find((candidate) => candidate.name === name);
  if (!found) throw new Error(`no field ${name}`);
  return found;
};
const design = () => useAppStore.getState().formDesign;

beforeEach(async () => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    activeTool: "select",
    formsMode: "fill",
    designTool: null,
    formDesign: null,
  });
  // The mock keeps its edits per document id: start each test from the file as opened.
  while ((await backend.undoEdit(1).catch(() => null))?.canUndo) {
    /* undo everything */
  }
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
  useAppStore.getState().updateDocument(1, { pageSize: { width: 612, height: 792 } });
});

describe("the form designer (ADR 0047)", () => {
  it("draws a field, changes it, moves it, and deletes it, each one undo step", async () => {
    await runCommand("forms.designMode");
    expect(useAppStore.getState().activeTool).toBe("forms");
    expect(design()?.fields.length).toBe(8);

    await runCommand("formDesign.text");
    expect(useAppStore.getState().designTool).toBe("text");
    // A click (no drag) places a field of the default size.
    await runCommand("formDesign.draw", { page: 0, rect: { x: 72, y: 600, width: 0, height: 0 } });
    const made = await field("Text 1");
    expect(made.widgets[0]?.view).toEqual({ x: 72, y: 600, width: 160, height: 22 });
    expect(design()?.selected).toEqual({ field: made.id, widget: 0 });
    expect(active()?.canUndo).toBe(true);

    await runCommand("formDesign.update", {
      spec: {
        kind: "text",
        name: "الاسم الكامل",
        tooltip: "كما في الهوية",
        required: true,
        readOnly: false,
        multiline: false,
        maxLen: 40,
        defaultValues: ["محمد العلي"],
        align: "end",
        font: "naskh",
        fontSize: 12,
        options: [],
        multiSelect: false,
        editable: false,
        bordered: true,
        caption: "",
      },
    });
    const changed = await field("الاسم الكامل");
    expect(changed).toMatchObject({ required: true, maxLen: 40, align: "end", font: "naskh" });
    expect(changed.values).toEqual(["محمد العلي"]);

    // A name another field has is refused.
    await runCommand("formDesign.update", { spec: { ...specFor(changed), name: "agree" } });
    expect(useAppStore.getState().errorCode).toBe("invalid_edit");
    useAppStore.getState().setError(null);

    await runCommand("formDesign.move", {
      field: changed.id,
      index: 0,
      page: 0,
      rect: { x: 100, y: 620, width: 200, height: 24 },
    });
    expect((await field("الاسم الكامل")).widgets[0]?.view).toEqual({
      x: 100,
      y: 620,
      width: 200,
      height: 24,
    });
    // The arrow keys nudge it, Shift by 10 points.
    await runCommand("formDesign.moveRightMore");
    await runCommand("formDesign.moveUp");
    expect((await field("الاسم الكامل")).widgets[0]?.view).toEqual({
      x: 110,
      y: 619,
      width: 200,
      height: 24,
    });

    await runCommand("formDesign.delete");
    expect((await fields()).some((f) => f.name === "الاسم الكامل")).toBe(false);
    expect(design()?.selected).toBeNull();
    await runCommand("edit.undo");
    expect((await fields()).some((f) => f.name === "الاسم الكامل")).toBe(true);
  });

  it("gives radio groups more buttons, and orders Tab by hand or by rows", async () => {
    await runCommand("formDesign.radio");
    await runCommand("formDesign.draw", {
      page: 0,
      rect: { x: 72, y: 700, width: 14, height: 14 },
    });
    const group = await field("Choice group 1");
    expect(group.options).toEqual([{ value: "0", label: "Choice 1" }]);
    await runCommand("formDesign.addButton");
    const two = await field("Choice group 1");
    expect(two.options.map((o) => o.label)).toEqual(["Choice 1", "Choice 2"]);
    // The new button sits beside the first and is selected.
    expect(two.widgets[1]?.view).toEqual({ x: 94, y: 700, width: 14, height: 14 });
    expect(design()?.selected).toEqual({ field: two.id, widget: 1 });
    await runCommand("formDesign.removeButton", { index: 0 });
    expect((await field("Choice group 1")).widgets).toHaveLength(1);

    // Earlier in tab order: from last to second last.
    const order = async () => (await fields()).map((f) => f.name);
    expect((await order()).at(-1)).toBe("Choice group 1");
    await runCommand("formDesign.tabEarlier");
    expect((await order()).slice(-2)).toEqual(["Choice group 1", "contractNo"]);

    // By rows, right to left: the radio buttons of "payment" share a row.
    await runCommand("formDesign.tabRowsRightToLeft");
    const names = await order();
    expect(names.indexOf("payment")).toBeLessThan(names.indexOf("city"));
  });

  it("finds fields on a flat page for review, and makes the ticked ones as one step", async () => {
    await runCommand("formDesign.detect");
    const found = design()?.detected;
    expect(found?.fields.map((f) => f.name)).toEqual(["الاسم", "Date", ""]);
    await runCommand("formDesign.toggleDetected", { index: 1 });
    expect(design()?.detected?.fields.map((f) => f.include)).toEqual([true, false, true]);
    await runCommand("formDesign.createDetected");
    const names = (await fields()).map((f) => f.name);
    expect(names).toContain("الاسم");
    expect(names).toContain("Check box 1");
    expect(names).not.toContain("Date");
    expect(design()?.detected).toBeNull();
    expect(useAppStore.getState().notice).toEqual({ key: "fieldsMade", values: { n: "2" } });
    // One undo takes both away.
    await runCommand("edit.undo");
    expect((await fields()).map((f) => f.name)).not.toContain("الاسم");
  });

  it.each([
    ["en", "Design", "Draw fields", "Text field", "Tab order on page 1"],
    // The Arabic UI numbers its pages in Arabic-Indic digits.
    ["ar", "تصميم", "رسم الحقول", "حقل نصي", "ترتيب التنقل في الصفحة ١"],
  ] as const)("shows the designer in %s", async (locale, mode, tools, kind, tabs) => {
    render(<App />);
    await act(async () => {
      useAppStore.getState().setLocale(locale);
    });
    await act(async () => {
      await runCommand("forms.openTool");
    });
    const panel = await screen.findByTestId("inspector");
    await act(async () => {
      within(panel).getByRole("radio", { name: mode }).click();
    });
    await screen.findByTestId("form-design-panel");
    expect(within(panel).getByText(tools)).toBeTruthy();
    expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");
    const name = (await field("tenant.name")).id;
    await act(async () => {
      await runCommand("formDesign.selectField", { field: name, index: 0 });
    });
    const inspector = await screen.findByTestId("field-inspector");
    expect(within(inspector).getByText(kind)).toBeTruthy();
    expect((within(inspector).getAllByRole("textbox")[0] as HTMLInputElement).value).toBe(
      "tenant.name",
    );
    await waitFor(() => expect(within(panel).getByText(tabs)).toBeTruthy());
    expect(within(screen.getByTestId("tab-order")).getAllByRole("listitem")).toHaveLength(8);
  });

  it("is switched off by a file's permissions unless opened with the owner password", async () => {
    const security = {
      encrypted: true,
      cipher: "aes256" as const,
      revision: 6,
      owner: false,
      permissions: { ...ALL_PERMISSIONS, modify: false },
    };
    useAppStore.setState({ documents: [], activeDocumentId: null });
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 3, security });
    await runCommand("forms.designMode");
    expect(isCommandEnabled("formDesign.text")).toBe(false);
    expect(isCommandEnabled("formDesign.createDetected")).toBe(false);
    // Looking is still allowed.
    expect(isCommandEnabled("formDesign.detect")).toBe(true);
  });
});

describe("the form designer's arithmetic", () => {
  it("places a click at the default size, kept on the page", () => {
    expect(
      drawnRect("checkbox", { x: 605, y: 10, width: 1, height: 1 }, { width: 612, height: 792 }),
    ).toEqual({
      x: 598,
      y: 10,
      width: 14,
      height: 14,
    });
    const drag = { x: 10, y: 10, width: 200, height: 30 };
    expect(drawnRect("text", drag, null)).toEqual(drag);
  });

  it("moves an id within a list, kept inside it", () => {
    expect(moved([1, 2, 3], 2, -1)).toEqual([1, 3, 2]);
    expect(moved([1, 2, 3], 0, -1)).toEqual([1, 2, 3]);
  });

  it("orders by rows, from the right or the left", () => {
    const at = (id: number, x: number, y: number): FormField => ({
      id,
      name: String(id),
      kind: "text",
      values: [],
      options: [],
      readOnly: false,
      required: false,
      multiline: false,
      password: false,
      comb: false,
      maxLen: null,
      multiSelect: false,
      editable: false,
      widgets: [{ page: 0, view: { x, y, width: 50, height: 20 }, onState: null, annotIndex: id }],
    });
    // Two rows; the second row's fields a little lower than each other.
    const list = [at(1, 10, 100), at(2, 300, 102), at(3, 300, 200), at(4, 10, 205)];
    expect(rowOrder(list, 0, false)).toEqual([1, 2, 4, 3]);
    expect(rowOrder(list, 0, true)).toEqual([2, 1, 3, 4]);
  });
});

function specFor(field: FormField) {
  return {
    kind: field.kind,
    name: field.name,
    tooltip: field.tooltip ?? "",
    required: field.required,
    readOnly: field.readOnly,
    multiline: field.multiline,
    maxLen: field.maxLen,
    defaultValues: field.defaultValues ?? [],
    align: field.align ?? "start",
    font: field.font ?? null,
    fontSize: field.fontSize ?? 0,
    options: field.options,
    multiSelect: field.multiSelect,
    editable: field.editable,
    bordered: field.bordered ?? true,
    caption: field.caption ?? "",
  } as const;
}
