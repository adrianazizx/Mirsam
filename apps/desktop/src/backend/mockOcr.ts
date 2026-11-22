import type {
  Backend,
  OcrHardware,
  OcrModelEvent,
  OcrModels,
  OcrPackage,
  OcrPackageState,
} from "./types";

/**
 * The OCR model downloads in the browser stand-in (ADR 0034): the shell's
 * catalog with its real sizes, and downloads that take a few steps of a
 * timer, so the settings panel and the first-use prompt can be reviewed and
 * tested without Tauri. Nothing is fetched.
 */

/** A mock download arrives in this many pieces, this far apart (slow enough to watch in `pnpm dev`). */
export const MOCK_DOWNLOAD_STEPS = 6;
export const MOCK_STEP_MS = import.meta.env.MODE === "test" ? 30 : 700;

type OcrMethods = Pick<
  Backend,
  | "ocrModels"
  | "downloadOcrModel"
  | "cancelOcrModelDownload"
  | "deleteOcrModel"
  | "onOcrModelProgress"
>;

function catalog(): OcrPackage[] {
  const base = { state: { kind: "notInstalled" } as OcrPackageState, downloading: false };
  return [
    {
      ...base,
      id: "nuqta-4b",
      role: "primary",
      name: "Nuqta-Arabic-OCR 4B",
      version: "f4fdd78",
      licence: "Apache-2.0",
      homepage: "https://huggingface.co/SalahALHaismawi/Nuqta-Arabic-OCR-4B-GGUF",
      downloadSize: 5_511_442_432,
      installedSize: 5_511_442_432,
      required: true,
    },
    {
      ...base,
      id: "nuqta-0.8b",
      role: "ensemble",
      name: "Nuqta-Arabic-OCR 0.8B",
      version: "4f72054",
      licence: "Apache-2.0",
      homepage: "https://huggingface.co/SalahALHaismawi/Nuqta-Arabic-OCR-0.8B-GGUF",
      downloadSize: 1_077_594_432,
      installedSize: 1_077_594_432,
      required: false,
    },
  ];
}

const APPLE_SILICON: OcrHardware = {
  os: "macos",
  arch: "aarch64",
  accelerators: ["metal"],
  runtimeGpu: "metal",
  memory: 36 * 2 ** 30,
};

let packages = catalog();
let hardware = APPLE_SILICON;
let failNext: { code: string; needed: number | null; available: number | null } | null = null;
const running = new Map<string, ReturnType<typeof setTimeout>>();
const listeners = new Set<(event: OcrModelEvent) => void>();

function emit(event: OcrModelEvent) {
  for (const listener of listeners) listener(event);
}

function setState(id: string, state: OcrPackageState) {
  packages = packages.map((pkg) => (pkg.id === id ? { ...pkg, state } : pkg));
}

function receivedOf(pkg: OcrPackage): number {
  return pkg.state.kind === "partial" ? pkg.state.received : 0;
}

/** Test hook: starts over with nothing installed, or with the given states and hardware. */
export function resetMockOcr(options?: {
  states?: Record<string, OcrPackageState>;
  hardware?: Partial<OcrHardware>;
}) {
  for (const timer of running.values()) clearTimeout(timer);
  running.clear();
  failNext = null;
  packages = catalog().map((pkg) => ({ ...pkg, state: options?.states?.[pkg.id] ?? pkg.state }));
  hardware = { ...APPLE_SILICON, ...options?.hardware };
}

/** Test hook: the next download fails at its first piece with this error. */
export function failNextMockDownload(
  code: string,
  sizes: { needed: number; available: number } | null = null,
) {
  failNext = { code, needed: sizes?.needed ?? null, available: sizes?.available ?? null };
}

export const mockOcrBackend: OcrMethods = {
  ocrModels: async (): Promise<OcrModels> => ({
    packages: packages.map((pkg) => ({ ...pkg, downloading: running.has(pkg.id) })),
    hardware,
    available: true,
  }),
  downloadOcrModel: async (id) => {
    const pkg = packages.find((candidate) => candidate.id === id);
    if (!pkg) throw { code: "model_unknown", detail: id };
    if (running.has(id)) throw { code: "model_busy", detail: id };
    if (pkg.state.kind === "installed") return;
    const total = pkg.downloadSize;
    let received = receivedOf(pkg);
    const failure = failNext;
    failNext = null;
    const step = () => {
      if (failure) {
        running.delete(id);
        emit({ id, kind: "failed", ...failure });
        return;
      }
      received = Math.min(total, received + Math.ceil(total / MOCK_DOWNLOAD_STEPS));
      setState(id, { kind: "partial", received });
      emit({ id, kind: "progress", phase: "downloading", received, total });
      if (received < total) {
        running.set(id, setTimeout(step, MOCK_STEP_MS));
        return;
      }
      running.delete(id);
      setState(id, { kind: "installed" });
      emit({ id, kind: "progress", phase: "installing", received: total, total });
      emit({ id, kind: "installed" });
    };
    running.set(id, setTimeout(step, MOCK_STEP_MS));
  },
  cancelOcrModelDownload: async (id) => {
    const timer = running.get(id);
    if (timer === undefined) return;
    clearTimeout(timer);
    running.delete(id);
    emit({ id, kind: "cancelled" });
  },
  deleteOcrModel: async (id) => {
    if (running.has(id)) throw { code: "model_busy", detail: id };
    if (!packages.some((pkg) => pkg.id === id)) throw { code: "model_unknown", detail: id };
    setState(id, { kind: "notInstalled" });
  },
  onOcrModelProgress: async (listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
};
