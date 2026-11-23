import { MOCK_STEP_MS } from "./mockOcr";
import type {
  Backend,
  EditOutcome,
  OcrApplied,
  OcrJob,
  OcrJobAction,
  OcrReviewPage,
} from "./types";

/**
 * Background OCR jobs in the browser stand-in (ADR 0035): a timer drafts every
 * page in one step, then "reads" one page per step in quality mode, writing
 * each as an edit, so the OCR panel, its progress and the review list can be
 * reviewed and tested without Tauri. Every third finished page has words to
 * review.
 */

type JobMethods = Pick<
  Backend,
  | "startOcr"
  | "ocrJobs"
  | "controlOcrJob"
  | "removeOcrJob"
  | "ocrReview"
  | "onOcrJob"
  | "onOcrApplied"
>;

export interface MockOcrHooks {
  pageCount(documentId: number): number;
  /** Records an edit of the document's content and returns its outcome. */
  write(documentId: number): EditOutcome;
}

interface MockJob {
  job: OcrJob;
  documentId: number;
  pages: number[];
  timer: ReturnType<typeof setTimeout> | null;
}

let jobs = new Map<number, MockJob>();
let nextJob = 1;
const reviews = new Map<number, Map<number, OcrReviewPage>>();
const jobListeners = new Set<(job: OcrJob) => void>();
const appliedListeners = new Set<(applied: OcrApplied) => void>();

/** Test hook: no jobs, no review lists. */
export function resetMockOcrJobs() {
  for (const { timer } of jobs.values()) if (timer) clearTimeout(timer);
  jobs = new Map();
  reviews.clear();
  nextJob = 1;
}

function reviewFor(page: number): OcrReviewPage | null {
  if (page % 3 !== 0) return null;
  return {
    page,
    uncertain: 1,
    corrected: 1,
    reordered: 0,
    truncated: false,
    confidence: 0.93,
    lowest: 0.41,
    words: [
      {
        index: 4,
        text: "١٣٣٤",
        probability: 0.41,
        status: { kind: "uncertain", alternatives: ["١٢٣٤"] },
        bounds: [300, 120, 340, 136],
      },
      {
        index: 9,
        text: "المراجعة",
        probability: 0.88,
        status: { kind: "corrected", read: "المراجعه" },
        bounds: [180, 160, 250, 176],
      },
    ],
  };
}

export function mockOcrJobs(hooks: MockOcrHooks): JobMethods {
  const emit = (entry: MockJob) => {
    const job = { ...entry.job, current: entry.job.current, documentIds: [entry.documentId] };
    entry.job = job;
    for (const listener of jobListeners) listener({ ...job });
  };
  const apply = (entry: MockJob, pages: [number, "draft" | "final"][]) => {
    const outcome = { ...hooks.write(entry.documentId), textChanged: true };
    for (const listener of appliedListeners)
      listener({ documentId: entry.documentId, outcome, pages });
  };
  const step = (entry: MockJob) => {
    entry.timer = null;
    const { job } = entry;
    if (job.state !== "running") return;
    if (job.drafted < entry.pages.length) {
      job.drafted = entry.pages.length;
      apply(
        entry,
        entry.pages.map((page) => [page, "draft"]),
      );
    } else {
      const page = entry.pages[job.finished];
      if (page === undefined) return;
      job.finished += 1;
      const review = reviewFor(page);
      if (review) {
        const list = reviews.get(entry.documentId) ?? new Map<number, OcrReviewPage>();
        list.set(page, review);
        reviews.set(entry.documentId, list);
      }
      apply(entry, [[page, "final"]]);
    }
    const next = entry.pages[job.finished];
    if (job.finished >= entry.pages.length || next === undefined) {
      job.state = "done";
      job.current = null;
    } else {
      job.current = [next, "quality"];
      entry.timer = setTimeout(() => step(entry), MOCK_STEP_MS);
    }
    emit(entry);
  };
  const start = (entry: MockJob) => {
    entry.job.state = "running";
    const first = entry.pages[entry.job.finished];
    entry.job.current =
      first === undefined ? null : [first, entry.job.drafted ? "quality" : "draft"];
    entry.timer = setTimeout(() => step(entry), MOCK_STEP_MS);
    emit(entry);
  };
  const find = (jobId: number) => {
    const entry = jobs.get(jobId);
    if (!entry) throw { code: "ocr_unknown_job", detail: `no job ${jobId}` };
    return entry;
  };

  return {
    startOcr: async (documentId, pages) => {
      const count = hooks.pageCount(documentId);
      const wanted = [...new Set(pages ?? Array.from({ length: count }, (_, i) => i))].filter(
        (page) => page >= 0 && page < count,
      );
      if (wanted.length === 0) throw { code: "invalid_edit", detail: "no pages" };
      const entry: MockJob = {
        job: {
          id: nextJob++,
          name: "mock.pdf",
          pages: wanted.length,
          state: "queued",
          error: null,
          drafted: 0,
          finished: 0,
          current: null,
          documentIds: [documentId],
        },
        documentId,
        pages: wanted,
        timer: null,
      };
      jobs.set(entry.job.id, entry);
      start(entry);
      return { ...entry.job };
    },
    ocrJobs: async (documentId) =>
      [...jobs.values()]
        .filter((entry) => entry.documentId === documentId)
        .map((entry) => ({ ...entry.job })),
    controlOcrJob: async (jobId, action: OcrJobAction) => {
      const entry = find(jobId);
      const { job } = entry;
      if (entry.timer) clearTimeout(entry.timer);
      entry.timer = null;
      if (action === "resume" && ["paused", "cancelled", "failed"].includes(job.state)) {
        start(entry);
      } else if (action === "pause" && ["running", "queued"].includes(job.state)) {
        job.state = "paused";
        job.current = null;
        emit(entry);
      } else if (action === "cancel" && ["running", "queued", "paused"].includes(job.state)) {
        job.state = "cancelled";
        job.current = null;
        emit(entry);
      }
      return { ...entry.job };
    },
    removeOcrJob: async (jobId) => {
      const entry = find(jobId);
      if (entry.job.state === "running") throw { code: "ocr_job_running", detail: "running" };
      jobs.delete(jobId);
    },
    ocrReview: async (documentId) =>
      [...(reviews.get(documentId)?.values() ?? [])].sort(
        (a, b) => b.uncertain - a.uncertain || a.page - b.page,
      ),
    onOcrJob: async (listener) => {
      jobListeners.add(listener);
      return () => jobListeners.delete(listener);
    },
    onOcrApplied: async (listener) => {
      appliedListeners.add(listener);
      return () => appliedListeners.delete(listener);
    },
  };
}
