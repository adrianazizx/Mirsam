import { create } from "zustand";
import type { OcrJob, OcrReviewPage, OcrReviewWord } from "../backend/types";

/** The flagged word shown in the viewer: its page and its box in points. */
export interface OcrFocus {
  documentId: number;
  page: number;
  /** The word's place in the review list: `[page index, word index]`. */
  at: [number, number];
  rect: { x: number; y: number; width: number; height: number } | null;
}

/**
 * Background OCR jobs and the pages-needing-review lists (ADR 0035), as the
 * shell reports them. Jobs are kept by id; a job knows which open documents
 * show its file.
 */
interface OcrJobsState {
  jobs: Record<number, OcrJob>;
  /** Each document's review list, once fetched. */
  review: Record<number, OcrReviewPage[]>;
  focus: OcrFocus | null;
  setJob(job: OcrJob): void;
  setJobs(jobs: OcrJob[]): void;
  forgetJob(jobId: number): void;
  setReview(documentId: number, pages: OcrReviewPage[]): void;
  setFocus(focus: OcrFocus | null): void;
}

export const useOcrJobs = create<OcrJobsState>()((set) => ({
  jobs: {},
  review: {},
  focus: null,
  setJob: (job) => set((state) => ({ jobs: { ...state.jobs, [job.id]: job } })),
  setJobs: (jobs) =>
    set((state) => ({
      jobs: { ...state.jobs, ...Object.fromEntries(jobs.map((job) => [job.id, job])) },
    })),
  forgetJob: (jobId) =>
    set((state) => {
      const { [jobId]: _gone, ...jobs } = state.jobs;
      return { jobs };
    }),
  setReview: (documentId, pages) =>
    set((state) => ({ review: { ...state.review, [documentId]: pages } })),
  setFocus: (focus) => set({ focus }),
}));

/** A document's jobs, oldest first. */
export function jobsOf(jobs: Record<number, OcrJob>, documentId: number): OcrJob[] {
  return Object.values(jobs)
    .filter((job) => job.documentIds.includes(documentId))
    .sort((a, b) => a.id - b.id);
}

/** Whether a job is still to finish (it can be paused or cancelled). */
export function isLive(job: OcrJob): boolean {
  return job.state === "queued" || job.state === "running";
}

/** The flagged words of a review list in order: `[page index, word index]` pairs. */
export function flaggedWords(pages: OcrReviewPage[]): [number, number][] {
  return pages.flatMap((page, p) => page.words.map((_, w) => [p, w] as [number, number]));
}

/** The word at `at` in a review list. */
export function wordAt(pages: OcrReviewPage[], at: [number, number]): OcrReviewWord | undefined {
  return pages[at[0]]?.words[at[1]];
}

/** A word's box as a viewer rectangle (points, top-left origin), when it has one. */
export function wordRect(word: OcrReviewWord): OcrFocus["rect"] {
  if (!word.bounds) return null;
  const [left, top, right, bottom] = word.bounds;
  return { x: left, y: top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
}
