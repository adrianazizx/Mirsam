import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { backend } from "../backend";
import { DEFAULT_SEARCH_OPTIONS, selectActiveDocument, useAppStore } from "../state/appStore";
import { ALL_PERMISSIONS } from "../state/security";
import { commandForEvent, isCommandEnabled, runCommand, shortcutLabel } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());
const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);

beforeEach(() => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    leftPanel: null,
    activeTool: "select",
    searchOptions: DEFAULT_SEARCH_OPTIONS,
    replaceOpen: false,
    replacement: "",
  });
  // The mock's sample document: "4,500" is on pages 1, 2, and twice on 3. The
  // mock reads its odd pages (page 2 here) as text the editor cannot edit.
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** Undoes every edit the test made, so the mock's document is as it was. */
async function undoAll() {
  while (active()?.canUndo) await runCommand("edit.undo");
}

describe("find and replace commands (ADR 0052)", () => {
  it("open with Ctrl+H, and ⌥⌘F on macOS, where ⌘H hides the app", () => {
    expect(commandForEvent(key({ key: "h", ctrlKey: true }), false)?.id).toBe("search.replaceOpen");
    // Option+F types "ƒ" on a Mac: the physical key decides.
    expect(
      commandForEvent(key({ key: "ƒ", code: "KeyF", metaKey: true, altKey: true }), true)?.id,
    ).toBe("search.replaceOpen");
    expect(commandForEvent(key({ key: "h", metaKey: true }), true)).toBeUndefined();
    expect(commandForEvent(key({ key: "f", metaKey: true }), true)?.id).toBe("search.focus");
    expect(commandForEvent(key({ key: "f", ctrlKey: true, altKey: true }), false)).toBeUndefined();
    expect(shortcutLabel("search.replaceOpen", true)).toBe("⌥⌘F");
    expect(shortcutLabel("search.replaceOpen", false)).toBe("Ctrl+H");
  });

  it("replace the hit shown, show the next, and say why a hit cannot be replaced", async () => {
    await runCommand("search.run", { query: "4,500" });
    expect(active()?.search.hits.map((hit) => hit.page)).toEqual([0, 1, 2, 2]);
    await runCommand("search.setReplacement", { text: "4,750" });
    await runCommand("search.goTo", { index: 0 });
    const replace = vi.spyOn(backend, "replaceText");

    await runCommand("search.replace");
    expect(replace).toHaveBeenCalledWith(1, "4,500", DEFAULT_SEARCH_OPTIONS, "4,750", {
      kind: "hit",
      page: 0,
      rects: expect.any(Array),
    });
    await waitFor(() => expect(active()?.search.hits).toHaveLength(3));
    expect(active()).toMatchObject({ canUndo: true, modified: true });
    expect(useAppStore.getState().notice).toEqual({
      key: "replaced",
      values: { count: "1", skipped: "0" },
    });
    // The next hit takes the replaced one's place: page 2's, which cannot be edited.
    expect(active()?.search.active).toBe(0);
    expect(active()?.search.hits[0]?.page).toBe(1);

    await runCommand("search.replace");
    expect(useAppStore.getState().notice).toEqual({
      key: "replaceNone",
      values: { reason: "Its font can't be read for editing" },
    });
    // Nothing changed: still one step to undo, and the hit is still there.
    await waitFor(() => expect(active()?.search.active).toBe(1));
    expect(active()?.search.hits).toHaveLength(3);

    await runCommand("edit.undo");
    expect(active()?.canUndo).toBe(false);
    await runCommand("search.run", { query: "4,500" });
    expect(active()?.search.hits).toHaveLength(4);
  });

  it("preview every match, leave out what is unticked, and replace the rest as one undo step", async () => {
    await runCommand("search.run", { query: "4,500" });
    await runCommand("search.setReplacement", { text: "4,750" });
    expect(isCommandEnabled("search.replaceConfirm")).toBe(false);

    await runCommand("search.replaceAll");
    const plan = active()?.replace.plan;
    expect(active()?.replace.status).toBe("ready");
    expect(plan?.occurrences.map((o) => o.skip)).toEqual([null, "font", null, null]);
    expect(plan?.occurrences.map((o) => o.replacement)).toEqual([
      "4,750",
      "4,750",
      "4,750",
      "4,750",
    ]);
    expect(plan?.occurrences[0]?.rtl).toBe(true);
    expect(isCommandEnabled("search.replaceConfirm")).toBe(true);
    // While the preview is open, nothing else replaces.
    expect(isCommandEnabled("search.replace")).toBe(false);

    await runCommand("search.replaceToggle", { index: 3 });
    expect(active()?.replace.excluded).toEqual([3]);
    const replace = vi.spyOn(backend, "replaceText");
    await runCommand("search.replaceConfirm");
    const scope = replace.mock.calls[0]?.[4];
    expect(scope?.kind === "targets" && scope.targets.map((target) => target.page)).toEqual([0, 2]);

    expect(active()?.replace.status).toBe("idle");
    expect(active()).toMatchObject({ canUndo: true, modified: true });
    // What is left: the hit the editor cannot change, and the one left out.
    await waitFor(() => expect(active()?.search.hits.map((hit) => hit.page)).toEqual([1, 2]));
    expect(useAppStore.getState().notice?.key).toBe("replaced");

    // One undo takes the whole replacement back.
    await runCommand("edit.undo");
    expect(active()).toMatchObject({ canUndo: false, modified: false });
    await runCommand("search.run", { query: "4,500" });
    expect(active()?.search.hits).toHaveLength(4);
  });

  it("a changed option or replacement makes the preview again; cancel closes it", async () => {
    await runCommand("search.run", { query: "4,500" });
    await runCommand("search.replaceAll");
    const preview = vi.spyOn(backend, "replacePreview");
    await runCommand("search.setReplacement", { text: "٤٬٧٥" });
    await runCommand("search.setReplacement", { text: "٤٬٧٥٠" });
    // The list stays while it is made again, but cannot be applied.
    expect(active()?.replace.status).toBe("loading");
    expect(active()?.replace.plan?.occurrences).toHaveLength(4);
    expect(isCommandEnabled("search.replaceConfirm")).toBe(false);
    // Made again once, after the pause in typing.
    await waitFor(() => expect(preview).toHaveBeenCalledTimes(1));
    expect(preview).toHaveBeenCalledWith(1, "4,500", DEFAULT_SEARCH_OPTIONS, "٤٬٧٥٠");
    await waitFor(() => expect(active()?.replace.plan?.occurrences[0]?.replacement).toBe("٤٬٧٥٠"));
    await runCommand("search.replaceCancel");
    expect(active()?.replace.status).toBe("idle");
    expect(active()?.canUndo).toBe(false);
  });

  it("are off when the file does not allow changing it", () => {
    useAppStore.getState().addDocument({
      id: 3,
      name: "b.pdf",
      pageCount: 3,
      security: {
        encrypted: true,
        cipher: "aes256",
        revision: 6,
        owner: false,
        permissions: { ...ALL_PERMISSIONS, modify: false },
      },
    });
    useAppStore.getState().updateSearch(3, { query: "x", status: "done", hits: [], active: -1 });
    expect(isCommandEnabled("search.replaceOpen")).toBe(true);
    expect(isCommandEnabled("search.replaceAll")).toBe(false);
  });
});

