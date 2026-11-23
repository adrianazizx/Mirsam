import type { Backend, CrashReportInfo } from "./types";

/**
 * Crash reports in the mock backend (ADR 0061). Tests set them; in the
 * browser, `?crash` in the address starts with one, to see the prompt.
 */
let reports: (CrashReportInfo & { offered: boolean })[] = [];
let recorded: { message: string; stack: string; commands: string[] }[] = [];

export const SAMPLE_CRASH_TEXT = `Mirsam crash report
Kind: the app stopped (Rust panic)
Report: 1763894530-3fa2b9c1
Time: 2026-11-23 10:42 UTC
Version: 0.0.1 (release, aarch64)
System: macOS 26.1 (25B78)

What happened:
  index out of bounds: the len is 3 but the index is 7
  at crates/safha-core/src/text/layout.rs:120:9, thread safha-render

Stack (innermost first):
  safha_core::text::layout::visual_order
  safha_core::text::page_text
  safha_core::render::worker

Last commands (oldest first):
  open_document_dialog
  page_sizes
  edit_text_block

No document contents, file names or paths are included.
`;

export function sampleCrashReport(id = "1763894530-3fa2b9c1"): CrashReportInfo {
  return {
    id,
    kind: "panic",
    created: 1_763_894_530,
    summary: "index out of bounds: the len is 3 but the index is 7",
    text: SAMPLE_CRASH_TEXT.replace("1763894530-3fa2b9c1", id),
  };
}

/** For tests: the reports the mock keeps, none offered yet. */
export function setMockCrashReports(list: CrashReportInfo[]): void {
  reports = list.map((report) => ({ ...report, offered: false }));
  recorded = [];
}

/** For tests: the UI errors handed to the mock. */
export function recordedUiErrors() {
  return recorded;
}

if (typeof location !== "undefined" && new URLSearchParams(location.search).has("crash")) {
  setMockCrashReports([sampleCrashReport()]);
}

const strip = ({ offered: _, ...report }: CrashReportInfo & { offered: boolean }) => report;

export const mockCrashBackend: Pick<
  Backend,
  | "crashReports"
  | "newCrashReports"
  | "onCrashReportsAdded"
  | "deleteCrashReport"
  | "saveCrashReport"
  | "openCrashIssue"
  | "recordUiError"
  | "selftestMark"
  | "selftestCrash"
  | "selftestCrashState"
> = {
  crashReports: async () => ({ reports: reports.map(strip), github: "adrianazizx/Mirsam" }),
  newCrashReports: async () => {
    const fresh = reports.filter((report) => !report.offered);
    for (const report of fresh) report.offered = true;
    return fresh.map(strip);
  },
  onCrashReportsAdded: async () => () => {},
  deleteCrashReport: async (id) => {
    reports = reports.filter((report) => report.id !== id);
  },
  saveCrashReport: async () => true,
  openCrashIssue: async () => false,
  recordUiError: async (message, stack, commands) => {
    recorded.push({ message, stack, commands });
    return `mock-${recorded.length}`;
  },
  selftestMark: async () => {},
  selftestCrash: async () => {},
  selftestCrashState: async () => null,
};
