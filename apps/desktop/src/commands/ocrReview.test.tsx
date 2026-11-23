import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { resetMockOcr } from "../backend/mockOcr";
import { resetMockOcrJobs } from "../backend/mockOcrJobs";
import { useAppStore } from "../state/appStore";
import { useOcrJobs } from "../state/ocrJobs";
import { useOcrModels } from "../state/ocrModels";
import { commandForEvent, isCommandEnabled, runCommand } from "./registry";

const app = () => useAppStore.getState();
const jobs = () => useOcrJobs.getState();
const installed = { kind: "installed" } as const;

beforeEach(() => {
  localStorage.clear();
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
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    activeTool: "select",
    settingsOpen: false,
  });
});

/** Opens a document and recognises its first page; resolves once its review list is in. */
async function recognised() {
  render(<App />);
  act(() => app().addDocument({ id: 1, name: "scan.pdf", pageCount: 6 }));
  await act(() => runCommand("ocr.recognizePage"));
  const job = await screen.findByTestId("ocr-job");
  await waitFor(() => expect(job.textContent).toContain("Done."));
  await screen.findByTestId("ocr-review");
}

/** The key event the app's shortcut handler sees. */
function key(init: KeyboardEventInit) {
  const event = new KeyboardEvent("keydown", { bubbles: true, ...init });
  const command = commandForEvent(event, false);
  return command ? runCommand(command.id) : Promise.resolve();
}

describe("OCR review mode (ADR 0037)", () => {
  it("steps through flagged words with the word's image beside its text, keyboard first", async () => {
    await recognised();
    await act(() => runCommand("ocr.startReview"));
    const card = await screen.findByTestId("ocr-review-card");
    expect(card.textContent).toContain("1 of 3 left to look at");
    // The page around the word, with the word marked on it.
    const image = within(card).getByTestId("ocr-review-image");
    expect(image.getAttribute("dir")).toBe("ltr");
    expect(within(image).getByRole("img", { name: "The page around ١٣٣٤" })).toBeTruthy();
    // The viewer is on the word's page, the word marked.
    expect(jobs().focus).toMatchObject({ page: 0, word: 4 });

    // The field holds the reading and has the focus; typing a correction and Enter writes it.
    const field = within(card).getByTestId<HTMLInputElement>("ocr-review-text");
    await waitFor(() => expect(document.activeElement).toBe(field));
    expect(field.value).toBe("١٣٣٤");
    expect(within(card).getByRole("button", { name: /^Use .*١٢٣٤/ })).toBeTruthy();
    fireEvent.change(field, { target: { value: "١٢٣٤" } });
    expect(within(card).getByRole("button", { name: "Correct" })).toBeTruthy();
    await act(async () => {
      fireEvent.submit(field);
    });
    // A correction is an edit of the document: undoable, the document modified.
    await waitFor(() => expect(app().documents[0]?.canUndo).toBe(true));
    const review = screen.getByTestId("ocr-review");
    await waitFor(() =>
      expect(
        within(review).getByRole("button", { name: /١٢٣٤.*corrected by you; read as .*١٣٣٤/ }),
      ).toBeTruthy(),
    );
    // On to the next open word.
    await waitFor(() => expect(jobs().focus?.word).toBe(9));
    expect(card.textContent).toContain("1 of 2 left to look at");

    // Enter on an unchanged reading accepts it; Cmd/Ctrl+Enter marks one checked.
    await act(async () => {
      fireEvent.submit(within(card).getByTestId("ocr-review-text"));
    });
    await waitFor(() => expect(jobs().focus?.word).toBe(12));
    expect(isCommandEnabled("ocr.reviewCheck")).toBe(true);
    await act(() => key({ key: "Enter", ctrlKey: true }));
    await waitFor(() => expect(card.textContent).toContain("Nothing left to review."));
    const decided = jobs().review[1]?.[0]?.words.map((w) => [w.text, w.review.kind]);
    expect(decided).toEqual([
      ["١٢٣٤", "edited"],
      ["المراجعة", "accepted"],
      ["Invoice", "checked"],
    ]);
    expect(screen.getByTestId("ocr-review").textContent).toContain("All decided");

    // Every number on request; F8 steps to it.
    act(() => {
      fireEvent.click(
        within(screen.getByTestId("ocr-review")).getByRole("checkbox", {
          name: "Also every number, date and ID",
        }),
      );
    });
    await waitFor(() => expect(jobs().review[1]?.[0]?.words.map((w) => w.text)).toContain("2025"));
    await act(() => key({ key: "F8" }));
    expect(jobs().focus?.word).toBe(15);
    // The page as a whole, then out of review mode.
    await act(() => key({ key: "Enter", ctrlKey: true, shiftKey: true }));
    await waitFor(() => expect(jobs().review[1]?.[0]?.checked).toBe(true));
    fireEvent.click(within(card).getByRole("button", { name: "Done reviewing" }));
    expect(jobs().reviewing).toBe(false);
    expect(screen.queryByTestId("ocr-review-card")).toBeNull();
  });

  it("reviews right to left in Arabic, with Arabic-Indic digits", async () => {
    await recognised();
    act(() => app().setLocale("ar"));
    await act(() => runCommand("ocr.startReview"));
    const card = await screen.findByTestId("ocr-review-card");
    expect(document.documentElement.dir).toBe("rtl");
    expect(within(card).getByRole("heading", { name: "المراجعة" })).toBeTruthy();
    expect(card.textContent).toContain("١ من ٣ باقية للمراجعة");
    expect(card.textContent).toContain("الصفحة ١");
    // The page's geometry stays left to right inside the right-to-left card.
    expect(within(card).getByTestId("ocr-review-image").getAttribute("dir")).toBe("ltr");
    expect(within(card).getByTestId<HTMLInputElement>("ocr-review-text").getAttribute("dir")).toBe(
      "auto",
    );
    expect(within(card).getByRole("button", { name: "قبول" })).toBeTruthy();
    expect(within(card).getByRole("button", { name: /^استخدام .*١٢٣٤/ })).toBeTruthy();
  });
});

