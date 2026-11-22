import { create } from "zustand";
import type { OcrModelEvent, OcrModels, OcrPackage, OcrPhase } from "../backend";

/**
 * The OCR models as the UI knows them (ADR 0034): the shell's listing, the
 * downloads running, and the last failure of each. Kept apart from the app
 * store: none of it is a setting, and it changes many times a second while a
 * download runs.
 */

/** A running download. */
export interface OcrJob {
  phase: OcrPhase;
  received: number;
  total: number;
}

/** Why the last download of a package stopped. */
export interface OcrFailure {
  code: string;
  needed: number | null;
  available: number | null;
}

export interface OcrModelsState {
  /** Null until first loaded. */
  models: OcrModels | null;
  /** Running downloads by package id. */
  jobs: Record<string, OcrJob>;
  failures: Record<string, OcrFailure>;
  /** The first-use prompt: OCR was asked for with a required model missing. */
  prompt: boolean;
  setModels(models: OcrModels): void;
  applyEvent(event: OcrModelEvent): void;
  /** A download was asked for: shows it running before the first progress arrives. */
  startJob(id: string): void;
  setPrompt(open: boolean): void;
}

function without<T>(record: Record<string, T>, key: string): Record<string, T> {
  const { [key]: _, ...rest } = record;
  return rest;
}

/** Bytes of `pkg` already downloaded (an install counts whole). */
export function receivedOf(pkg: OcrPackage): number {
  switch (pkg.state.kind) {
    case "partial":
      return pkg.state.received;
    case "installed":
      return pkg.downloadSize;
    default:
      return 0;
  }
}

/** The packages OCR needs that are not installed at their current version. */
export function missingRequired(models: OcrModels | null): OcrPackage[] {
  return models?.packages.filter((pkg) => pkg.required && pkg.state.kind !== "installed") ?? [];
}

/** OCR can run: everything it needs is installed. */
export function ocrReady(models: OcrModels | null): boolean {
  return models?.available === true && missingRequired(models).length === 0;
}

/** Bytes still to download for `packages`. */
export function bytesToDownload(packages: OcrPackage[]): number {
  return packages.reduce(
    (sum, pkg) => sum + (pkg.state.kind === "installed" ? 0 : pkg.downloadSize - receivedOf(pkg)),
    0,
  );
}

function withState(models: OcrModels | null, id: string, change: (pkg: OcrPackage) => OcrPackage) {
  if (!models) return null;
  return { ...models, packages: models.packages.map((pkg) => (pkg.id === id ? change(pkg) : pkg)) };
}

export const useOcrModels = create<OcrModelsState>((set) => ({
  models: null,
  jobs: {},
  failures: {},
  prompt: false,
  setModels(models) {
    set((state) => {
      // Downloads still running in the shell (a reloaded window) keep showing;
      // the listing is the truth for the rest.
      const jobs: Record<string, OcrJob> = {};
      for (const pkg of models.packages) {
        if (!pkg.downloading) continue;
        jobs[pkg.id] = state.jobs[pkg.id] ?? {
          phase: "downloading",
          received: receivedOf(pkg),
          total: pkg.downloadSize,
        };
      }
      return { models, jobs };
    });
  },
  applyEvent(event) {
    set((state) => {
      switch (event.kind) {
        case "progress":
          return {
            jobs: {
              ...state.jobs,
              [event.id]: { phase: event.phase, received: event.received, total: event.total },
            },
            failures: without(state.failures, event.id),
          };
        case "installed":
          return {
            jobs: without(state.jobs, event.id),
            models: withState(state.models, event.id, (pkg) => ({
              ...pkg,
              downloading: false,
              state: { kind: "installed" },
            })),
          };
        case "cancelled": {
          const received = state.jobs[event.id]?.received ?? 0;
          return {
            jobs: without(state.jobs, event.id),
            models: withState(state.models, event.id, (pkg) => ({
              ...pkg,
              downloading: false,
              state: received > 0 ? { kind: "partial", received } : pkg.state,
            })),
          };
        }
        case "failed":
          return {
            jobs: without(state.jobs, event.id),
            failures: {
              ...state.failures,
              [event.id]: { code: event.code, needed: event.needed, available: event.available },
            },
            models: withState(state.models, event.id, (pkg) => ({ ...pkg, downloading: false })),
          };
      }
    });
  },
  startJob(id) {
    set((state) => {
      const pkg = state.models?.packages.find((candidate) => candidate.id === id);
      if (!pkg || state.jobs[id]) return {};
      return {
        jobs: {
          ...state.jobs,
          [id]: { phase: "downloading", received: receivedOf(pkg), total: pkg.downloadSize },
        },
        failures: without(state.failures, id),
      };
    });
  },
  setPrompt(prompt) {
    set({ prompt });
  },
}));
