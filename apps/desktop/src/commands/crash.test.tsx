import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { axeViolations } from "../a11y/axe";
import { backend } from "../backend";
import {
  recordedUiErrors,
  SAMPLE_CRASH_TEXT,
  sampleCrashReport,
  setMockCrashReports,
} from "../backend/mockCrash";
import { CrashBoundary } from "../components/shell/CrashBoundary";
import { applyLocale } from "../i18n";
import { useAppStore } from "../state/appStore";
import {
  captureUncaughtErrors,
  recentCommands,
  reportUiError,
  resetCrashCapture,
  useCrashReports,
} from "../state/crashReports";
import { runCommand } from "./registry";

/** A component that fails to render. */
function Broken(): never {
  throw new Error("render failed");
}

/** Clipboard writes through the `copy` event, as in the app. */
function captureClipboard() {
  const copied: string[] = [];
  const exec = vi.fn((command: string) => {
    if (command !== "copy") return false;
    const data = new Map<string, string>();
    const event = new Event("copy", { cancelable: true }) as ClipboardEvent;
    Object.defineProperty(event, "clipboardData", {
      value: { setData: (type: string, text: string) => data.set(type, text) },
    });
    document.dispatchEvent(event);
    copied.push(data.get("text/plain") ?? "");
    return true;
  });
  Object.defineProperty(document, "execCommand", { value: exec, configurable: true });
  return copied;
}

beforeEach(() => {
  localStorage.clear();
  resetCrashCapture();
  setMockCrashReports([]);
  useCrashReports.setState({ prompt: [], kept: [], github: null, setting: "ask", status: {} });
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    settingsOpen: false,
  });
  applyLocale("en");
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("crash reports (ADR 0061)", () => {
  it("asks once after a crash, showing exactly what would be sent", async () => {
    setMockCrashReports([sampleCrashReport()]);
    render(<App />);
    const dialog = await screen.findByTestId("crash-reports");
    expect(within(dialog).getByText("Send a crash report?")).toBeTruthy();
    // The first report's text is shown as it is, left to right.
    const text = within(dialog).getByTestId("crash-report-text");
    expect(text.textContent).toBe(SAMPLE_CRASH_TEXT);
    expect(text.querySelector("pre")?.getAttribute("dir")).toBe("ltr");
    // WCAG 2.1 AA (ADR 0070): the prompt, with its report open.
    expect(await axeViolations()).toEqual([]);

    // Copy puts exactly that text on the clipboard.
    const copied = captureClipboard();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Copy" }));
    });
    expect(copied).toEqual([SAMPLE_CRASH_TEXT]);
    expect(within(dialog).getByRole("status").textContent).toBe("Copied to the clipboard.");

    // Not now closes it; the report stays kept, and is not offered again.
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Not now" }));
    });
    expect(screen.queryByTestId("crash-reports")).toBeNull();
    expect((await backend.crashReports()).reports).toHaveLength(1);
    expect(await backend.newCrashReports()).toEqual([]);
  });

  it("deletes a report, files an issue after copying it, and can be turned off", async () => {
    setMockCrashReports([sampleCrashReport("1-a"), sampleCrashReport("2-b")]);
    render(<App />);
    const dialog = await screen.findByTestId("crash-reports");
    expect(within(dialog).getAllByTestId("crash-report")).toHaveLength(2);

    const first = within(dialog).getAllByTestId("crash-report")[0] as HTMLElement;
    await act(async () => {
      fireEvent.click(within(first).getByRole("button", { name: "Delete" }));
    });
    expect(within(dialog).getAllByTestId("crash-report")).toHaveLength(1);
    expect((await backend.crashReports()).reports.map((report) => report.id)).toEqual(["2-b"]);

    const copied = captureClipboard();
    const issue = vi.spyOn(backend, "openCrashIssue");
    await act(() => runCommand("crash.openIssue", { reportId: "2-b" }));
    expect(copied).toHaveLength(1);
    expect(issue).toHaveBeenCalledWith("2-b");

    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Don't keep crash reports" }));
    });
    expect(screen.queryByTestId("crash-reports")).toBeNull();
    expect(useCrashReports.getState().setting).toBe("off");
    expect((await backend.settings()).crashReports).toBe("off");
  });

  it("lists kept reports in Settings, in Arabic too", async () => {
    setMockCrashReports([sampleCrashReport()]);
    await backend.newCrashReports(); // already offered
    render(<App />);
    act(() => useAppStore.getState().setLocale("ar"));
    await act(() => runCommand("app.settings"));
    const settings = await screen.findByTestId("settings");
    const group = await within(settings).findByRole("radiogroup", {
      name: "عندما يتوقف مرسام على نحو غير متوقّع",
    });
    expect(within(group).getByRole("radio", { name: "اسألني بعد حدوث عطل" })).toHaveProperty(
      "checked",
      true,
    );
    const card = await within(settings).findByTestId("crash-report");
    expect(within(card).getByText("توقّف التطبيق")).toBeTruthy();
    // Hidden until asked for, then shown as sent: English, left to right.
    expect(within(card).queryByTestId("crash-report-text")).toBeNull();
    fireEvent.click(within(card).getByRole("button", { name: "عرض التقرير" }));
    expect(within(card).getByTestId("crash-report-text").textContent).toBe(SAMPLE_CRASH_TEXT);
    expect(await axeViolations()).toEqual([]);
  });

  it("records the ids of commands run, never their arguments", async () => {
    await runCommand("app.toggleTheme");
    await runCommand("app.toggleTheme");
    await runCommand("view.goToPage", { page: 3 });
    expect(recentCommands()).toEqual(["app.toggleTheme ×2"]);
    await runCommand("app.settings");
    expect(recentCommands()).toEqual(["app.toggleTheme ×2", "app.settings"]);
  });

  it("keeps errors nothing handled, without turning thrown objects into text", async () => {
    const stop = captureUncaughtErrors();
    await runCommand("app.settings");
    await reportUiError(new TypeError("x is undefined"));
    // The same error again is not a second report; a benign one is none.
    await reportUiError(new TypeError("x is undefined"));
    await reportUiError(new Error("ResizeObserver loop completed with undelivered notifications"));
    await reportUiError({ secret: "document text" });
    window.dispatchEvent(new ErrorEvent("error", { error: new RangeError("bad page") }));
    await Promise.resolve();
    stop();
    const recorded = recordedUiErrors();
    expect(recorded.map((error) => error.message)).toEqual([
      "TypeError: x is undefined",
      "thrown value of type object",
      "RangeError: bad page",
    ]);
    expect(recorded[0]?.commands).toEqual(["app.settings"]);
    expect(JSON.stringify(recorded)).not.toContain("document text");
  });

  it("shows a way back when the window fails to render, and keeps a report", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    render(
      <CrashBoundary>
        <Broken />
      </CrashBoundary>,
    );
    const fallback = await screen.findByTestId("crash-fallback");
    expect(within(fallback).getByRole("button", { name: "Reload window" })).toBeTruthy();
    await Promise.resolve();
    expect(recordedUiErrors().at(-1)?.message).toBe("Error: render failed");
    expect(recordedUiErrors().at(-1)?.stack).toContain("Broken");
  });
});
