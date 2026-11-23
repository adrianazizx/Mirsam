import { MOCK_STEP_MS } from "./mockOcr";
import type {
  Backend,
  EditOutcome,
  OcrAction,
  OcrApplied,
  OcrJob,
  OcrJobAction,
  OcrJobOptions,
  OcrLayerCheck,
  OcrReviewPage,
  OcrReviewWord,
} from "./types";

/**
 * Background OCR jobs in the browser stand-in (ADR 0035, 0037–0039): a timer
 * drafts every page in one step, then "reads" one page per step in quality
 * mode, writing each as an edit, so the OCR panel, its progress, the review
 * list, review mode, areas read again, layer checks and conversion can be
 * reviewed and tested without Tauri. Every third finished page has words to
 * review; page 1 has another tool's poor text layer.
 */

type JobMethods = Pick<
  Backend,
  | "startOcr"
  | "startOcrRegion"
  | "checkTextLayers"
  | "ocrLayerChecks"
  | "ocrJobs"
  | "controlOcrJob"
  | "removeOcrJob"
  | "ocrReview"
  | "ocrDecide"
  | "ocrConvert"
  | "ocrCropUrl"
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
/** Each document's review data by page: every word, listed or not. */
const reviews = new Map<number, Map<number, OcrReviewPage>>();
const checks = new Map<number, Map<number, OcrLayerCheck>>();
const jobListeners = new Set<(job: OcrJob) => void>();
const appliedListeners = new Set<(applied: OcrApplied) => void>();

/** Test hook: no jobs, no review lists. */
export function resetMockOcrJobs() {
  for (const { timer } of jobs.values()) if (timer) clearTimeout(timer);
  jobs = new Map();
  reviews.clear();
  checks.clear();
  nextJob = 1;
}

const word = (
  index: number,
  text: string,
  probability: number,
  status: OcrReviewWord["status"],
  reasons: OcrReviewWord["reasons"],
  bounds: [number, number, number, number],
): OcrReviewWord => ({
  index,
  text,
  probability,
  status,
  bounds,
  line: [60, bounds[1] - 2, 540, bounds[3] + 2],
  review: { kind: "open" },
  reasons,
});

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
    undecided: 1,
    open: 3,
    checked: false,
    converted: false,
    stale: false,
    words: [
      word(
        4,
        "١٣٣٤",
        0.41,
        { kind: "uncertain", alternatives: ["١٢٣٤"] },
        ["uncertain", "number"],
        [300, 120, 340, 136],
      ),
      word(
        9,
        "المراجعة",
        0.88,
        { kind: "corrected", read: "المراجعه" },
        ["corrected"],
        [180, 160, 250, 176],
      ),
      word(12, "Invoice", 0.62, { kind: "unchecked" }, ["lowConfidence"], [80, 200, 140, 216]),
      word(15, "2025", 0.99, { kind: "confirmed" }, ["number"], [150, 200, 190, 216]),
    ],
  };
}

/** A page's entry as listed: number-only words on request, counts brought up to date. */
function listed(page: OcrReviewPage, numbers: boolean): OcrReviewPage | null {
  const words = page.words.filter((w) => numbers || w.reasons.some((r) => r !== "number"));
  if (words.length === 0 && !page.truncated) return null;
  const undecided = page.words.filter(
    (w) => w.status.kind === "uncertain" && w.review.kind === "open",
  ).length;
  return {
    ...page,
    undecided,
    open: words.filter((w) => w.review.kind === "open").length,
    words: words.map((w) => ({ ...w })),
  };
}

const needsDecision = (page: OcrReviewPage) =>
  (page.truncated && !page.checked) || page.undecided > 0;

function list(documentId: number, numbers: boolean): OcrReviewPage[] {
  return [...(reviews.get(documentId)?.values() ?? [])]
    .map((page) => listed(page, numbers))
    .filter((page): page is OcrReviewPage => page !== null)
    .sort(
      (a, b) =>
        Number(needsDecision(b)) - Number(needsDecision(a)) ||
        b.undecided - a.undecided ||
        a.page - b.page,
    );
}

