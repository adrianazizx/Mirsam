import { beforeEach, describe, expect, it } from "vitest";
import type { OcrModels, OcrPackage } from "../backend";
import { formatBytes, formatMemory, isolate } from "../i18n";
import { bytesToDownload, missingRequired, ocrReady, receivedOf, useOcrModels } from "./ocrModels";

function pkg(id: string, patch: Partial<OcrPackage> = {}): OcrPackage {
  return {
    id,
    role: "primary",
    name: id,
    version: "v1",
    licence: "Apache-2.0",
    homepage: "",
    downloadSize: 1000,
    installedSize: 1000,
    state: { kind: "notInstalled" },
    required: true,
    downloading: false,
    ...patch,
  };
}

function listing(packages: OcrPackage[]): OcrModels {
  return {
    packages,
    hardware: { os: "linux", arch: "x86_64", accelerators: [], runtimeGpu: null, memory: null },
    available: true,
  };
}

const store = () => useOcrModels.getState();

beforeEach(() => {
  useOcrModels.setState({ models: null, jobs: {}, failures: {}, prompt: false });
});

describe("OCR model helpers", () => {
  it("knows what OCR still needs and how much is left to download", () => {
    const models = listing([
      pkg("a", { state: { kind: "partial", received: 400 } }),
      pkg("b", { required: false }),
      pkg("c", { state: { kind: "installed" } }),
      pkg("d", { state: { kind: "outdated", version: "v0" } }),
    ]);
    expect(missingRequired(models).map((p) => p.id)).toEqual(["a", "d"]);
    expect(bytesToDownload(missingRequired(models))).toBe(600 + 1000);
    expect(ocrReady(models)).toBe(false);
    expect(ocrReady(null)).toBe(false);
    expect(missingRequired(null)).toEqual([]);
    expect(
      ocrReady(
        listing([pkg("x", { state: { kind: "installed" } }), pkg("y", { required: false })]),
      ),
    ).toBe(true);
    expect(ocrReady({ ...listing([]), available: false })).toBe(false);
    expect(receivedOf(pkg("i", { state: { kind: "installed" } }))).toBe(1000);
  });

  it("formats sizes in both languages, in the Arabic UI's digits", () => {
    expect(formatBytes(5_511_442_432, "en")).toBe("5.5 GB");
    expect(formatBytes(204_986_688, "en")).toBe("205 MB");
    expect(formatBytes(1_500_000, "en")).toBe("1.5 MB");
    expect(formatMemory(36 * 2 ** 30, "en")).toBe("36 GB");
    const arabic = formatBytes(5_511_442_432, "ar");
    expect(arabic).toContain("٥٫٥");
    expect(arabic).not.toMatch(/[0-9]/);
    expect(isolate("Nuqta 4B")).toBe("⁨Nuqta 4B⁩");
  });
});

describe("OCR model download state", () => {
  it("follows progress, cancel, failure and success", () => {
    store().setModels(listing([pkg("a"), pkg("b", { required: false })]));
    store().startJob("a");
    expect(store().jobs.a).toEqual({ phase: "downloading", received: 0, total: 1000 });

    store().applyEvent({
      id: "a",
      kind: "progress",
      phase: "downloading",
      received: 300,
      total: 1000,
    });
    expect(store().jobs.a?.received).toBe(300);
    store().applyEvent({ id: "a", kind: "cancelled" });
    expect(store().jobs.a).toBeUndefined();
    expect(store().models?.packages[0]?.state).toEqual({ kind: "partial", received: 300 });

    store().startJob("a");
    expect(store().jobs.a?.received).toBe(300);
    store().applyEvent({
      id: "a",
      kind: "failed",
      code: "model_disk_full",
      needed: 9,
      available: 1,
    });
    expect(store().failures.a).toEqual({ code: "model_disk_full", needed: 9, available: 1 });
    expect(store().jobs.a).toBeUndefined();

    // A new attempt clears the old failure.
    store().applyEvent({
      id: "a",
      kind: "progress",
      phase: "checking",
      received: 300,
      total: 1000,
    });
    expect(store().failures.a).toBeUndefined();
    store().applyEvent({ id: "a", kind: "installed" });
    expect(store().models?.packages[0]?.state).toEqual({ kind: "installed" });
    expect(store().jobs).toEqual({});
  });

  it("keeps showing downloads the shell still runs after a reload", () => {
    store().setModels(
      listing([
        pkg("a", { downloading: true, state: { kind: "partial", received: 250 } }),
        pkg("b"),
      ]),
    );
    expect(store().jobs).toEqual({ a: { phase: "downloading", received: 250, total: 1000 } });
    // A listing without it (it finished meanwhile) drops the job.
    store().setModels(listing([pkg("a", { state: { kind: "installed" } }), pkg("b")]));
    expect(store().jobs).toEqual({});
  });
});
