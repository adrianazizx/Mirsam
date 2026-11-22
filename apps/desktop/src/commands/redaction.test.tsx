import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { DEFAULT_SANITIZE } from "../state/redaction";
import { isCommandEnabled, runCommand } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());

beforeEach(() => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    activeTool: "protect",
    redactPrompt: false,
    sanitizeOptions: DEFAULT_SANITIZE,
  });
  // The mock's sample document: "4,500" is on pages 1, 2, and twice on 3.
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
});

describe("redaction commands (ADR 0042)", () => {
  it("find, mark, review, undo before applying, and apply as one backend step", async () => {
    await runCommand("redact.find", { query: "4,500" });
    expect(active()?.redaction.find.hits.map((hit) => hit.page)).toEqual([0, 1, 2, 2]);
    expect(isCommandEnabled("redact.apply")).toBe(false);

    await runCommand("redact.markHit", { index: 0 });
    await runCommand("redact.markAllHits");
    // The first hit was marked already: marking all adds the other three.
    expect(active()?.redaction.marks.map((mark) => mark.page)).toEqual([0, 1, 2, 2]);
    expect(active()?.redaction.marks[0]?.text).toBe("4,500");
    expect(isCommandEnabled("redact.apply")).toBe(true);

    // Undo steps through the marks while the Protect tool is open, redo brings one back.
    await runCommand("edit.undo");
    expect(active()?.redaction.marks).toHaveLength(3);
    await runCommand("edit.redo");
    expect(active()?.redaction.marks).toHaveLength(4);
    const firstId = active()?.redaction.marks[0]?.id ?? 0;
    await runCommand("redact.removeMark", { markId: firstId });
    expect(active()?.redaction.marks).toHaveLength(3);

    // Applying asks first; cancelling changes nothing.
    await runCommand("redact.apply");
    expect(useAppStore.getState().redactPrompt).toBe(true);
    await runCommand("redact.cancel");
    expect(useAppStore.getState().redactPrompt).toBe(false);
    expect(active()?.redaction.marks).toHaveLength(3);
    expect(active()?.modified).toBe(false);

    await runCommand("redact.confirm");
    expect(active()?.redaction.marks).toEqual([]);
    expect(active()?.redaction.applied?.pages).toBe(2);
    expect(active()?.modified).toBe(true);
    expect(active()?.canUndo).toBe(true);
    // With nothing marked, undo is the document's: it takes the redaction back.
    await runCommand("edit.undo");
    expect(active()?.modified).toBe(false);
  });

  it("marks areas by dragging, ignoring stray clicks, and removes duplicates", async () => {
    const rect = { x: 20, y: 30, width: 100, height: 14 };
    await runCommand("redact.addArea", { page: 0, rect: { ...rect, width: 1 } });
    expect(active()?.redaction.marks).toEqual([]);
    await runCommand("redact.addArea", { page: 0, rect });
    await runCommand("redact.addArea", { page: 0, rect });
    expect(active()?.redaction.marks).toHaveLength(1);
    await runCommand("redact.clearMarks");
    expect(active()?.redaction.marks).toEqual([]);
  });

  it("finds presets, and reports a bad regular expression", async () => {
    await runCommand("redact.findPreset", { preset: "idNumber" });
    expect(active()?.redaction.preset).toBe("idNumber");
    expect(active()?.redaction.find.status).toBe("done");
    await runCommand("redact.toggleRegex");
    expect(active()?.redaction.regex).toBe(true);
    await runCommand("redact.find", { query: "  " });
    expect(active()?.redaction.find.status).toBe("idle");
  });

  it("sanitizes with the chosen options and reports what went", async () => {
    await runCommand("sanitize.setOption", { option: "comments", value: false });
    expect(useAppStore.getState().sanitizeOptions.comments).toBe(false);
    await runCommand("sanitize.run");
    expect(active()?.redaction.sanitized).toEqual({
      metadata: 0,
      scripts: 0,
      attachments: 0,
      comments: 0,
      hidden: 0,
    });
    for (const option of Object.keys(DEFAULT_SANITIZE) as (keyof typeof DEFAULT_SANITIZE)[]) {
      await runCommand("sanitize.setOption", { option, value: false });
    }
    expect(isCommandEnabled("sanitize.run")).toBe(false);
  });

  it("starts over when the pages change, since page numbers moved", async () => {
    await runCommand("redact.addArea", { page: 0, rect: { x: 1, y: 1, width: 50, height: 10 } });
    expect(active()?.redaction.marks).toHaveLength(1);
    await runCommand("pages.rotateClockwise");
    expect(active()?.redaction.marks).toEqual([]);
  });
});

describe("the Protect panel", () => {
  it.each([
    ["en", "Redact", "Apply redactions…", "Find"],
    ["ar", "التنقيح", "تطبيق التنقيح…", "بحث"],
  ] as const)(
    "shows its controls and the confirmation in %s",
    async (locale, heading, apply, find) => {
      render(<App />);
      act(() => useAppStore.getState().setLocale(locale));
      // The panel is queried afresh each time: the language change re-renders it.
      const panel = () => within(screen.getByTestId("protect-panel"));
      expect(await screen.findByRole("heading", { name: heading })).toBeTruthy();
      expect(panel().getByRole("button", { name: find })).toBeTruthy();
      expect(panel().getByRole("button", { name: apply }).hasAttribute("disabled")).toBe(true);

      await act(async () => {
        await runCommand("redact.find", { query: "4,500" });
        await runCommand("redact.markAllHits");
      });
      expect((await screen.findByTestId("mark-summary")).textContent).toContain("4");
      const applyButton = panel().getByRole("button", { name: apply });
      expect(applyButton.hasAttribute("disabled")).toBe(false);
      fireEvent.click(applyButton);
      const dialog = await screen.findByTestId("redact-dialog");
      expect(dialog.textContent).toContain("4");
      expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");

      fireEvent.click(within(dialog).getAllByRole("button")[0] as HTMLElement);
      await waitFor(() => expect(active()?.redaction.marks).toEqual([]));
      expect(await screen.findByTestId("redaction-report")).toBeTruthy();
    },
  );
});
