import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { App } from "../../App";
import { axeViolations } from "../../a11y/axe";
import { press } from "../../a11y/keyboard";
import { setMockRecoveries } from "../../backend/mock";
import { runCommand } from "../../commands/registry";
import { useAnnouncer } from "../../state/announcer";
import { useAppStore } from "../../state/appStore";
import {
  GUIDE_SETTING,
  guideDue,
  HINTS_SETTING,
  hintDue,
  parseHints,
  setOnboardingAutomatic,
  useOnboarding,
} from "../../state/onboarding";

/** What the mock backend stored (it keeps settings in localStorage). */
function stored(): Record<string, string> {
  return JSON.parse(localStorage.getItem("safha.mock.settings") ?? "{}") as Record<string, string>;
}

async function settle() {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

function resetOnboarding() {
  useOnboarding.setState({
    loaded: false,
    guide: "new",
    hintsSeen: [],
    guideOpen: false,
    step: 0,
    hint: null,
  });
}

beforeEach(() => {
  localStorage.clear();
  setMockRecoveries([]);
  setOnboardingAutomatic(true);
  resetOnboarding();
  useAppStore.setState({
    locale: "en",
    uiDigits: "arabicIndic",
    theme: "light",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    unlocks: [],
    recoveries: [],
    leftPanel: null,
    activeTool: "select",
    galleryOpen: false,
    paletteOpen: false,
    settingsOpen: false,
  });
});

afterEach(() => {
  setOnboardingAutomatic(false);
});

async function startApp() {
  render(<App />);
  await settle();
}

async function click(element: HTMLElement) {
  act(() => {
    fireEvent.click(element);
  });
  await settle();
}

describe("the first-run guide", () => {
  it("opens by itself on a first empty launch, walks through by keyboard, and stays done", async () => {
    await startApp();
    const guide = await screen.findByRole("dialog", { name: /Welcome to Mirsam/ });
    expect(within(guide).getByText("Step 1 of 4")).toBeTruthy();
    expect(within(guide).getByRole("heading", { name: "Open a PDF" })).toBeTruthy();
    expect(await axeViolations()).toEqual([]);

    // Focus starts inside; Enter on "Next" walks every step, focus staying put.
    const next = within(guide).getByRole("button", { name: "Next" });
    act(() => next.focus());
    for (const [n, title] of [
      [2, "The tool rail"],
      [3, "Arabic and English"],
      [4, "OCR stays on this computer"],
    ] as const) {
      press("Enter");
      await settle();
      expect(within(guide).getByText(`Step ${n} of 4`)).toBeTruthy();
      expect(within(guide).getByRole("heading", { name: title })).toBeTruthy();
      expect(useAnnouncer.getState().polite.text).toBe(`Step ${n} of 4: ${title}`);
      expect(guide.contains(document.activeElement)).toBe(true);
      expect(await axeViolations(), title).toEqual([]);
    }
    // The tool rail step lists the rail's tools in its order.
    await click(within(guide).getByRole("button", { name: "Back" }));
    await click(within(guide).getByRole("button", { name: "Back" }));
    expect(within(guide).getByText("OCR & Convert")).toBeTruthy();
    await click(within(guide).getByRole("button", { name: "Next" }));
    await click(within(guide).getByRole("button", { name: "Next" }));

    // The OCR step says the models download once and documents stay here.
    expect(within(guide).getByText(/download its recognition models/)).toBeTruthy();
    expect(within(guide).getByText("Your documents never leave this computer.")).toBeTruthy();
    expect(within(guide).queryByRole("button", { name: "Skip" })).toBeNull();
    await click(within(guide).getByRole("button", { name: "Done" }));
    expect(screen.queryByTestId("onboarding-guide")).toBeNull();
    expect(stored()[GUIDE_SETTING]).toBe("done");
  });

  it("is not shown again once finished or skipped", async () => {
    localStorage.setItem("safha.mock.settings", JSON.stringify({ [GUIDE_SETTING]: "skipped" }));
    await startApp();
    expect(useOnboarding.getState().loaded).toBe(true);
    expect(screen.queryByTestId("onboarding-guide")).toBeNull();
  });

  it("Escape skips it for good; Help reopens it, and skipping a finished guide keeps it finished", async () => {
    await startApp();
    await screen.findByTestId("onboarding-guide");
    press("Escape");
    await settle();
    expect(screen.queryByTestId("onboarding-guide")).toBeNull();
    expect(stored()[GUIDE_SETTING]).toBe("skipped");

    await act(() => runCommand("help.guide"));
    const guide = await screen.findByTestId("onboarding-guide");
    expect(within(guide).getByText("Step 1 of 4")).toBeTruthy();
    await act(() => runCommand("onboarding.finish"));
    expect(stored()[GUIDE_SETTING]).toBe("done");
    await act(() => runCommand("help.guide"));
    await click(
      within(screen.getByTestId("onboarding-guide")).getByRole("button", { name: "Skip" }),
    );
    expect(stored()[GUIDE_SETTING]).toBe("done");
  });

  it("is reachable from the command palette", async () => {
    localStorage.setItem("safha.mock.settings", JSON.stringify({ [GUIDE_SETTING]: "done" }));
    await startApp();
    await act(() => runCommand("app.commandPalette"));
    const input = await screen.findByTestId("palette-input");
    act(() => {
      fireEvent.change(input, { target: { value: "Welcome guide" } });
    });
    await settle();
    await click(screen.getByRole("option", { name: /Welcome guide/ }));
    expect(await screen.findByTestId("onboarding-guide")).toBeTruthy();
  });

  it("works in Arabic, right to left, with the language and digits chosen in it", async () => {
    await startApp();
    act(() => useAppStore.getState().setLocale("ar"));
    const guide = await screen.findByRole("dialog", { name: /مرحبًا بك في مرسام/ });
    expect(document.documentElement.dir).toBe("rtl");
    expect(within(guide).getByText("الخطوة ١ من ٤")).toBeTruthy();
    expect(await axeViolations()).toEqual([]);
    await act(() => runCommand("onboarding.next"));
    await act(() => runCommand("onboarding.next"));
    expect(within(guide).getByRole("heading", { name: "العربية والإنجليزية" })).toBeTruthy();
    expect(await axeViolations()).toEqual([]);

    await click(within(guide).getByRole("radio", { name: /123/ }));
    expect(useAppStore.getState().uiDigits).toBe("western");
    expect(within(guide).getByText("الخطوة 3 من 4")).toBeTruthy();
    await click(within(guide).getByRole("radio", { name: "English" }));
    expect(document.documentElement.dir).toBe("ltr");
    expect(within(guide).getByText("Step 3 of 4")).toBeTruthy();
    expect(stored().locale).toBe("en");
  });

  it("waits when the window starts with a document or something else to answer", () => {
    const ready = { loaded: true, guide: "new" as const, guideOpen: false };
    expect(guideDue(ready, { documents: 0, busy: false })).toBe(true);
    expect(guideDue(ready, { documents: 1, busy: false })).toBe(false);
    expect(guideDue(ready, { documents: 0, busy: true })).toBe(false);
    expect(guideDue({ ...ready, loaded: false }, { documents: 0, busy: false })).toBe(false);
    expect(guideDue({ ...ready, guide: "done" }, { documents: 0, busy: false })).toBe(false);
    setOnboardingAutomatic(false);
    expect(guideDue(ready, { documents: 0, busy: false })).toBe(false);
  });

  it("does not open over recovered changes", async () => {
    setMockRecoveries([
      { token: 9, name: "draft.pdf", savedAt: 1_795_000_000, originalChanged: false },
    ] as never);
    await startApp();
    expect(useAppStore.getState().recoveries).toHaveLength(1);
    expect(screen.queryByTestId("onboarding-guide")).toBeNull();
    expect(stored()[GUIDE_SETTING]).toBeUndefined();
  });
});

describe("the tools' one-time hints", () => {
  async function withDocument() {
    localStorage.setItem("safha.mock.settings", JSON.stringify({ [GUIDE_SETTING]: "done" }));
    await startApp();
    await act(() => runCommand("file.open"));
    await settle();
    expect(useAppStore.getState().documents).toHaveLength(1);
  }

  it("explain OCR, Sign and Edit once each, the first time they are opened", async () => {
    await withDocument();
    for (const [tool, title] of [
      ["ocr", "Make scans searchable"],
      ["sign", "Fill in and sign"],
      ["edit", "Edit the page"],
    ] as const) {
      act(() => useAppStore.getState().setActiveTool(tool));
      const hint = await screen.findByTestId("tool-hint");
      expect(within(hint).getByRole("heading", { name: title })).toBeTruthy();
      // Announced, never focused.
      expect(useAnnouncer.getState().polite.text.startsWith(`${title}.`)).toBe(true);
      expect(hint.contains(document.activeElement)).toBe(false);
      if (tool !== "ocr") expect(await axeViolations(), tool).toEqual([]);
      await click(within(hint).getByRole("button", { name: "Got it" }));
      expect(screen.queryByTestId("tool-hint")).toBeNull();
      // Not again: another tool, then back.
      act(() => useAppStore.getState().setActiveTool("select"));
      act(() => useAppStore.getState().setActiveTool(tool));
      await settle();
      expect(screen.queryByTestId("tool-hint")).toBeNull();
    }
    expect(parseHints(stored()[HINTS_SETTING])).toEqual(["ocr", "sign", "edit"]);
  });

  it("go when another tool is chosen, and stay seen", async () => {
    await withDocument();
    act(() => useAppStore.getState().setActiveTool("sign"));
    expect(await screen.findByTestId("tool-hint")).toBeTruthy();
    act(() => useAppStore.getState().setActiveTool("comment"));
    await settle();
    expect(screen.queryByTestId("tool-hint")).toBeNull();
    expect(stored()[HINTS_SETTING]).toBe("sign");
  });

  it("wait for a document, and for the guide to close", () => {
    const state = { loaded: true, hintsSeen: [], guideOpen: false };
    expect(hintDue("ocr", true, state)).toBe("ocr");
    expect(hintDue("ocr", false, state)).toBeNull();
    expect(hintDue("ocr", true, { ...state, guideOpen: true })).toBeNull();
    expect(hintDue("ocr", true, { ...state, loaded: false })).toBeNull();
    expect(hintDue("ocr", true, { ...state, hintsSeen: ["ocr"] })).toBeNull();
    expect(hintDue("comment", true, state)).toBeNull();
  });

  it("read in Arabic", async () => {
    await withDocument();
    act(() => useAppStore.getState().setLocale("ar"));
    act(() => useAppStore.getState().setActiveTool("edit"));
    const hint = await screen.findByTestId("tool-hint");
    expect(within(hint).getByRole("heading", { name: "حرّر الصفحة" })).toBeTruthy();
    expect(within(hint).getByRole("button", { name: "حسنًا" })).toBeTruthy();
    expect(await axeViolations()).toEqual([]);
  });
});

describe("the start screen", () => {
  it("offers first steps, each a command", async () => {
    localStorage.setItem("safha.mock.settings", JSON.stringify({ [GUIDE_SETTING]: "done" }));
    await startApp();
    const steps = screen.getByRole("region", { name: "Get started" });
    expect(await axeViolations()).toEqual([]);

    await click(within(steps).getByRole("button", { name: /^Take the tour/ }));
    expect(await screen.findByTestId("onboarding-guide")).toBeTruthy();
    await act(() => runCommand("onboarding.skip"));

    const language = within(steps).getByRole("button", { name: /^العربية/ });
    expect(language.querySelector('[lang="ar"]')?.textContent).toBe("العربية");
    await click(language);
    expect(document.documentElement.dir).toBe("rtl");
    const arabic = screen.getByRole("region", { name: "ابدأ" });
    expect(within(arabic).getByRole("button", { name: /^English/ })).toBeTruthy();
    expect(await axeViolations()).toEqual([]);

    await click(within(arabic).getByRole("button", { name: /^الإعدادات/ }));
    expect(useAppStore.getState().settingsOpen).toBe(true);
  });
});
