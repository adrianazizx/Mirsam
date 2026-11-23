import { create } from "zustand";
import { backend, type CrashReportInfo, isBackendError } from "../backend";

/**
 * Crash reports in the UI (ADR 0061): the reports offered after a crash, the
 * ones kept (Settings), the setting, the last commands run, and the capture
 * of errors nothing else handled.
 */

/** `ask` (the default): keep reports and ask after a crash. `off`: keep none. */
export type CrashSetting = "ask" | "off";
export const CRASH_SETTING = "crashReports";

/** What the last action on a report did, shown beside it. */
export type CrashStatus = "copied" | "saved" | "opened" | "openedShortened" | "failed";

interface CrashReportsState {
  /** Reports offered after a crash; the prompt shows while there are any. */
  prompt: CrashReportInfo[];
  /** Every kept report, newest first (Settings). */
  kept: CrashReportInfo[];
  /** Where reports can be filed on GitHub (`owner/name`), if anywhere. */
  github: string | null;
  setting: CrashSetting;
  status: Record<string, CrashStatus>;
  setPrompt(reports: CrashReportInfo[]): void;
  setKept(reports: CrashReportInfo[], github: string | null): void;
  /** A report deleted: gone from the prompt and the list. */
  forget(id: string): void;
  setStatus(id: string, status: CrashStatus): void;
  applySetting(value: string | undefined): void;
}

export const useCrashReports = create<CrashReportsState>()((set) => ({
  prompt: [],
  kept: [],
  github: null,
  setting: "ask",
  status: {},
  setPrompt: (prompt) => set({ prompt }),
  setKept: (kept, github) => set({ kept, github }),
  forget: (id) =>
    set((state) => ({
      prompt: state.prompt.filter((report) => report.id !== id),
      kept: state.kept.filter((report) => report.id !== id),
    })),
  setStatus: (id, status) => set((state) => ({ status: { ...state.status, [id]: status } })),
  applySetting: (value) => set({ setting: value === "off" ? "off" : "ask" }),
}));

/** Reloads the kept reports (when Settings opens, after a change). */
export async function refreshCrashReports(): Promise<void> {
  const { reports, github } = await backend.crashReports();
  useCrashReports.getState().setKept(reports, github);
}

/** The text of a report the UI holds. */
export function crashReportText(id: string): string | undefined {
  const { prompt, kept } = useCrashReports.getState();
  return [...prompt, ...kept].find((report) => report.id === id)?.text;
}

// The last commands run: ids only, never their arguments (a typed page
// number, a search, note text). Repeats are folded: `view.zoomIn ×4`.
const MAX_COMMANDS = 30;
const trail: { id: string; count: number }[] = [];

/** Records that a command ran. */
export function recordCommand(id: string): void {
  const last = trail.at(-1);
  if (last?.id === id) {
    last.count += 1;
    return;
  }
  trail.push({ id, count: 1 });
  if (trail.length > MAX_COMMANDS) trail.shift();
}

/** The last commands run, oldest first. */
export function recentCommands(): string[] {
  return trail.map(({ id, count }) => (count > 1 ? `${id} ×${count}` : id));
}

/** Messages the browser engine raises that are not errors of the app. */
const IGNORED = [/ResizeObserver loop/];
/** Errors already reported this session (message and first frame). */
const reported = new Set<string>();

/**
 * Keeps an error nothing handled as a crash report. Only the error's name,
 * message and stack go to the shell, which redacts them; a thrown object
 * that is not an `Error` is never turned into text (it may hold document
 * data). `extraStack` is React's component stack, when there is one.
 */
export async function reportUiError(error: unknown, extraStack = ""): Promise<string | null> {
  const { message, stack } = describe(error);
  if (IGNORED.some((pattern) => pattern.test(message))) return null;
  const key = `${message}\n${stack.split("\n")[0] ?? ""}`;
  if (reported.has(key)) return null;
  reported.add(key);
  const frames = extraStack ? `${stack}\n${extraStack}` : stack;
  try {
    return await backend.recordUiError(message, frames, recentCommands());
  } catch {
    return null;
  }
}

function describe(error: unknown): { message: string; stack: string } {
  if (error instanceof Error) {
    return { message: `${error.name}: ${error.message}`, stack: error.stack ?? "" };
  }
  // A backend error's code is a stable identifier; its detail may name a file.
  if (isBackendError(error))
    return { message: `unhandled backend error: ${error.code}`, stack: "" };
  if (typeof error === "string") return { message: error, stack: "" };
  return { message: `thrown value of type ${typeof error}`, stack: "" };
}

/** Listens for errors and rejections nothing else handled. Returns a stop function. */
export function captureUncaughtErrors(): () => void {
  const onError = (event: ErrorEvent) => {
    void reportUiError(event.error ?? event.message);
  };
  const onRejection = (event: PromiseRejectionEvent) => {
    void reportUiError(event.reason);
  };
  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);
  return () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
  };
}

/** For tests: forget the commands and errors seen. */
export function resetCrashCapture(): void {
  trail.length = 0;
  reported.clear();
}
