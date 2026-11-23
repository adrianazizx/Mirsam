import { act, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setMockRecoveries } from "../backend/mock";
import { type CommandId, isCommandEnabled, runCommand } from "../commands/registry";
import { useAnnouncer } from "../state/announcer";
import { parseSettings, selectActiveDocument, useAppStore } from "../state/appStore";

/**
 * What screen readers are told without looking (WCAG 4.1.3), names that carry
 * their state and language (4.1.2, 3.1.2), and the interface size (1.4.4).
 */

async function settle(ms = 40) {
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function run(command: CommandId, arg?: unknown) {
  await act(async () => {
    await runCommand(command, arg as never);
  });
  await settle();
}

const polite = () => screen.getByTestId("live-polite").textContent;
const assertive = () => screen.getByTestId("live-assertive").textContent;

/** Marks a document as having unsaved changes (an edit would). */
function markModified(id: number) {
  useAppStore.setState((state) => ({
    documents: state.documents.map((open) => (open.id === id ? { ...open, modified: true } : open)),
  }));
}

beforeEach(() => {
  useAnnouncer.setState({ polite: { text: "", seq: 0 }, assertive: { text: "", seq: 0 } });
  localStorage.clear();
  setMockRecoveries([]);
  useAppStore.setState({
    locale: "en",
    uiScale: 1,
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    unlocks: [],
    leftPanel: null,
    activeTool: "select",
    paletteOpen: false,
    settingsOpen: false,
  });
});

describe("live regions", () => {
  it("are in the window from the start, kept audible over dialogs", async () => {
    render(<App />);
    const regions = screen.getByTestId("live-polite").parentElement;
    expect(regions?.hasAttribute("data-live-announcer")).toBe(true);
    expect(screen.getByTestId("live-polite").getAttribute("role")).toBe("status");
    expect(screen.getByTestId("live-assertive").getAttribute("role")).toBe("alert");
    expect(polite()).toBe("");
  });

  it("say errors at once, and notices politely", async () => {
    render(<App />);
    act(() => useAppStore.getState().setError("invalid_pdf"));
    expect(assertive()).toContain("could not be read");
    act(() => useAppStore.getState().setNotice({ key: "split", values: { count: "3" } }));
    expect(polite()).toBe("The document was split into 3 files.");
  });

  it("say a save, in the interface's language", async () => {
    render(<App />);
    await run("file.open");
    const id = useAppStore.getState().activeDocumentId ?? 0;
    act(() => {
      useAppStore.getState().setLocale("ar");
      markModified(id);
    });
    expect(isCommandEnabled("file.save")).toBe(true);
    await run("file.save");
    const name = selectActiveDocument(useAppStore.getState())?.name ?? "";
    expect(polite()).toBe(`حُفظ ${name}.`);
  });

  it("say the page and zoom a command moved to, once they settle", async () => {
    render(<App />);
    await run("file.open");
    const id = useAppStore.getState().activeDocumentId ?? 0;
    // A fitted zoom following the window is not said.
    act(() => useAppStore.getState().updateDocument(id, { zoomMode: "fit-width", zoom: 0.8 }));
    act(() => useAppStore.getState().updateDocument(id, { zoom: 0.7 }));
    await settle(800);
    act(() => useAppStore.getState().updateDocument(id, { page: 4 }));
    act(() =>
      useAppStore.getState().updateDocument(id, { page: 5, zoom: 1.5, zoomMode: "custom" }),
    );
    expect(polite()).not.toContain("Page");
    await settle(800);
    expect(polite()).toBe("Page 6 of 240, Zoom 150%");
  });
});

describe("names", () => {
  it("a tab with unsaved changes says so in its name", async () => {
    render(<App />);
    act(() => {
      useAppStore.getState().addDocument({ id: 4, name: "عقد.pdf", pageCount: 2 });
      markModified(4);
    });
    const tab = await screen.findByRole("tab", { name: /^عقد\.pdf\s*, Unsaved changes$/ });
    expect(tab).toBeTruthy();
    // The close mark is the pointer's: hidden from screen readers, not a control in the tab.
    expect(within(tab).queryByRole("button")).toBeNull();
  });

  it("the language switch is named in its own language", async () => {
    render(<App />);
    const toArabic = screen.getByRole("button", { name: "العربية" });
    expect(toArabic.querySelector('[lang="ar"]')?.textContent).toBe("العربية");
    act(() => useAppStore.getState().setLocale("ar"));
    const toEnglish = await screen.findByRole("button", { name: "English" });
    expect(toEnglish.querySelector('[lang="en"]')?.textContent).toBe("English");
  });
});

describe("interface size", () => {
  it("is a stored setting that scales the interface from the root", async () => {
    expect(parseSettings({ uiScale: "1.5" }).uiScale).toBe(1.5);
    expect(parseSettings({ uiScale: "3" }).uiScale).toBeUndefined();
    render(<App />);
    expect(document.documentElement.dataset.uiScale).toBe("1");
    expect(isCommandEnabled("app.uiSmaller")).toBe(false);
    await run("app.uiLarger");
    await run("app.uiLarger");
    expect(useAppStore.getState().uiScale).toBe(1.5);
    expect(document.documentElement.dataset.uiScale).toBe("1.5");
    expect(polite()).toBe("Interface size 150%");
    await run("app.uiDefaultSize");
    expect(document.documentElement.dataset.uiScale).toBe("1");
  });

  it("is chosen in Settings", async () => {
    render(<App />);
    await run("app.settings");
    const group = screen.getByRole("radiogroup", { name: "Interface size" });
    act(() => within(group).getByRole("radio", { name: "200%" }).click());
    expect(useAppStore.getState().uiScale).toBe(2);
  });
});
