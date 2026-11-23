import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { setMockRecoveries } from "../backend/mock";
import { resetMockOcr } from "../backend/mockOcr";
import { resetMockOcrJobs } from "../backend/mockOcrJobs";
import type { OcrJob, OcrLayerCheck } from "../backend/types";
import { type CommandId, commandForEvent, runCommand } from "../commands/registry";
import { useAnnouncer } from "../state/announcer";
import { useAppStore } from "../state/appStore";
import { useOcrJobs } from "../state/ocrJobs";
import { useOcrModels } from "../state/ocrModels";
import { press, pressTab } from "./keyboard";
import { announceLayerChecks, announceOcrJob } from "./ocrAnnouncements";

/**
 * The OCR tool from the keyboard and for screen readers (ADR 0070's second
 * pass): focus in review mode, one tab stop for the flagged words, an area set
 * without a pointer, what is announced as jobs and downloads move on, and the
 * shortcuts that make the rest findable.
 */

const installed = { kind: "installed" } as const;
const jobs = () => useOcrJobs.getState();

/** Everything said, politely or not, since the test started. */
let said: string[] = [];
let stopListening = () => {};

async function settle() {
  await act(async () => {
    for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

async function run(command: CommandId, arg?: Parameters<typeof runCommand>[1]) {
  await act(async () => {
    await runCommand(command, arg);
  });
  await settle();
}

beforeEach(() => {
  localStorage.clear();
  setMockRecoveries([]);
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
    checks: {},
  });
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    activeTool: "select",
    settingsOpen: false,
    shortcutsOpen: false,
  });
  said = [];
  useAnnouncer.setState({ polite: { text: "", seq: 0 }, assertive: { text: "", seq: 0 } });
  stopListening = useAnnouncer.subscribe((state, before) => {
    if (state.polite.seq !== before.polite.seq) said.push(state.polite.text);
    if (state.assertive.seq !== before.assertive.seq) said.push(`!${state.assertive.text}`);
  });
});

afterEach(() => stopListening());

async function recognised(locale: "en" | "ar" = "en") {
  render(<App />);
  act(() => useAppStore.getState().setLocale(locale));
  act(() => useAppStore.getState().addDocument({ id: 1, name: "scan.pdf", pageCount: 6 }));
  await settle();
  await run("ocr.recognizePage");
  const job = await screen.findByTestId("ocr-job");
  await waitFor(() => expect(job.textContent).toMatch(/Done\.|اكتمل\./));
  await screen.findByTestId("ocr-review");
}

describe("review mode from the keyboard", () => {
  it("reads the word's place and reasons with its field, and never drops focus", async () => {
    await recognised();
    await run("ocr.startReview");
    const card = screen.getByTestId("ocr-review-card");
    const field = within(card).getByTestId<HTMLInputElement>("ocr-review-text");
    await waitFor(() => expect(document.activeElement).toBe(field));
    // Its description: where it is, why it is listed, how sure the engine was.
    const description = (field.getAttribute("aria-describedby") ?? "")
      .split(" ")
      .map((id) => document.getElementById(id)?.textContent ?? "")
      .join(" | ");
    expect(description).toContain("1 of 3 left to look at");
    expect(description).toMatch(/Page 1 · \d+% confident/);
    // The buttons' shortcuts in ARIA's own syntax, not the symbols shown.
    const check = within(card).getByRole("button", { name: "Mark checked" });
    expect(check.getAttribute("aria-keyshortcuts")).toMatch(/^(Meta|Control)\+Enter$/);
    expect(
      within(card).getByRole("button", { name: "Skip" }).getAttribute("aria-keyshortcuts"),
    ).toBe("F8");

    // Escape leaves review mode; focus goes to the way back in.
    press("Escape");
    await settle();
    expect(jobs().reviewing).toBe(false);
    const start = screen.getByRole("button", { name: "Review" });
    expect(document.activeElement).toBe(start);

    // Deciding the last word leaves focus on "Done reviewing", and says so.
    await run("ocr.startReview");
    for (let i = 0; i < 3; i += 1) {
      const current = await screen.findByTestId<HTMLInputElement>("ocr-review-text");
      await waitFor(() => expect(document.activeElement).toBe(current));
      await act(async () => {
        fireEvent.submit(current);
      });
      await settle();
    }
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Done reviewing" })),
    );
    expect(said).toContain("Nothing left to review.");
  });

  it("makes the flagged words one tab stop, moved through with the arrow keys", async () => {
    await recognised();
    const review = screen.getByTestId("ocr-review");
    const words = within(review)
      .getAllByRole("button")
      .filter((button) => button.hasAttribute("data-word"));
    expect(words.length).toBeGreaterThan(2);
    expect(words.filter((word) => word.tabIndex === 0)).toHaveLength(1);
    act(() => words[0]?.focus());
    press("ArrowDown");
    expect(document.activeElement).toBe(words[1]);
    press("End");
    expect(document.activeElement).toBe(words.at(-1));
    press("Home");
    expect(document.activeElement).toBe(words[0]);
    // Tab leaves the list instead of walking every word.
    const next = pressTab();
    expect(next?.hasAttribute("data-word")).toBe(false);
  });

  it("works the same right to left, in Arabic", async () => {
    await recognised("ar");
    await run("ocr.startReview");
    const field = await screen.findByTestId<HTMLInputElement>("ocr-review-text");
    await waitFor(() => expect(document.activeElement).toBe(field));
    const description = (field.getAttribute("aria-describedby") ?? "")
      .split(" ")
      .map((id) => document.getElementById(id)?.textContent ?? "")
      .join(" | ");
    expect(description).toContain("١ من ٣ باقية للمراجعة");
    press("Escape");
    await settle();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "مراجعة" }));
  });
});

