import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setMockRecoveries } from "../backend/mock";
import { REGIONS } from "../commands/regions";
import { type CommandId, runCommand } from "../commands/registry";
import { useAppStore } from "../state/appStore";
import { press, pressTab, regionOf, tabbables } from "./keyboard";

/**
 * Keyboard flows (WCAG 2.1.1, 2.1.2, 2.4.3, 2.4.7): Tab and Shift+Tab reach
 * every area in a fixed order and never get stuck; F6 moves between areas in
 * the order they are seen; arrow keys move within toolbars, the tab bar, the
 * thumbnails and the page grid, mirrored in Arabic; Escape closes dialogs and
 * popovers and gives focus back.
 */

/** Lets the mock backend answer and React Aria finish (it gives focus back on an animation frame). */
async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 40));
  });
}

async function run(command: CommandId, arg?: unknown) {
  await act(async () => {
    await runCommand(command, arg as never);
  });
  await settle();
}

async function openApp(locale: "en" | "ar") {
  render(<App />);
  act(() => useAppStore.getState().setLocale(locale));
  await settle();
  await run("file.open");
}

beforeEach(() => {
  localStorage.clear();
  setMockRecoveries([]);
  (document.activeElement as HTMLElement | null)?.blur();
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

/**
 * Presses Tab (or Shift+Tab) until focus comes back to where the round began,
 * and lists the areas it passed through, in order, merging repeats. Fails if
 * focus never comes round: something kept it.
 */
function areasInOneRound(shift = false): string[] {
  const first = pressTab(shift);
  const visited = [regionOf(first) ?? "outside"];
  for (let i = 0; i < 200; i += 1) {
    const element = pressTab(shift);
    if (element === first) return visited;
    const region = regionOf(element) ?? "outside";
    if (visited.at(-1) !== region) visited.push(region);
  }
  throw new Error(`focus did not come round: ${visited.join(", ")}`);
}

describe.each(["en", "ar"] as const)("keyboard (%s)", (locale) => {
  const forward = locale === "ar" ? "ArrowLeft" : "ArrowRight";
  const backward = locale === "ar" ? "ArrowRight" : "ArrowLeft";

  it("Tab reaches every area once and comes round again; Shift+Tab goes the other way", async () => {
    await openApp(locale);
    await run("view.showThumbnails");
    expect(tabbables().length).toBeGreaterThan(10);
    // The document first after its tab, then the tools in the order they are seen.
    // Toolbars and radio groups hand Tab on rather than keeping it: no area is a trap.
    const order = ["tabs", "document", "tools", "panel", "status"];
    expect(areasInOneRound()).toEqual(order);
    // Shift+Tab from the tab bar: the same areas, the other way round.
    expect(areasInOneRound(true)).toEqual([...order].reverse());
  });

  it("F6 and Shift+F6 move between the areas in the order they are seen", async () => {
    await openApp(locale);
    await run("view.showThumbnails");
    const seen: (string | null)[] = [];
    for (let i = 0; i < REGIONS.length + 1; i += 1) {
      press("F6");
      seen.push(regionOf(document.activeElement));
    }
    expect(seen).toEqual([...REGIONS, "tabs"]);
    // Each area's own place: the selected tab, the active tool, the page area.
    expect(document.activeElement?.getAttribute("role")).toBe("tab");
    expect(document.activeElement?.getAttribute("aria-selected")).toBe("true");
    press("F6");
    expect(document.activeElement?.getAttribute("aria-checked")).toBe("true");
    press("F6");
    press("F6");
    expect(document.activeElement).toBe(screen.getByTestId("viewer"));
    // Backwards, from the page area: the side panel, the tools, the tabs, round to the status bar.
    const back: (string | null)[] = [];
    for (let i = 0; i < 4; i += 1) {
      press("F6", { shiftKey: true });
      back.push(regionOf(document.activeElement));
    }
    expect(back).toEqual(["panel", "tools", "tabs", "status"]);
  });

  it("arrow keys move along the tool rail", async () => {
    await openApp(locale);
    const rail = screen.getByTestId("tool-rail");
    const tools = within(rail).getAllByRole("radio");
    act(() => tools[0]?.focus());
    press("ArrowDown");
    expect(document.activeElement).toBe(tools[1]);
    press("ArrowUp");
    expect(document.activeElement).toBe(tools[0]);
  });

  it("arrow keys move between document tabs, mirrored in Arabic", async () => {
    await openApp(locale);
    act(() => {
      useAppStore.getState().addDocument({ id: 91, name: "b.pdf", pageCount: 2 });
      useAppStore.getState().addDocument({ id: 92, name: "ج.pdf", pageCount: 2 });
      useAppStore.getState().activateDocument(useAppStore.getState().documents[0]?.id ?? 0);
    });
    await settle();
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(3);
    act(() => tabs[0]?.focus());
    press(forward);
    expect(document.activeElement).toBe(tabs[1]);
    press(forward);
    expect(document.activeElement).toBe(tabs[2]);
    press(backward);
    expect(document.activeElement).toBe(tabs[1]);
  });

  it("arrow keys move along the page organizer's toolbar, mirrored in Arabic", async () => {
    await openApp(locale);
    act(() => useAppStore.getState().setActiveTool("pages"));
    await settle();
    const toolbar = screen.getByRole("toolbar");
    const buttons = within(toolbar).getAllByRole("button");
    act(() => buttons[0]?.focus());
    press(forward);
    expect(document.activeElement).toBe(buttons[1]);
    press(backward);
    expect(document.activeElement).toBe(buttons[0]);
  });

  it("the thumbnails are one tab stop; arrows, Page keys, Home and End move within", async () => {
    await openApp(locale);
    await run("view.showThumbnails");
    const strip = screen.getByTestId("thumbnails");
    // jsdom lays nothing out: give the strip a height and let it mount its thumbnails.
    Object.defineProperty(strip, "clientHeight", { configurable: true, value: 600 });
    act(() => {
      strip.dispatchEvent(new Event("scroll"));
    });
    const items = () => [...strip.querySelectorAll<HTMLElement>("button[data-page]")];
    expect(items().length).toBeGreaterThan(1);
    expect(items().filter((item) => item.tabIndex === 0)).toHaveLength(1);
    const current = items().find((item) => item.tabIndex === 0);
    expect(current?.getAttribute("aria-current")).toBe("page");
    act(() => current?.focus());
    press("ArrowDown");
    expect(document.activeElement?.getAttribute("data-page")).toBe("1");
    press("End");
    const last = String((useAppStore.getState().documents[0]?.pageCount ?? 0) - 1);
    expect(document.activeElement?.getAttribute("data-page")).toBe(last);
    press("Home");
    expect(document.activeElement?.getAttribute("data-page")).toBe("0");
    press("PageDown");
    expect(Number(document.activeElement?.getAttribute("data-page"))).toBeGreaterThan(0);
    // Still one way in from Tab: the thumbnail last focused.
    expect(items().filter((item) => item.tabIndex === 0)).toEqual([document.activeElement]);
  });

  it("Escape closes the command palette and gives focus back", async () => {
    await openApp(locale);
    const viewer = screen.getByTestId("viewer");
    act(() => viewer.focus());
    press("k", { metaKey: true, ctrlKey: false });
    if (!useAppStore.getState().paletteOpen) press("k", { ctrlKey: true });
    await settle();
    expect(useAppStore.getState().paletteOpen).toBe(true);
    expect(document.activeElement).toBe(screen.getByTestId("palette-input"));
    press("Escape");
    await settle();
    expect(useAppStore.getState().paletteOpen).toBe(false);
    expect(document.activeElement).toBe(viewer);
  });

  it("Escape closes settings and a dropdown, and gives focus back", async () => {
    await openApp(locale);
    const viewMode = within(screen.getByTestId("view-mode")).getByRole("button");
    act(() => viewMode.focus());
    press("ArrowDown");
    await settle();
    expect(screen.getByRole("listbox")).toBeTruthy();
    press("Escape");
    await settle();
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(document.activeElement).toBe(viewMode);

    await run("app.settings");
    expect(screen.getByTestId("settings")).toBeTruthy();
    press("Escape");
    await settle();
    expect(screen.queryByTestId("settings")).toBeNull();
  });

  it("Escape backs out of the password prompt", async () => {
    render(<App />);
    act(() => {
      useAppStore.getState().setLocale(locale);
      useAppStore.setState({ unlocks: [{ token: 5, name: "سري.pdf", retry: false }] });
    });
    await settle();
    const field = screen.getByTestId("unlock-password");
    expect(document.activeElement).toBe(field);
    act(() => {
      fireEvent.keyDown(field, { key: "Escape" });
    });
    await settle();
    expect(useAppStore.getState().unlocks).toEqual([]);
  });
});
