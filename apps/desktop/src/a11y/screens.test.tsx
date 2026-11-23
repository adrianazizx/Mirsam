import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setMockRecoveries } from "../backend/mock";
import { type CommandId, isCommandEnabled, runCommand } from "../commands/registry";
import { type ToolId, useAppStore } from "../state/appStore";
import { useCertSign } from "../state/certSign";
import { axeViolations } from "./axe";

/**
 * axe-core over the main screens: the empty window, a document in the page
 * view, the side panels, every tool's panel, and the dialogs, in English (LTR)
 * and Arabic (RTL). The OCR tool is left out while other work rebuilds it;
 * it joins this list with it.
 *
 * The theme does not change what jsdom can check (it computes no colours), so
 * every screen runs in English light and Arabic dark, and the main ones in the
 * other two combinations; Chrome checks all four with contrast
 * (`scripts/a11y-check.mjs`).
 */

const PANELS: CommandId[] = [
  "view.showThumbnails",
  "view.showOutline",
  "view.showComments",
  "view.showSignatures",
];
const TOOLS: ToolId[] = ["edit", "comment", "pages", "forms", "sign", "protect"];
const DIALOGS: CommandId[] = [
  "app.settings",
  "app.commandPalette",
  "pages.split",
  "security.open",
  "certSign.open",
  "signature.new",
  "pages.watermark",
  "file.compress",
  "file.exportImages",
];

/** Lets pending promises (mock backend calls) settle and React render. */
async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

async function run(command: CommandId) {
  expect(isCommandEnabled(command), command).toBe(true);
  await act(async () => {
    await runCommand(command);
  });
  await settle();
}

function closeEverything() {
  act(() => {
    useAppStore.setState({
      settingsOpen: false,
      paletteOpen: false,
      splitPrompt: false,
      securityPrompt: false,
      outputDialog: null,
      galleryOpen: false,
      signatureDialog: null,
    });
  });
}

async function expectClean(screenName: string) {
  const violations = await axeViolations();
  expect(violations, screenName).toEqual([]);
}

beforeEach(() => {
  localStorage.clear();
  setMockRecoveries([]);
  useAppStore.setState({
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    unlocks: [],
    recoveries: [],
    leftPanel: null,
    activeTool: "select",
    galleryOpen: false,
    paletteOpen: false,
    settingsOpen: false,
  });
});

async function openApp(locale: "en" | "ar", theme: "light" | "dark") {
  render(<App />);
  act(() => {
    useAppStore.getState().setLocale(locale);
    useAppStore.getState().setTheme(theme);
  });
  await settle();
  expect(document.documentElement.lang).toBe(locale);
  expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");
}

describe("the axe check itself", () => {
  it("catches what it is there to catch", async () => {
    const host = document.createElement("div");
    // A nameless icon button and a tab with a button inside it.
    host.innerHTML =
      '<button><svg aria-hidden="true"></svg></button><div role="tablist"><div role="tab" tabindex="0" aria-selected="true">a.pdf<button>x</button></div></div>';
    document.body.append(host);
    try {
      const violations = await axeViolations(host);
      expect(violations.some((line) => line.startsWith("button-name"))).toBe(true);
      expect(violations.some((line) => line.startsWith("nested-interactive"))).toBe(true);
    } finally {
      host.remove();
    }
  });
});

describe.each([
  ["en", "light"],
  ["ar", "dark"],
] as const)("every main screen passes axe (%s, %s)", (locale, theme) => {
  it("the empty window and a document in the page view", async () => {
    await openApp(locale, theme);
    await expectClean("empty window");
    await run("file.open");
    expect(screen.getByTestId("viewer")).toBeTruthy();
    await expectClean("page view");
  });

  it("the side panels", async () => {
    await openApp(locale, theme);
    await run("file.open");
    for (const command of PANELS) {
      await run(command);
      await expectClean(command);
    }
  });

  it("the search panel: results, the replace row, and the replace preview (ADR 0052)", async () => {
    await openApp(locale, theme);
    await run("file.open");
    await run("search.replaceOpen");
    await act(async () => {
      await runCommand("search.run", { query: "4,500" });
      await runCommand("search.setReplacement", { text: "4,750" });
    });
    await settle();
    expect(screen.getByTestId("search-results")).toBeTruthy();
    await expectClean("search results and the replace row");
    await run("search.replaceAll");
    expect(screen.getByTestId("replace-preview")).toBeTruthy();
    await expectClean("replace preview");
    await run("search.replaceCancel");
  });

  it("each tool's panel", async () => {
    await openApp(locale, theme);
    await run("file.open");
    for (const tool of TOOLS) {
      act(() => useAppStore.getState().setActiveTool(tool));
      await settle();
      await expectClean(`tool ${tool}`);
    }
  });

  it("the dialogs", async () => {
    await openApp(locale, theme);
    await run("file.open");
    for (const command of DIALOGS) {
      await run(command);
      expect(document.querySelector('[role="dialog"]'), command).not.toBeNull();
      await expectClean(command);
      closeEverything();
      await act(async () => {
        if (useCertSign.getState().dialogOpen) await runCommand("certSign.cancel");
      });
      await settle();
    }
  });
});

describe.each([
  ["en", "dark"],
  ["ar", "light"],
] as const)("the main screens pass axe (%s, %s)", (locale, theme) => {
  it("empty window, page view, thumbnails, settings", async () => {
    await openApp(locale, theme);
    await expectClean("empty window");
    await run("file.open");
    await expectClean("page view");
    await run("view.showThumbnails");
    await expectClean("thumbnails");
    await run("app.settings");
    await expectClean("settings");
  });
});
