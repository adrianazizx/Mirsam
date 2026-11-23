import { act, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setMockRecoveries } from "../backend/mock";
import { sampleCrashReport, setMockCrashReports } from "../backend/mockCrash";
import { resetMockOcr } from "../backend/mockOcr";
import { resetMockOcrJobs } from "../backend/mockOcrJobs";
import { type CommandId, isCommandEnabled, runCommand } from "../commands/registry";
import { useAppStore } from "../state/appStore";
import { useCertSign } from "../state/certSign";
import { useOcrJobs } from "../state/ocrJobs";
import { useOcrModels } from "../state/ocrModels";
import { axeViolations } from "./axe";

/**
 * axe-core over the screens added after the first accessibility pass (ADR
 * 0070's second pass): the OCR tool (jobs, the review list, review mode's
 * card, an area read again, layer checks, editable text, the model prompt),
 * Settings' OCR models (the Tesseract note, a download, a delete being
 * confirmed) and crash reports, the certificate dialog's time-stamp field,
 * and the keyboard shortcuts list. English light and Arabic dark in full, the
 * main ones in the other two combinations; Chrome checks contrast
 * (`scripts/a11y-check.mjs`).
 */

const installed = { kind: "installed" } as const;

async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

async function run(command: CommandId, arg?: Parameters<typeof runCommand>[1]) {
  expect(isCommandEnabled(command), command).toBe(true);
  await act(async () => {
    await runCommand(command, arg);
  });
  await settle();
}

async function expectClean(screenName: string) {
  expect(await axeViolations(), screenName).toEqual([]);
}

beforeEach(() => {
  localStorage.clear();
  setMockRecoveries([]);
  setMockCrashReports([]);
  resetMockOcr({ states: { "nuqta-4b": installed, "llama-cpp": installed } });
  resetMockOcrJobs();
  useOcrModels.setState({ models: null, jobs: {}, failures: {}, prompt: false });
  useOcrJobs.setState({
    jobs: {},
    review: {},
    focus: null,
    reviewing: false,
    numbers: false,
    drawingRegion: false,
    region: null,
    regionLanguage: "auto",
    regionNumbers: false,
    checks: {},
    keepScan: true,
    family: "naskh",
  });
  useAppStore.setState({
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    activeTool: "select",
    leftPanel: null,
    settingsOpen: false,
    shortcutsOpen: false,
    paletteOpen: false,
  });
});

async function openApp(locale: "en" | "ar", theme: "light" | "dark") {
  render(<App />);
  act(() => {
    useAppStore.getState().setLocale(locale);
    useAppStore.getState().setTheme(theme);
  });
  await settle();
  expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");
}

/** A six-page scan with its first page recognised; resolves once the review list is in. */
async function recognised() {
  act(() => useAppStore.getState().addDocument({ id: 1, name: "scan.pdf", pageCount: 6 }));
  await settle();
  await run("ocr.recognizePage");
  const job = await screen.findByTestId("ocr-job");
  await waitFor(() => expect(job.textContent).toMatch(/Done\.|اكتمل\./));
  await screen.findByTestId("ocr-review");
}

describe.each([
  ["en", "light"],
  ["ar", "dark"],
] as const)("the OCR tool passes axe (%s, %s)", (locale, theme) => {
  it("the panel before and after a job, with the pages-needing-review list", async () => {
    await openApp(locale, theme);
    act(() => useAppStore.getState().addDocument({ id: 1, name: "scan.pdf", pageCount: 6 }));
    act(() => useAppStore.getState().setActiveTool("ocr"));
    await settle();
    expect(screen.getByTestId("ocr-panel")).toBeTruthy();
    await expectClean("OCR panel, nothing run");
    await recognised();
    await expectClean("OCR panel with a finished job and the review list");
  });

  it("review mode's card: a word, then nothing left", async () => {
    await openApp(locale, theme);
    await recognised();
    await run("ocr.startReview");
    const card = await screen.findByTestId("ocr-review-card");
    expect(within(card).getByTestId("ocr-review-text")).toBeTruthy();
    await expectClean("review card with a word");
    for (let i = 0; i < 3; i += 1) await run("ocr.reviewConfirm");
    await waitFor(() => expect(card.querySelector("input")).toBeNull());
    await expectClean("review card, nothing left");
  });

  it("an area read again, set from the keyboard", async () => {
    await openApp(locale, theme);
    await recognised();
    await run("ocr.drawRegion");
    await expectClean("drawing an area");
    await run("ocr.regionWholePage");
    expect(useOcrJobs.getState().region).not.toBeNull();
    expect(screen.getByTestId("ocr-area-width")).toBeTruthy();
    await expectClean("an area set, with its fields");
  });

  it("layer checks and editable text", async () => {
    await openApp(locale, theme);
    act(() => useAppStore.getState().addDocument({ id: 1, name: "scan.pdf", pageCount: 3 }));
    await settle();
    await run("ocr.checkLayers");
    const layers = await screen.findByTestId("ocr-layers");
    await waitFor(() => expect(layers.querySelector("ul li")).not.toBeNull());
    await expectClean("layer checks with a poor page");
  });

  it("the model prompt", async () => {
    resetMockOcr();
    await openApp(locale, theme);
    act(() => useAppStore.getState().addDocument({ id: 1, name: "scan.pdf", pageCount: 3 }));
    await settle();
    await run("ocr.recognize");
    expect(screen.getByTestId("ocr-prompt")).toBeTruthy();
    await expectClean("OCR model prompt");
  });
});

