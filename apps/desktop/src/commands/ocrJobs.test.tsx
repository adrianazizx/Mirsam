import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { resetMockOcr } from "../backend/mockOcr";
import { resetMockOcrJobs } from "../backend/mockOcrJobs";
import { useAppStore } from "../state/appStore";
import { useOcrJobs } from "../state/ocrJobs";
import { useOcrModels } from "../state/ocrModels";
import { isCommandEnabled, runCommand } from "./registry";

const app = () => useAppStore.getState();
const jobs = () => useOcrJobs.getState();
const installed = { kind: "installed" } as const;

beforeEach(() => {
  localStorage.clear();
  resetMockOcr({ states: { "nuqta-4b": installed, "llama-cpp": installed } });
  resetMockOcrJobs();
  useOcrModels.setState({ models: null, jobs: {}, failures: {}, prompt: false });
  useOcrJobs.setState({ jobs: {}, review: {}, focus: null });
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

function openDocument() {
  act(() => app().addDocument({ id: 1, name: "scan.pdf", pageCount: 240 }));
}

describe("background OCR jobs (ADR 0035)", () => {
  it("drafts the page, then writes the quality text as an edit, and lists what to review", async () => {
    render(<App />);
    openDocument();
    await act(() => runCommand("ocr.recognizePage"));
    expect(app().activeTool).toBe("ocr");
    const panel = await screen.findByTestId("ocr-panel");
    const job = await within(panel).findByTestId("ocr-job");
    expect(within(job).getByRole("progressbar")).toBeTruthy();

    await waitFor(() => expect(job.textContent).toContain("Done."));
    expect(job.textContent).toContain("1 of 1 pages read in quality mode; 1 searchable");
    // Draft, then quality: two edits, the document modified and undoable.
    expect(app().documents[0]?.canUndo).toBe(true);
    expect(app().documents[0]?.modified).toBe(true);

    // Page 1 has an unsure number and a corrected word to look at.
    const review = await screen.findByTestId("ocr-review");
    expect(within(review).getByRole("heading", { name: "Pages needing review" })).toBeTruthy();
    expect(review.textContent).toContain("Page 1");
    expect(review.textContent).toContain("Unsure: 1");
    expect(review.textContent).toContain("Corrected: 1");
    expect(review.textContent).toContain("93% confident");
    const unsure = within(review).getByRole("button", {
      name: /١٣٣٤.*Unsure; also read as .*١٢٣٤.*41% confident/,
    });
    fireEvent.click(unsure);
    expect(jobs().focus).toEqual({
      documentId: 1,
      page: 0,
      at: [0, 0],
      rect: { x: 300, y: 120, width: 40, height: 16 },
    });
    expect(unsure.getAttribute("aria-current")).toBe("true");

    // Next and previous step through the flagged words, wrapping around.
    expect(isCommandEnabled("ocr.nextFlag")).toBe(true);
    await act(() => runCommand("ocr.nextFlag"));
    expect(jobs().focus?.at).toEqual([0, 1]);
    await act(() => runCommand("ocr.nextFlag"));
    expect(jobs().focus?.at).toEqual([0, 0]);
    await act(() => runCommand("ocr.previousFlag"));
    expect(jobs().focus?.at).toEqual([0, 1]);

    // A finished job can be removed.
    fireEvent.click(within(job).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(screen.queryByTestId("ocr-job")).toBeNull());
  });

  it("pauses, resumes and cancels a job", async () => {
    render(<App />);
    openDocument();
    await act(() => runCommand("ocr.recognize"));
    const job = await screen.findByTestId("ocr-job");
    await waitFor(() => expect(job.textContent).toMatch(/Reading page \d+ in quality mode/));
    fireEvent.click(within(job).getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(job.textContent).toContain("Paused."));
    expect(within(job).queryByRole("button", { name: "Pause" })).toBeNull();
    fireEvent.click(within(job).getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(within(job).getByRole("button", { name: "Pause" })).toBeTruthy());
    fireEvent.click(within(job).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(job.textContent).toContain("Cancelled."));
    expect(Object.values(jobs().jobs).map((j) => j.state)).toEqual(["cancelled"]);
  });

  it("lays the panel out right to left in Arabic, with Western digits", async () => {
    render(<App />);
    act(() => app().setLocale("ar"));
    openDocument();
    await act(() => runCommand("ocr.recognizePage"));
    const panel = await screen.findByTestId("ocr-panel");
    expect(document.documentElement.dir).toBe("rtl");
    const job = await within(panel).findByTestId("ocr-job");
    await waitFor(() => expect(job.textContent).toContain("اكتمل."));
    const review = await screen.findByTestId("ocr-review");
    expect(within(review).getByRole("heading", { name: "صفحات تحتاج إلى مراجعة" })).toBeTruthy();
    expect(review.textContent).toContain("الصفحة 1");
    expect(review.textContent).toContain("غير مؤكَّدة: 1");
    // The word keeps its own direction inside the Arabic label.
    expect(review.querySelector("bdi")?.textContent).toBe("١٣٣٤");
    expect(within(panel).getByRole("button", { name: "التعرّف على هذه الصفحة" })).toBeTruthy();
  });

  it("explains why a job stopped", async () => {
    render(<App />);
    openDocument();
    act(() =>
      jobs().setJob({
        id: 9,
        name: "scan.pdf",
        pages: 3,
        state: "failed",
        error: "ocr_engine_stopped",
        drafted: 3,
        finished: 1,
        current: null,
        documentIds: [1],
      }),
    );
    act(() => app().setActiveTool("ocr"));
    const job = await screen.findByTestId("ocr-job");
    expect(job.textContent).toContain(
      "Stopped: The OCR engine stopped, perhaps for lack of memory.",
    );
    expect(within(job).getByRole("button", { name: "Resume" })).toBeTruthy();
    expect(screen.getByTestId("ocr-panel").textContent).toContain(
      "Pages show a quick draft until the quality reading replaces it.",
    );
  });
});
