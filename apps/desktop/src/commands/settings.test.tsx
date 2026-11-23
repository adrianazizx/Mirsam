import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { parseSettings, useAppStore } from "../state/appStore";
import { runCommand } from "./registry";

const app = () => useAppStore.getState();

beforeEach(() => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    uiDigits: "arabicIndic",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    activeTool: "select",
    settingsOpen: false,
  });
});

describe("digits in the Arabic interface (owner decision, 2026-11-23)", () => {
  it("are Arabic-Indic by default, Western when chosen in Settings, and kept", async () => {
    render(<App />);
    act(() => {
      app().setLocale("ar");
      app().addDocument({ id: 1, name: "عقد.pdf", pageCount: 240 });
    });
    const bar = await screen.findByTestId("status-bar");
    expect(bar.textContent).toContain("٢٤٠");
    expect(bar.textContent).not.toContain("240");

    await act(() => runCommand("app.settings"));
    const settings = await screen.findByTestId("settings");
    const group = within(settings).getByRole("radiogroup", { name: "الأرقام في الواجهة العربية" });
    expect(within(group).getByRole("radio", { name: "هندية (١٢٣)" })).toHaveProperty(
      "checked",
      true,
    );
    act(() => {
      fireEvent.click(within(group).getByRole("radio", { name: "غربية (123)" }));
    });
    expect(app().uiDigits).toBe("western");
    expect(screen.getByTestId("status-bar").textContent).toContain("240");
    expect(screen.getByTestId("status-bar").textContent).not.toMatch(/[٠-٩]/);

    // Kept with the other settings, and back to Arabic-Indic by command.
    const cached = JSON.parse(localStorage.getItem("safha.preferences") ?? "{}");
    expect(parseSettings(cached).uiDigits).toBe("western");
    await act(() => runCommand("app.digitsArabicIndic"));
    expect(screen.getByTestId("status-bar").textContent).toContain("٢٤٠");
  });

  it("leave the English interface's numbers alone", async () => {
    render(<App />);
    act(() => app().addDocument({ id: 1, name: "a.pdf", pageCount: 240 }));
    expect((await screen.findByTestId("status-bar")).textContent).toContain("240");
    await act(() => runCommand("app.digitsArabicIndic"));
    expect(screen.getByTestId("status-bar").textContent).toContain("240");
  });

  it("ignore an unknown stored value", () => {
    expect(parseSettings({ uiDigits: "roman" }).uiDigits).toBeUndefined();
    expect(parseSettings({ uiDigits: "western" }).uiDigits).toBe("western");
  });
});