describe("reading an area again (ADR 0038)", () => {
  it("draws an area, reads it as numbers only, and lists what is still unsure", async () => {
    await recognised();
    await act(() => runCommand("ocr.drawRegion"));
    expect(jobs().drawingRegion).toBe(true);
    const panel = screen.getByTestId("ocr-panel");
    expect(panel.textContent).toContain("Drag a rectangle on the page");
    // The viewer reports the drag.
    await act(() =>
      runCommand("ocr.setRegion", { page: 0, rect: { x: 295, y: 115, width: 50, height: 25 } }),
    );
    expect(jobs().drawingRegion).toBe(false);
    expect(panel.textContent).toContain("Area on page 1");
    fireEvent.click(within(panel).getByRole("radio", { name: "Arabic" }));
    fireEvent.click(within(panel).getByRole("checkbox", { name: "Numbers only" }));
    expect([jobs().regionLanguage, jobs().regionNumbers]).toEqual(["arabic", true]);
    await act(() => runCommand("ocr.readRegion"));
    expect(jobs().region).toBeNull();
    const job = Object.values(jobs().jobs).find((j) => j.options.kind.kind === "region");
    expect(job?.options.kind).toEqual({
      kind: "region",
      rect: [295, 115, 345, 140],
      language: "arabic",
      numbers: true,
    });
    // The area's word replaces the old one and stays flagged.
    await waitFor(() => {
      const words = jobs().review[1]?.[0]?.words.map((w) => w.text) ?? [];
      expect(words).toContain("I234");
      expect(words).not.toContain("١٣٣٤");
    });
  });
});

describe("replacing a poor text layer (ADR 0038)", () => {
  it("checks the layers, says what is wrong, and replaces the poor ones", async () => {
    render(<App />);
    act(() => app().addDocument({ id: 1, name: "scan.pdf", pageCount: 3 }));
    await act(() => runCommand("ocr.checkLayers"));
    const layers = await screen.findByTestId("ocr-layers");
    await waitFor(() =>
      expect(layers.textContent).toMatch(/Poor text layer on 1 pages; \d+ look right\./),
    );
    expect(layers.textContent).toContain(
      "Garbled characters · Does not match the page · 4% like the page",
    );
    expect(layers.querySelector("bdi")?.textContent).toBe("ÇáÚÞÏ Èíä ÇáØÑÝíä");
    expect(isCommandEnabled("ocr.replaceLayers")).toBe(true);
    fireEvent.click(within(layers).getByRole("button", { name: "Replace 1 poor layers" }));
    await waitFor(() =>
      expect(Object.values(jobs().jobs).some((j) => j.options.replace)).toBe(true),
    );
    const replacing = Object.values(jobs().jobs).find((j) => j.options.replace);
    expect(replacing?.pages).toBe(1);
  });
});

describe("scan to editable text (ADR 0039)", () => {
  it("converts a page once its unsure words are decided", async () => {
    await recognised();
    await act(() => runCommand("ocr.convertPage"));
    // An unsure word is never made page text without a decision.
    expect(app().notice).toEqual({ key: "ocrConvertUnreviewed", values: { pages: "1" } });
    const edits = app().documents[0]?.canUndo;
    await act(() => runCommand("ocr.startReview"));
    await act(() => runCommand("ocr.reviewChoose", { reviewText: "١٢٣٤" }));
    fireEvent.click(screen.getByRole("radio", { name: "Remove the scan" }));
    expect(jobs().keepScan).toBe(false);
    await act(() => runCommand("ocr.convertPage"));
    await waitFor(() =>
      expect(screen.getByTestId("ocr-review").textContent).toContain("Converted to page text"),
    );
    expect(edits).toBe(true);
    // Its words are fixed with the Edit tool now.
    await act(() => runCommand("ocr.startReview"));
    const card = await screen.findByTestId("ocr-review-card");
    expect(card.textContent).toContain("fix its words with the Edit tool");
  });
});