describe.each([
  ["en", "light"],
  ["ar", "dark"],
] as const)("Settings and signing pass axe (%s, %s)", (locale, theme) => {
  it("OCR models with the Tesseract note, a download and a delete being confirmed", async () => {
    resetMockOcr({
      states: { "nuqta-4b": installed },
      hardware: { accelerators: [], runtimeGpu: null, os: "linux", arch: "x86_64" },
    });
    await openApp(locale, theme);
    await run("app.settings");
    const settings = screen.getByTestId("settings");
    expect(within(settings).getByTestId("ocr-tesseract-note")).toBeTruthy();
    await expectClean("settings: OCR models on Linux");
    // Every row's buttons say which model they act on.
    const row = within(settings).getByTestId("ocr-model-nuqta-4b");
    const remove = within(row).getAllByRole("button")[0] as HTMLElement;
    expect(remove.getAttribute("aria-describedby")).toBeTruthy();
    act(() => remove.click());
    await settle();
    await expectClean("settings: deleting a model, confirming");
    await run("ocrModels.download", { modelId: "llama-cpp" });
    await expectClean("settings: a download running");
  });

  it("crash reports kept, one shown", async () => {
    setMockCrashReports([sampleCrashReport()]);
    await openApp(locale, theme);
    // The report is offered once after the crash; "Not now" leaves it in Settings.
    await waitFor(() => expect(isCommandEnabled("crash.dismiss")).toBe(true));
    await expectClean("crash report prompt");
    await run("crash.dismiss");
    await run("app.settings");
    const report = await screen.findByTestId("crash-report");
    await expectClean("settings: a crash report kept");
    act(() => (within(report).getAllByRole("button")[0] as HTMLElement).click());
    await settle();
    expect(screen.getByTestId("crash-report-text")).toBeTruthy();
    await expectClean("settings: a crash report shown");
  });

  it("the certificate dialog's time-stamp field, empty", async () => {
    await openApp(locale, theme);
    await run("file.open");
    await run("certSign.open");
    act(() => useCertSign.getState().updateForm({ timestamp: true, tsaUrl: "" }));
    await settle();
    const field = document.getElementById("cert-sign-tsa");
    expect(field?.getAttribute("aria-invalid")).toBe("true");
    expect(field?.getAttribute("aria-describedby")).toContain("cert-sign-tsa-missing");
    await expectClean("certificate dialog with an empty time-stamp address");
    await act(() => runCommand("certSign.cancel"));
  });

  it("the keyboard shortcuts list", async () => {
    await openApp(locale, theme);
    await run("app.shortcuts");
    const list = screen.getByTestId("shortcuts");
    expect(within(list).getByText("F6")).toBeTruthy();
    await expectClean("keyboard shortcuts");
  });
});

describe.each([
  ["en", "dark"],
  ["ar", "light"],
] as const)("the main new screens pass axe (%s, %s)", (locale, theme) => {
  it("OCR panel and review card, Settings", async () => {
    await openApp(locale, theme);
    await recognised();
    await expectClean("OCR panel");
    await run("ocr.startReview");
    await expectClean("review card");
    await run("ocr.stopReview");
    await run("app.settings");
    await expectClean("settings");
  });
});
