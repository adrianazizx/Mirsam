import { create } from "zustand";
import type { ToolId } from "./appStore";

/**
 * Onboarding (spec §12, M7): the first-run guide and the one-time hints shown
 * the first time a tool is opened. What the user has seen is kept in the
 * shell's settings store (two keys on its allowlist), so it is the same in
 * every window and survives a reinstall of the UI.
 */

/** Settings keys. Allowlisted in `src-tauri/src/commands.rs`. */
export const GUIDE_SETTING = "onboarding.guide";
export const HINTS_SETTING = "onboarding.hints";

/** The guide's steps, in order. */
export const GUIDE_STEPS = ["open", "tools", "language", "ocr"] as const;
export type GuideStep = (typeof GUIDE_STEPS)[number];

/** The tools that explain themselves once, the first time they are opened. */
export const HINT_TOOLS = ["ocr", "sign", "edit"] as const satisfies readonly ToolId[];
export type HintTool = (typeof HINT_TOOLS)[number];

/**
 * `new`: never finished nor skipped, so the guide opens on an empty window.
 * `done` or `skipped`: it never opens by itself again (Help reopens it).
 */
export type GuideStatus = "new" | "done" | "skipped";

export interface OnboardingState {
  /** The stored state was read; nothing opens by itself before that. */
  loaded: boolean;
  guide: GuideStatus;
  /** Hints already shown, each at most once. */
  hintsSeen: HintTool[];
  /** The guide is open, at this step (an index into `GUIDE_STEPS`). */
  guideOpen: boolean;
  step: number;
  /** The hint on screen now, if any. */
  hint: HintTool | null;
  openGuide(): void;
  setStep(step: number): void;
  /** Closes the guide, remembering it as finished or skipped. */
  closeGuide(outcome: "done" | "skipped"): void;
  /** Shows a tool's hint and remembers that it was shown. */
  showHint(tool: HintTool): void;
  dismissHint(): void;
  /** Applies the stored settings at startup. */
  hydrate(settings: Record<string, string | undefined>): void;
  /** A setting another window stored. */
  applySetting(key: string, value: string): void;
}

export function parseGuide(value: string | undefined): GuideStatus {
  return value === "done" || value === "skipped" ? value : "new";
}

export function parseHints(value: string | undefined): HintTool[] {
  const listed = new Set((value ?? "").split(","));
  return HINT_TOOLS.filter((tool) => listed.has(tool));
}

/** Where the state is written; the backend's settings store in the app (set by `App`). */
let write: (key: string, value: string) => void = () => {};
export function setOnboardingWriter(writer: (key: string, value: string) => void): void {
  write = writer;
}

/**
 * Whether the guide opens by itself and hints appear. Off in unit tests
 * (`src/test/setup.ts`), whose screens would otherwise start under the guide;
 * the onboarding tests turn it on.
 */
let automatic = true;
export function setOnboardingAutomatic(on: boolean): void {
  automatic = on;
}
export function onboardingAutomatic(): boolean {
  return automatic;
}

export const useOnboarding = create<OnboardingState>()((set, get) => ({
  loaded: false,
  guide: "new",
  hintsSeen: [],
  guideOpen: false,
  step: 0,
  hint: null,
  openGuide: () => set({ guideOpen: true, step: 0, hint: null }),
  setStep: (step) => set({ step: Math.max(0, Math.min(GUIDE_STEPS.length - 1, step)) }),
  closeGuide(outcome) {
    // Finishing is what counts: skipping a guide reopened later keeps "done".
    const guide = get().guide === "done" ? "done" : outcome;
    set({ guideOpen: false, step: 0, guide });
    write(GUIDE_SETTING, guide);
  },
  showHint(tool) {
    const hintsSeen = get().hintsSeen.includes(tool) ? get().hintsSeen : [...get().hintsSeen, tool];
    set({ hint: tool, hintsSeen });
    write(HINTS_SETTING, hintsSeen.join(","));
  },
  dismissHint: () => set({ hint: null }),
  hydrate(settings) {
    set({
      loaded: true,
      guide: parseGuide(settings[GUIDE_SETTING]),
      hintsSeen: parseHints(settings[HINTS_SETTING]),
    });
  },
  applySetting(key, value) {
    if (key === GUIDE_SETTING) set({ guide: parseGuide(value) });
    if (key === HINTS_SETTING) {
      // A hint another window showed counts here too; one showing here stays.
      set({ hintsSeen: parseHints(value) });
    }
  },
}));

/** True for the tools that have a hint. */
export function isHintTool(tool: ToolId): tool is HintTool {
  return (HINT_TOOLS as readonly ToolId[]).includes(tool);
}

/**
 * The hint to show now, if any: the active tool's, the first time it is
 * opened with a document on screen, once the stored state is known and the
 * guide is not in the way.
 */
export function hintDue(
  tool: ToolId,
  hasDocument: boolean,
  state: Pick<OnboardingState, "loaded" | "hintsSeen" | "guideOpen">,
): HintTool | null {
  if (!automatic || !state.loaded || state.guideOpen || !hasDocument) return null;
  if (!isHintTool(tool) || state.hintsSeen.includes(tool)) return null;
  return tool;
}

/**
 * Whether the guide should open by itself at startup: never finished nor
 * skipped, the window starts empty, and nothing else asks for attention first
 * (recovered changes, a crash report, a password). It waits for a later
 * launch otherwise.
 */
export function guideDue(
  state: Pick<OnboardingState, "loaded" | "guide" | "guideOpen">,
  window: { documents: number; busy: boolean },
): boolean {
  return (
    automatic &&
    state.loaded &&
    state.guide === "new" &&
    !state.guideOpen &&
    window.documents === 0 &&
    !window.busy
  );
}
