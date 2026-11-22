import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { backend } from "../backend";
import { setMockInsertIsLocked, setMockRecoveries } from "../backend/mock";
import { paletteMatch } from "../components/palette/match";
import { pageFromInput } from "../components/shell/StatusBar";
import {
  IDLE_SEARCH,
  parseSettings,
  resetSettingsChanges,
  selectActiveDocument,
  useAppStore,
} from "../state/appStore";
import { DEFAULT_COMMENT_FILTER } from "../state/comments";
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
    commentAuthor: "",
    commentFilter: DEFAULT_COMMENT_FILTER,
    commentSort: "page",
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
        commentAuthor: "  Reviewer  ",
        "search.options": '{"wholeWord":true,"regex":"yes"}',
      }),
    ).toEqual({
      locale: "ar",
      readingTheme: "night",
      leftPanel: null,
      commentAuthor: "Reviewer",
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

describe("comments panel", () => {
  const addNote = async (id: number, page: number, text: string) => {
    await act(async () => {
      useAppStore.getState().applyEdit(id, await backend.addNote(id, page, 30, 40, text));
    });
  };

  it("lists, answers, resolves and filters comments, signed with the chosen name", async () => {
    render(<App />);
    act(() => useAppStore.getState().addDocument({ id: 41, name: "review.pdf", pageCount: 240 }));
    await addNote(41, 0, "First note");
    await act(async () => {
      await runCommand("view.showComments");
    });
    const panel = await screen.findByTestId("comments");
    await within(panel).findByText("First note");
    expect(within(panel).getByText("No name")).toBeTruthy();

    // A name chosen in the panel signs what comes next.
    const name = within(panel).getByLabelText("Your name on comments");
    fireEvent.change(name, { target: { value: "مراجِع" } });
    await act(async () => {
      fireEvent.blur(name);
    });
    expect(useAppStore.getState().commentAuthor).toBe("مراجِع");
    await addNote(41, 2, "راجع البند ٤");
    await within(panel).findByText("راجع البند ٤");
    expect(within(panel).getByRole("heading", { name: "Page 3" })).toBeTruthy();

    // Reply to the first comment.
    const first = () => within(panel).getAllByTestId("comment")[0] as HTMLElement;
    fireEvent.click(within(first()).getByRole("button", { name: "Reply" }));
    fireEvent.change(within(first()).getByLabelText("Your reply"), { target: { value: "Agreed" } });
    await act(async () => {
      fireEvent.click(within(first()).getByRole("button", { name: "Reply" }));
    });
    await within(first()).findByText("Agreed");
    expect(within(first()).getAllByText("مراجِع")).toHaveLength(1);

    // Resolve it, then filter.
    await act(async () => {
      fireEvent.click(within(first()).getByRole("button", { name: "Resolve" }));
    });
    await within(first()).findByText("Resolved");
    expect(within(first()).getByRole("button", { name: "Reopen" })).toBeTruthy();
    await act(async () => {
      await runCommand("comments.filter", { filter: { status: "open" } });
    });
    expect(within(panel).queryByText("First note")).toBeNull();
    expect(within(panel).getByText("راجع البند ٤")).toBeTruthy();
    await act(async () => {
      await runCommand("comments.filter", { filter: { status: "resolved", kind: "textBoxes" } });
    });
    expect(within(panel).getByText("No comments match these filters.")).toBeTruthy();

    // Undo takes the resolution back.
    await act(async () => {
      await runCommand("comments.filter", { filter: DEFAULT_COMMENT_FILTER });
      await runCommand("edit.undo");
    });
    await within(first()).findByRole("button", { name: "Resolve" });

    // Choosing a comment goes to its page.
    fireEvent.click(within(panel).getByRole("button", { name: "Note on page 3: show it" }));
    expect(selectActiveDocument(useAppStore.getState())?.page).toBe(2);
  });

  it("speaks Arabic, right to left", async () => {
    render(<App />);
    act(() => {
      useAppStore.getState().setLocale("ar");
      useAppStore.getState().addDocument({ id: 42, name: "عقد.pdf", pageCount: 240 });
    });
    await act(async () => {
      await runCommand("view.showComments");
    });
    expect(await screen.findByText("لا تعليقات في هذا المستند بعد.")).toBeTruthy();
    expect(screen.getByRole("radio", { name: "التعليقات" })).toBeTruthy();
    await addNote(42, 0, "ملاحظة");
    const panel = screen.getByTestId("comments");
    await within(panel).findByText("ملاحظة");
    // The UI's own numbers are Western digits in Arabic too (i18n `formatTag`).
    expect(within(panel).getByRole("heading", { name: "الصفحة 1" })).toBeTruthy();
    expect(within(panel).getByText("بلا اسم")).toBeTruthy();
  });
});

describe("split dialog", () => {
  it("splits by pages, at top-level bookmarks, or by file size", async () => {
    render(<App />);
    act(() => useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 }));
    await act(async () => {
      await runCommand("pages.split");
    });
    const dialog = await screen.findByRole("dialog", { name: "Split into files" });
    expect(within(dialog).getByLabelText("Pages per file")).toBeTruthy();

    // The mock's outline starts parts at its three top-level entries, named in the dialog.
    fireEvent.click(await within(dialog).findByRole("radio", { name: "At top-level bookmarks" }));
    const list = await within(dialog).findByTestId("split-bookmarks");
    expect(list.textContent).toContain("عقد إيجار");
    expect(list.querySelector("bdi")).toBeTruthy();

    fireEvent.click(within(dialog).getByRole("radio", { name: "By file size" }));
    expect(within(dialog).getByLabelText("Largest file, in MB")).toBeTruthy();

    fireEvent.click(within(dialog).getByRole("radio", { name: "At top-level bookmarks" }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Choose a folder…" }));
    });
    expect(useAppStore.getState().splitPrompt).toBe(false);
    expect(useAppStore.getState().notice).toEqual({ key: "split", values: { count: "3" } });
  });
});

describe("inserting from a password-protected file", () => {
  it("asks for its password in the page area, then inserts and selects the pages", async () => {
    setMockInsertIsLocked(true);
    try {
      render(<App />);
      const before = (await backend.pageSizes(43)).length;
      act(() =>
        useAppStore.getState().addDocument({ id: 43, name: "merge.pdf", pageCount: before }),
      );
      await act(async () => {
        await runCommand("pages.insertFromFile");
      });
      expect(await screen.findByText("Enter the password to insert pages from")).toBeTruthy();
      fireEvent.change(screen.getByLabelText("Password"), { target: { value: "wrong" } });
      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "Insert" }));
      });
      expect(await screen.findByText("That password is not correct. Try again.")).toBeTruthy();
      expect(useAppStore.getState().unlocks[0]?.insert?.documentId).toBe(43);
      fireEvent.change(screen.getByLabelText("Password"), { target: { value: "1234" } });
      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "Insert" }));
      });
      expect(useAppStore.getState().unlocks).toHaveLength(0);
      const merged = selectActiveDocument(useAppStore.getState());
      expect(merged?.pageCount).toBe(before + 3);
      expect(merged?.selectedPages).toEqual([before, before + 1, before + 2]);
    } finally {
      setMockInsertIsLocked(false);
    }
  });
});
