import { create } from "zustand";
import type {
  OcrJob,
  OcrLanguage,
  OcrLayerCheck,
  OcrReviewPage,
  OcrReviewWord,
  ViewRect,
} from "../backend/types";

/** The flagged word shown in the viewer: its page and its box in points. */
export interface OcrFocus {
  documentId: number;
  page: number;
  /** The word's index among the page's words; null: the page itself (review mode). */
  word: number | null;
  /** The word's place in the review list: `[page index, word index]`. */
  at: [number, number] | null;
  rect: { x: number; y: number; width: number; height: number } | null;
}

/** An area drawn to be read again (ADR 0038), on the displayed page. */
export interface OcrRegion {
  documentId: number;
  page: number;
  rect: ViewRect;
  /** The page's displayed size in points, when known: the panel's fields are percentages of it. */
  size?: { width: number; height: number } | null;
}

/**
 * Background OCR jobs and the pages-needing-review lists (ADR 0035), as the
 * shell reports them, with review mode (ADR 0037), the area being read again
 * and the layer checks (ADR 0038), and how pages are converted (ADR 0039).
 * Jobs are kept by id; a job knows which open documents show its file.
 */
interface OcrJobsState {
  jobs: Record<number, OcrJob>;
  /** Each document's review list, once fetched. */
  review: Record<number, OcrReviewPage[]>;
  focus: OcrFocus | null;
  /** Review mode: the card steps through the flagged words, keyboard first. */
  reviewing: boolean;
  /** The list also holds every number, date and ID. */
  numbers: boolean;
  /** Drawing an area to read again, and the area drawn. */
  drawingRegion: boolean;
  region: OcrRegion | null;
  regionLanguage: OcrLanguage;
  regionNumbers: boolean;
  /** Each document's layer checks, once fetched. */
  checks: Record<number, OcrLayerCheck[]>;
  /** Conversion to page text: keep the scan behind it, and the family. */
  keepScan: boolean;
  family: "naskh" | "sans";
  setJob(job: OcrJob): void;
  setJobs(jobs: OcrJob[]): void;
  forgetJob(jobId: number): void;
  setReview(documentId: number, pages: OcrReviewPage[]): void;
  setFocus(focus: OcrFocus | null): void;
  setReviewing(reviewing: boolean): void;
  setNumbers(numbers: boolean): void;
  setDrawingRegion(drawing: boolean): void;
  setRegion(region: OcrRegion | null): void;
  setRegionOptions(options: { language?: OcrLanguage; numbers?: boolean }): void;
  setChecks(documentId: number, checks: OcrLayerCheck[]): void;
  setConvertOptions(options: { keepScan?: boolean; family?: "naskh" | "sans" }): void;
}

export const useOcrJobs = create<OcrJobsState>()((set) => ({
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
  setReviewing: (reviewing) => set({ reviewing }),
  setNumbers: (numbers) => set({ numbers }),
  setDrawingRegion: (drawingRegion) => set({ drawingRegion }),
  setRegion: (region) => set({ region }),
  setRegionOptions: ({ language, numbers }) =>
    set((state) => ({
      regionLanguage: language ?? state.regionLanguage,
      regionNumbers: numbers ?? state.regionNumbers,
    })),
  setChecks: (documentId, checks) =>
    set((state) => ({ checks: { ...state.checks, [documentId]: checks } })),
  setConvertOptions: ({ keepScan, family }) =>
    set((state) => ({ keepScan: keepScan ?? state.keepScan, family: family ?? state.family })),
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

/**
 * What review mode steps through, in page order (ADR 0037): each word nobody
 * has looked at, and each page whose reading may be cut off and nobody has
 * checked (`word` null). Positions are `[page index, word index]` in `pages`.
 */
export interface ReviewItem {
  page: number;
  /** The word's index among the page's words; null for the page itself. */
  word: number | null;
  at: [number, number] | null;
}

export function reviewQueue(pages: OcrReviewPage[]): ReviewItem[] {
  const ordered = pages.map((page, p) => ({ page, p })).sort((a, b) => a.page.page - b.page.page);
  const items: ReviewItem[] = [];
  for (const { page, p } of ordered) {
    page.words.forEach((word, w) => {
      if (word.review.kind === "open")
        items.push({ page: page.page, word: word.index, at: [p, w] });
    });
    const left = page.words.some((word) => word.review.kind === "open");
    if (page.truncated && !page.checked && !left)
      items.push({ page: page.page, word: null, at: null });
  }
  return items;
}

/** Where `[page, word]` is in a review list, if it is there. */
export function positionOf(
  pages: OcrReviewPage[],
  page: number,
  word: number,
): [number, number] | null {
  const p = pages.findIndex((entry) => entry.page === page);
  if (p < 0) return null;
  const w = pages[p]?.words.findIndex((entry) => entry.index === word) ?? -1;
  return w < 0 ? null : [p, w];
}

/**
 * The item after `page`/`word` in reading order (wrapping round); the first
 * item when there is no current one. Null when nothing is left.
 */
export function nextItem(
  queue: ReviewItem[],
  current: { page: number; word: number | null } | null,
  pages: OcrReviewPage[],
): ReviewItem | null {
  if (queue.length === 0) return null;
  if (!current) return queue[0] ?? null;
  // Order within a page: the word's place in the page's list.
  const rank = (item: { page: number; word: number | null }) => {
    const entry = pages.find((p) => p.page === item.page);
    const w =
      item.word === null
        ? Number.MAX_SAFE_INTEGER
        : (entry?.words.findIndex((x) => x.index === item.word) ?? -1);
    return [item.page, w] as const;
  };
  const [cp, cw] = rank(current);
  const after = queue.find((item) => {
    const [ip, iw] = rank(item);
    return ip > cp || (ip === cp && iw > cw);
  });
  return after ?? queue[0] ?? null;
}
