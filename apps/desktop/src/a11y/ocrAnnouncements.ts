import type { OcrJob, OcrLayerCheck, OcrModelEvent } from "../backend/types";
import { formatBytes, formatNumber, i18n, isolate } from "../i18n";
import { announce } from "../state/announcer";
import { useAppStore } from "../state/appStore";
import { useOcrModels } from "../state/ocrModels";

/**
 * What screen readers hear about OCR running in the background (ADR 0070's
 * second pass): a job starting, pausing, finishing or failing, the verdict of
 * a layer check, and a model download ending. Progress itself is not spoken
 * (the progress bars say it when asked); only the changes nobody is looking at.
 */

const lng = () => useAppStore.getState().locale;

function jobLabel(job: OcrJob): string {
  const name = isolate(job.name);
  switch (job.options.kind.kind) {
    case "region":
      return i18n.t("ocrPanel.job.labelRegion", { name, lng: lng() });
    case "check":
      return i18n.t("ocrPanel.job.labelCheck", { name, lng: lng() });
    default:
      return job.options.replace
        ? i18n.t("ocrPanel.job.labelReplace", { name, lng: lng() })
        : i18n.t("ocrPanel.job.label", { name, lng: lng() });
  }
}

/** Announces how `next` differs from `previous` (the same job as last reported, if any). */
export function announceOcrJob(previous: OcrJob | undefined, next: OcrJob): void {
  const label = jobLabel(next);
  const say = (state: string, politeness: "polite" | "assertive" = "polite") =>
    announce(i18n.t("ocrPanel.announce.job", { label, state, lng: lng() }), politeness);
  if (!previous) {
    if (next.state === "queued" || next.state === "running")
      say(i18n.t("ocrPanel.announce.started", { lng: lng() }));
    return;
  }
  if (previous.state === next.state) return;
  switch (next.state) {
    case "paused":
      say(i18n.t("ocrPanel.job.paused", { lng: lng() }));
      break;
    case "cancelled":
      say(i18n.t("ocrPanel.job.cancelled", { lng: lng() }));
      break;
    case "failed": {
      const generic = i18n.t("errors.generic", { lng: lng() });
      const why = next.error
        ? i18n.t(`errors.${next.error}`, { defaultValue: generic, lng: lng() })
        : generic;
      say(i18n.t("ocrPanel.job.failed", { reason: why, lng: lng() }), "assertive");
      break;
    }
    case "done":
      // A check's verdict is said once its results are in (announceLayerChecks).
      if (next.options.kind.kind !== "check") say(i18n.t("ocrPanel.job.done", { lng: lng() }));
      break;
    case "running":
      if (previous.state === "paused") say(i18n.t("ocrPanel.announce.resumed", { lng: lng() }));
      break;
    default:
      break;
  }
}

/** A finished layer check's verdict: how many pages have a poor text layer. */
export function announceLayerChecks(checks: OcrLayerCheck[]): void {
  const n = (value: number) => formatNumber(value, lng());
  const poor = checks.filter((check) => check.verdict === "poor").length;
  const good = checks.filter((check) => check.verdict === "good").length;
  announce(
    poor > 0
      ? i18n.t("ocrPanel.layers.found", { poor: n(poor), good: n(good), lng: lng() })
      : i18n.t("ocrPanel.layers.none", { good: n(good), lng: lng() }),
  );
}

/** A model download's end: installed, or why it stopped. */
export function announceOcrModel(event: OcrModelEvent): void {
  if (event.kind === "progress") return;
  const pkg = useOcrModels.getState().models?.packages.find((entry) => entry.id === event.id);
  const name = isolate(pkg?.name ?? event.id);
  if (event.kind === "installed") {
    announce(i18n.t("ocrModels.announce.installed", { name, lng: lng() }));
  } else if (event.kind === "cancelled") {
    announce(i18n.t("ocrModels.announce.cancelled", { name, lng: lng() }));
  } else {
    // As the row says it (OcrModelsSection), sizes included for a full disk.
    const reason =
      event.code === "model_disk_full" && event.needed !== null && event.available !== null
        ? i18n.t("ocrModels.diskFull", {
            needed: formatBytes(event.needed, lng()),
            available: formatBytes(event.available, lng()),
            lng: lng(),
          })
        : i18n.t(`errors.${event.code}`, {
            defaultValue: i18n.t("errors.generic", { lng: lng() }),
            lng: lng(),
          });
    announce(i18n.t("ocrModels.announce.failed", { name, reason, lng: lng() }), "assertive");
  }
}