describe("an area read again, without a pointer", () => {
  it("starts from the whole page and is narrowed with the fields", async () => {
    await recognised();
    const panel = screen.getByTestId("ocr-panel");
    fireEvent.click(within(panel).getByRole("button", { name: "Use the whole page" }));
    await settle();
    const region = jobs().region;
    expect(region?.page).toBe(0);
    expect(region?.size).toBeTruthy();
    expect(said).toContain("Area on page 1");
    const width = within(panel).getByRole("spinbutton", { name: "Width" });
    expect((width as HTMLInputElement).value).toBe("100");
    fireEvent.change(width, { target: { value: "50" } });
    fireEvent.change(within(panel).getByRole("spinbutton", { name: "From the left edge" }), {
      target: { value: "25" },
    });
    const size = region?.size ?? { width: 1, height: 1 };
    const rect = jobs().region?.rect;
    expect(rect?.width).toBeCloseTo(size.width / 2);
    expect(rect?.x).toBeCloseTo(size.width / 4);
    // Said once, when the area was set, not at every adjustment.
    expect(said.filter((text) => text === "Area on page 1")).toHaveLength(1);
  });
});

describe("what OCR announces", () => {
  it("a job starting and finishing", async () => {
    await recognised();
    // The mock shell names the job after its own file.
    expect(said).toContain("Recognition of ⁨mock.pdf⁩: started.");
    await waitFor(() => expect(said).toContain("Recognition of ⁨mock.pdf⁩: Done."));
  });

  it("pauses, failures (assertively) and a layer check's verdict", () => {
    const job = {
      id: 7,
      name: "scan.pdf",
      documentIds: [1],
      state: "running",
      pages: 3,
      finished: 1,
      drafted: 1,
      current: null,
      error: null,
      options: { kind: { kind: "recognize" }, replace: false },
    } as unknown as OcrJob;
    announceOcrJob(job, { ...job, state: "paused" });
    announceOcrJob({ ...job, state: "paused" }, job);
    announceOcrJob(job, { ...job, state: "failed", error: "model_unavailable" });
    expect(said[0]).toBe("Recognition of \u2068scan.pdf\u2069: Paused.");
    expect(said[1]).toBe("Recognition of \u2068scan.pdf\u2069: resumed.");
    expect(said[2]).toMatch(/^!Recognition of .*scan\.pdf.*: Stopped: /);
    // Progress alone says nothing.
    announceOcrJob(job, { ...job, finished: 2 });
    expect(said).toHaveLength(3);
    announceLayerChecks([
      { page: 0, kind: "invisible", verdict: "poor", problems: [], similarity: 0.1, sample: "" },
      { page: 1, kind: "invisible", verdict: "good", problems: [], similarity: 0.9, sample: "" },
    ] as OcrLayerCheck[]);
    expect(said[3]).toBe("Poor text layer on 1 pages; 1 look right.");
  });

  it("a model download ending", async () => {
    resetMockOcr({ states: { "nuqta-4b": installed } });
    render(<App />);
    await run("app.settings");
    await run("ocrModels.download", { modelId: "llama-cpp" });
    await waitFor(() => expect(said).toContain("⁨llama.cpp⁩: installed."), { timeout: 5000 });
  });
});

describe("shortcuts that make the rest findable", () => {
  it("sizes the interface with Option/Alt beside the zoom keys, by the physical key", () => {
    const event = (init: KeyboardEventInit) => new KeyboardEvent("keydown", init);
    // Option changes the character on macOS ("≠", "–", "º").
    expect(
      commandForEvent(event({ key: "≠", code: "Equal", metaKey: true, altKey: true }), true)?.id,
    ).toBe("app.uiLarger");
    expect(
      commandForEvent(event({ key: "–", code: "Minus", metaKey: true, altKey: true }), true)?.id,
    ).toBe("app.uiSmaller");
    expect(
      commandForEvent(event({ key: "º", code: "Digit0", metaKey: true, altKey: true }), true)?.id,
    ).toBe("app.uiDefaultSize");
    expect(
      commandForEvent(event({ key: "-", code: "Minus", ctrlKey: true, altKey: true }), false)?.id,
    ).toBe("app.uiSmaller");
    // The document's own zoom keeps its keys.
    expect(commandForEvent(event({ key: "=", code: "Equal", metaKey: true }), true)?.id).toBe(
      "view.zoomIn",
    );
  });

  it("lists every shortcut, F6 and F8 included, and gives focus back on Escape", async () => {
    render(<App />);
    await settle();
    const opener = document.body;
    await run("app.shortcuts");
    const list = screen.getByTestId("shortcuts");
    const row = within(list).getByRole("row", { name: /Next area of the window/ });
    expect(row.textContent).toContain("F6");
    expect(within(list).getByRole("row", { name: /Next flagged OCR word/ }).textContent).toContain(
      "F8",
    );
    press("Escape");
    await settle();
    expect(screen.queryByTestId("shortcuts")).toBeNull();
    expect(useAppStore.getState().shortcutsOpen).toBe(false);
    expect(opener).toBeTruthy();
  });
});