describe("the search panel's replace row and preview", () => {
  afterEach(async () => {
    await act(undoAll);
  });

  it.each([
    [
      "en",
      "Replace with",
      "Replace all…",
      "To replace: 3. Can't be replaced: 1.",
      "Replace 3",
      "Replaced: 3.",
    ],
    // The Arabic UI counts in Arabic-Indic digits.
    [
      "ar",
      "استبدال بـ",
      "استبدال الكل…",
      "للاستبدال: ٣. لا يمكن استبدالها: ١.",
      "استبدال ٣",
      "ما استُبدل: ٣.",
    ],
  ] as const)("works in %s", async (locale, label, replaceAll, summary, confirm, done) => {
    render(<App />);
    act(() => useAppStore.getState().setLocale(locale));
    await act(async () => {
      await runCommand("search.replaceOpen");
      await runCommand("search.run", { query: "4,500" });
    });
    const field = await screen.findByRole("textbox", { name: label });
    fireEvent.change(field, { target: { value: "4,750" } });
    expect(useAppStore.getState().replacement).toBe("4,750");

    fireEvent.click(await screen.findByRole("button", { name: replaceAll }));
    expect((await screen.findByTestId("replace-status")).textContent).toBe(summary);
    const preview = within(await screen.findByTestId("replace-preview"));
    // Each match reads in its own direction; a skipped one says why.
    const rows = preview.getAllByRole("row");
    expect(rows).toHaveLength(4);
    expect(rows[0]?.querySelector("[dir]")?.getAttribute("dir")).toBe("rtl");
    expect(rows[1]?.querySelector("[dir]")?.getAttribute("dir")).toBe("ltr");
    expect(rows[1]?.textContent).toContain(
      locale === "ar" ? "لا يمكن قراءة خطها" : "font can't be read",
    );
    expect(rows[0]?.querySelector("ins")?.textContent).toBe("4,750");
    expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");

    expect(screen.getByTestId("replace-confirm").textContent).toBe(confirm);
    fireEvent.click(screen.getByTestId("replace-confirm"));
    await waitFor(() => expect(active()?.replace.status).toBe("idle"));
    expect(await screen.findByText(done)).toBeTruthy();
  });
});