function decide(page: OcrReviewPage, index: number | null, action: OcrAction): boolean {
  if (action.kind === "checkPage") {
    page.checked = true;
    for (const w of page.words) if (w.review.kind === "open") w.review = { kind: "checked" };
    return false;
  }
  const target = page.words.find((w) => w.index === index);
  if (!target) throw { code: "ocr_review_no_word", detail: "no such word" };
  if (action.kind === "edit") {
    const text = action.text.trim().split(/\s+/).join(" ");
    if (text === target.text) {
      if (target.review.kind !== "edited") target.review = { kind: "accepted" };
      return false;
    }
    if (page.converted) throw { code: "ocr_review_converted", detail: "converted" };
    const read = target.review.kind === "edited" ? target.review.read : target.text;
    target.text = text;
    target.review = { kind: "edited", read };
    return true;
  }
  if (action.kind === "accept" && target.review.kind !== "edited")
    target.review = { kind: "accepted" };
  if (action.kind === "check" && target.review.kind === "open") target.review = { kind: "checked" };
  return false;
}

function checkFor(page: number): OcrLayerCheck {
  if (page === 1)
    return {
      page,
      kind: "invisible",
      verdict: "poor",
      problems: ["mojibake", "mismatch"],
      similarity: 0.04,
      sample: "ÇáÚÞÏ Èíä ÇáØÑÝíä",
    };
  return {
    page,
    kind: page % 2 ? "invisible" : "none",
    verdict: page % 2 ? "good" : "notChecked",
    problems: [],
    similarity: page % 2 ? 0.91 : null,
    sample: page % 2 ? "نص العقد بين الطرفين" : "",
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
  const keep = (documentId: number, page: OcrReviewPage) => {
    const pages = reviews.get(documentId) ?? new Map<number, OcrReviewPage>();
    pages.set(page.page, page);
    reviews.set(documentId, pages);
  };
  const step = (entry: MockJob) => {
    entry.timer = null;
    const { job } = entry;
    if (job.state !== "running") return;
    const kind = job.options.kind;
    if (kind.kind === "check") {
      const page = entry.pages[job.finished];
      if (page === undefined) return;
      const found = checks.get(entry.documentId) ?? new Map<number, OcrLayerCheck>();
      found.set(page, checkFor(page));
      checks.set(entry.documentId, found);
      job.finished += 1;
      job.drafted = job.finished;
    } else if (kind.kind === "region") {
      const page = entry.pages[0] ?? 0;
      const [x0, y0, x1, y1] = kind.rect;
      const text = kind.numbers ? "I234" : "مراجعة";
      const before =
        reviews.get(entry.documentId)?.get(page) ?? reviewFor(page) ?? listedEmpty(page);
      const kept = before.words.filter((w) => {
        if (!w.bounds) return true;
        const [cx, cy] = [(w.bounds[0] + w.bounds[2]) / 2, (w.bounds[1] + w.bounds[3]) / 2];
        return !(cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1);
      });
      kept.push(
        word(
          100 + nextJob,
          text,
          0.7,
          { kind: "uncertain", alternatives: [] },
          ["uncertain"],
          [x0, y0, x1, y1],
        ),
      );
      keep(entry.documentId, { ...before, uncertain: before.uncertain + 1, words: kept });
      job.finished = 1;
      job.drafted = 1;
      apply(entry, [[page, "final"]]);
    } else if (job.drafted < entry.pages.length) {
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
      if (review) keep(entry.documentId, review);
      apply(entry, [[page, "final"]]);
    }
    const next = entry.pages[job.finished];
    if (job.finished >= entry.pages.length || next === undefined) {
      job.state = "done";
      job.current = null;
    } else {
      job.current = [next, kind.kind === "check" ? "check" : "quality"];
      entry.timer = setTimeout(() => step(entry), MOCK_STEP_MS);
    }
    emit(entry);
  };
  const start = (entry: MockJob) => {
    entry.job.state = "running";
    const first = entry.pages[entry.job.finished];
    const kind = entry.job.options.kind.kind;
    entry.job.current =
      first === undefined
        ? null
        : [
            first,
            kind === "check"
              ? "check"
              : entry.job.drafted || kind === "region"
                ? "quality"
                : "draft",
          ];
    entry.timer = setTimeout(() => step(entry), MOCK_STEP_MS);
    emit(entry);
  };
  const find = (jobId: number) => {
    const entry = jobs.get(jobId);
    if (!entry) throw { code: "ocr_unknown_job", detail: `no job ${jobId}` };
    return entry;
  };
  const submit = (documentId: number, pages: number[] | undefined, options: OcrJobOptions) => {
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
        options,
        documentIds: [documentId],
      },
      documentId,
      pages: wanted,
      timer: null,
    };
    jobs.set(entry.job.id, entry);
    start(entry);
    return { ...entry.job };
  };

  return {
    startOcr: async (documentId, pages, replace) =>
      submit(documentId, pages, { kind: { kind: "recognize" }, replace: replace ?? false }),
    startOcrRegion: async (documentId, page, rect, language, numbers) =>
      submit(documentId, [page], {
        kind: { kind: "region", rect, language, numbers },
        replace: false,
      }),
    checkTextLayers: async (documentId, pages) =>
      submit(documentId, pages, { kind: { kind: "check" }, replace: false }),
    ocrLayerChecks: async (documentId) =>
      [...(checks.get(documentId)?.values() ?? [])].sort((a, b) => a.page - b.page),
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
    ocrReview: async (documentId, numbers) => list(documentId, numbers ?? false),
    ocrDecide: async (documentId, page, index, action, numbers) => {
      const entry = reviews.get(documentId)?.get(page);
      if (!entry) throw { code: "ocr_review_no_word", detail: "no such page" };
      const writes = decide(entry, index, action);
      const outcome = writes ? { ...hooks.write(documentId), textChanged: true } : null;
      return { review: list(documentId, numbers ?? false), outcome };
    },
    ocrConvert: async (documentId, wanted) => {
      const entries = reviews.get(documentId) ?? new Map<number, OcrReviewPage>();
      const pages = wanted ?? [...entries.keys()];
      if (pages.length === 0) throw { code: "ocr_convert_unrecognized", detail: "" };
      const blocked = pages.filter((page) => {
        const entry = entries.get(page);
        return entry ? needsDecision(listed(entry, true) ?? entry) : false;
      });
      if (blocked.length > 0)
        throw { code: "ocr_convert_unreviewed", detail: blocked.map((p) => p + 1).join(",") };
      for (const page of pages) {
        const entry = entries.get(page);
        if (entry) entry.converted = true;
      }
      return { ...hooks.write(documentId), textChanged: true };
    },
    // A stand-in picture of the area (the real one is the page rendered by the shell).
    ocrCropUrl: (_documentId, page, rect, height) => {
      const width = Math.max(1, Math.round(((rect[2] - rect[0]) / (rect[3] - rect[1])) * height));
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${Math.round(height)}"><rect width="100%" height="100%" fill="#f4f1ea"/><text x="50%" y="60%" font-size="${Math.round(height / 3)}" text-anchor="middle" fill="#333">p${page + 1}</text></svg>`;
      return `data:image/svg+xml,${encodeURIComponent(svg)}`;
    },
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

/** A page with no review data yet (an area read on a page never recognised). */
function listedEmpty(page: number): OcrReviewPage {
  return {
    page,
    uncertain: 0,
    corrected: 0,
    reordered: 0,
    truncated: false,
    confidence: 1,
    lowest: 1,
    undecided: 0,
    open: 0,
    checked: false,
    converted: false,
    stale: false,
    words: [],
  };
}
