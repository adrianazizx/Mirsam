import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setMockRecoveries } from "../backend/mock";
import { paletteMatch } from "../components/palette/match";
import { pageFromInput } from "../components/shell/StatusBar";
import {
  IDLE_SEARCH,
  parseSettings,
  resetSettingsChanges,
  selectActiveDocument,
  useAppStore,
} from "../state/appStore";
import { commandForEvent, isCommandEnabled, runCommand, shortcutLabel } from "./registry";

const key = (init: KeyboardEventInit & { target?: EventTarget }) => {
  const event = new KeyboardEvent("keydown", init);
  if (init.target) Object.defineProperty(event, "target", { value: init.target });
  return event;
};

beforeEach(() => {
  localStorage.clear();
  setMockRecoveries([]);
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    unlocks: [],
    recoveries: [],
    leftPanel: null,
    activeTool: "select",
  });
});

describe("search commands", () => {
  it("search the document and step through hits from the reader's page", async () => {
    // The mock's sample document: "4,500" appears on pages 1, 2, and twice on 3.
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    useAppStore.getState().updateDocument(1, { page: 1 });
    await runCommand("search.run", { query: "4,500" });
    let search = selectActiveDocument(useAppStore.getState())?.search ?? IDLE_SEARCH;
    expect(search.status).toBe("done");
    expect(search.hits.map((hit) => hit.page)).toEqual([0, 1, 2, 2]);
    // The first hit at or after the current page (page 2, index 1) is active.
    expect(search.active).toBe(1);
    await runCommand("search.next");
    await runCommand("search.next");
    await runCommand("search.next");
    search = selectActiveDocument(useAppStore.getState())?.search ?? IDLE_SEARCH;
    expect(search.active).toBe(0);
    await runCommand("search.previous");
    expect(selectActiveDocument(useAppStore.getState())?.search.active).toBe(3);
  });

  it("finds Arabic with and without hamza and reports an empty query as idle", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
    await runCommand("search.run", { query: "ابرم" });
    expect(selectActiveDocument(useAppStore.getState())?.search.hits).toHaveLength(1);
    await runCommand("search.toggleArabic");
    expect(selectActiveDocument(useAppStore.getState())?.search.hits).toHaveLength(0);
    await runCommand("search.toggleArabic");
    await runCommand("search.run", { query: "  " });
    expect(selectActiveDocument(useAppStore.getState())?.search.status).toBe("idle");
  });

  it("enables copy only with a selection", () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 2 });
    expect(isCommandEnabled("edit.copy")).toBe(false);
    useAppStore.getState().updateDocument(1, { hasSelection: true });
    expect(isCommandEnabled("edit.copy")).toBe(true);
  });
});

describe("shortcuts", () => {
  it("leave Cmd+C, Cmd+A, and Escape to text fields", () => {
    const field = document.createElement("input");
    expect(commandForEvent(key({ key: "c", metaKey: true }), true)?.id).toBe("edit.copy");
    expect(commandForEvent(key({ key: "c", metaKey: true, target: field }), true)).toBeUndefined();
    expect(commandForEvent(key({ key: "Escape", target: field }), true)).toBeUndefined();
    // Opening a file still works from a field.
    expect(commandForEvent(key({ key: "o", metaKey: true, target: field }), true)?.id).toBe(
      "file.open",
    );
  });

  it("switch tabs with Ctrl+Tab on every platform", () => {
    expect(commandForEvent(key({ key: "Tab", ctrlKey: true }), true)?.id).toBe("tabs.next");
    expect(commandForEvent(key({ key: "Tab", ctrlKey: true, shiftKey: true }), false)?.id).toBe(
      "tabs.previous",
    );
    expect(shortcutLabel("tabs.next", true)).toBe("⌃⇥");
    expect(shortcutLabel("search.previous", false)).toBe("Ctrl+Shift+G");
  });

  it("give every shortcut to exactly one command", () => {
    const seen = new Map<string, string>();
    for (const mac of [true, false]) {
      for (const id of [
        "file.open",
        "search.previous",
        "app.toggleGallery",
        "edit.copy",
      ] as const) {
        const label = `${mac}:${shortcutLabel(id, mac)}`;
        expect(seen.has(label), label).toBe(false);
        seen.set(label, id);
      }
    }
  });
});

describe("settings, palette, and page labels", () => {
  it("take the store as the source of truth over the first-paint copy", () => {
    resetSettingsChanges();
    useAppStore.setState({ leftPanel: "thumbnails", readingTheme: "night" });
    useAppStore.getState().setTheme("dark"); // changed before the store answered
    useAppStore.getState().hydrate({ readingTheme: "sepia", theme: "light" });
    expect(useAppStore.getState().leftPanel).toBeNull();
    expect(useAppStore.getState().readingTheme).toBe("sepia");
    expect(useAppStore.getState().theme).toBe("dark");
  });

  it("parse stored settings and ignore anything unknown", () => {
    expect(
      parseSettings({
        locale: "ar",
        theme: "purple",
        readingTheme: "night",
        leftPanel: "none",
        "search.options": '{"wholeWord":true,"regex":"yes"}',
      }),
    ).toEqual({
      locale: "ar",
      readingTheme: "night",
      leftPanel: null,
      searchOptions: {
        caseSensitive: false,
        wholeWord: true,
        regex: false,
        arabicNormalize: true,
        taMarbutaAsHa: false,
      },
    });
  });

  it("match palette commands loosely in either language", () => {
    expect(paletteMatch("Zoom in تكبير", "zi")).toBe(true);
    expect(paletteMatch("Zoom in تكبير", "تكب")).toBe(true);
    expect(paletteMatch("Next result النتيجة التالية", "النتيجه")).toBe(false);
    expect(paletteMatch("Pages: night الصفحات: ليلي", "night pages")).toBe(true);
    expect(paletteMatch("Copy نسخ", "paste")).toBe(false);
  });

  it("go to a page by its label or its number", () => {
    const labels = ["i", "ii", "1", "2"];
    expect(pageFromInput("ii", labels)).toBe(1);
    // A label that looks like a number wins over the page number.
    expect(pageFromInput("2", labels)).toBe(3);
    expect(pageFromInput("٣", null)).toBe(2);
    expect(pageFromInput("x", labels)).toBeNull();
  });
});

describe("tabs", () => {
  it("move in tab order and carry the id the drag code finds them by", async () => {
    render(<App />);
    act(() => {
      for (const id of [1, 2, 3]) {
        useAppStore.getState().addDocument({ id, name: `${id}.pdf`, pageCount: 1 });
      }
    });
    expect(document.querySelector('[data-document-id="2"]')?.getAttribute("role")).toBe("tab");
    await act(async () => {
      await runCommand("tabs.moveEarlier"); // the active tab (3) moves before 2
    });
    expect(useAppStore.getState().documents.map((open) => open.id)).toEqual([1, 3, 2]);
    await act(async () => {
      await runCommand("tabs.move", { documentId: 1, index: 2 });
    });
    expect(useAppStore.getState().documents.map((open) => open.id)).toEqual([3, 2, 1]);
  });
});

describe("password prompt", () => {
  it("asks again after a wrong password and opens with the right one", async () => {
    render(<App />);
    act(() => useAppStore.getState().pushUnlock({ token: 2, name: "سري.pdf", retry: false }));
    const field = (await screen.findByLabelText("Password")) as HTMLInputElement;
    fireEvent.change(field, { target: { value: "wrong" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Open" }));
    });
    expect(await screen.findByText("That password is not correct. Try again.")).toBeTruthy();
    expect(useAppStore.getState().unlocks[0]?.retry).toBe(true);
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "1234" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Open" }));
    });
    expect(useAppStore.getState().unlocks).toHaveLength(0);
    expect(selectActiveDocument(useAppStore.getState())?.name).toBe("سري — Confidential.pdf");
  });
});

describe("recovering unsaved changes", () => {
  const savedAt = Date.UTC(2026, 10, 22, 9, 30) / 1000;

  it("offers each document once, restores one with its changes, and discards the other", async () => {
    setMockRecoveries([
      { token: 11, name: "عقد إيجار — Lease Agreement.pdf", savedAt, originalChanged: false },
      { token: 12, name: "draft.pdf", savedAt, originalChanged: true },
    ]);
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Recover unsaved changes?" })).toBeTruthy();
    expect(screen.getByText("draft.pdf")).toBeTruthy();
    // Only the entry whose original changed says it is saved as a new file.
    expect(screen.getAllByText(/has changed since/)).toHaveLength(1);
    expect(screen.getAllByText(/^Last change /)).toHaveLength(2);

    await act(async () => {
      fireEvent.click(screen.getAllByRole("button", { name: "Restore" })[0] as HTMLElement);
    });
    const restored = selectActiveDocument(useAppStore.getState());
    expect(restored?.modified).toBe(true);
    expect(restored?.canUndo).toBe(true);
    expect(useAppStore.getState().recoveries.map((entry) => entry.token)).toEqual([12]);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    });
    expect(useAppStore.getState().recoveries).toHaveLength(0);
    expect(screen.queryByRole("heading", { name: "Recover unsaved changes?" })).toBeNull();
  });

  it("asks for an encrypted recovery file's password, again after a wrong one", async () => {
    setMockRecoveries([
      { token: 21, name: "سري — Confidential.pdf", savedAt, originalChanged: false },
    ]);
    render(<App />);
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Restore" }));
    });
    expect(useAppStore.getState().unlocks[0]?.recovery).toBe(true);
    fireEvent.change(await screen.findByLabelText("Password"), { target: { value: "wrong" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Open" }));
    });
    expect(useAppStore.getState().unlocks[0]?.retry).toBe(true);
    expect(useAppStore.getState().unlocks[0]?.recovery).toBe(true);
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "1234" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Open" }));
    });
    expect(useAppStore.getState().unlocks).toHaveLength(0);
    const restored = selectActiveDocument(useAppStore.getState());
    expect(restored?.name).toBe("سري — Confidential.pdf");
    expect(restored?.modified).toBe(true);
  });

  it("keeps them for the next start when the answer is later, in Arabic too", async () => {
    setMockRecoveries([{ token: 31, name: "draft.pdf", savedAt, originalChanged: true }]);
    render(<App />);
    act(() => useAppStore.getState().setLocale("ar"));
    expect(
      await screen.findByRole("heading", { name: "استعادة التغييرات غير المحفوظة؟" }),
    ).toBeTruthy();
    expect(screen.getByText(/^آخر تغيير /)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "لاحقًا" }));
    });
    expect(useAppStore.getState().recoveries).toHaveLength(0);
    expect(useAppStore.getState().documents).toHaveLength(0);
  });
});
