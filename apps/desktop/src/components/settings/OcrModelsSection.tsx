import { useState } from "react";
import { ProgressBar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { OcrHardware, OcrPackage } from "../../backend";
import { type CommandId, runCommand } from "../../commands/registry";
import { formatBytes, formatMemory, isolate } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { type OcrFailure, type OcrJob, useOcrModels } from "../../state/ocrModels";
import { Button } from "../ui/Button";
import styles from "./OcrModelsSection.module.css";

/**
 * The OCR models in Settings (ADR 0034): each with its role, size, licence and
 * state, and the download, cancel, resume and delete actions, all commands.
 */
export function OcrModelsSection() {
  const { t } = useTranslation();
  const models = useOcrModels((state) => state.models);
  const jobs = useOcrModels((state) => state.jobs);
  const failures = useOcrModels((state) => state.failures);

  if (!models) return <p className={styles.hint}>{t("ocrModels.loading")}</p>;
  if (!models.available) return <p className={styles.hint}>{t("ocrModels.unavailable")}</p>;
  return (
    <div className={styles.section}>
      <p className={styles.hint}>{t("ocrModels.intro")}</p>
      <HardwareLine hardware={models.hardware} />
      <ul className={styles.list}>
        {models.packages.map((pkg) => (
          <PackageRow key={pkg.id} pkg={pkg} job={jobs[pkg.id]} failure={failures[pkg.id]} />
        ))}
      </ul>
    </div>
  );
}

function HardwareLine({ hardware }: { hardware: OcrHardware }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const gpu = hardware.runtimeGpu === "cuda" ? null : hardware.runtimeGpu;
  return (
    <p className={styles.hardware} data-testid="ocr-hardware">
      {t(`ocrModels.hardware.${gpu ?? "none"}`)}
      {hardware.memory
        ? ` ${t("ocrModels.hardware.memory", { size: formatMemory(hardware.memory, locale) })}`
        : null}
    </p>
  );
}

function PackageRow({
  pkg,
  job,
  failure,
}: {
  pkg: OcrPackage;
  job: OcrJob | undefined;
  failure: OcrFailure | undefined;
}) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const [confirming, setConfirming] = useState(false);
  const size = (bytes: number) => formatBytes(bytes, locale);
  const run = (command: CommandId) => void runCommand(command, { modelId: pkg.id });

  const jobText = job
    ? t(`ocrModels.status.${job.phase}`, { received: size(job.received), total: size(job.total) })
    : "";
  const stateText =
    pkg.state.kind === "partial"
      ? t("ocrModels.status.partial", {
          received: size(pkg.state.received),
          total: size(pkg.downloadSize),
        })
      : t(`ocrModels.status.${pkg.state.kind}`);
  const failureText = failure
    ? failure.code === "model_disk_full" && failure.needed !== null && failure.available !== null
      ? t("ocrModels.diskFull", {
          needed: size(failure.needed),
          available: size(failure.available),
        })
      : t(`errors.${failure.code}`, { defaultValue: t("errors.generic") })
    : null;

  let actions: React.ReactNode;
  if (job) {
    actions = (
      <Button onPress={() => run("ocrModels.cancel")}>{t("ocrModels.actions.cancel")}</Button>
    );
  } else if (confirming) {
    actions = (
      <>
        <Button
          variant="primary"
          onPress={() => {
            setConfirming(false);
            run("ocrModels.delete");
          }}
        >
          {t("ocrModels.actions.delete")}
        </Button>
        <Button autoFocus onPress={() => setConfirming(false)}>
          {t("ocrModels.actions.keep")}
        </Button>
      </>
    );
  } else {
    const primary = {
      notInstalled: "download",
      partial: "resume",
      outdated: "update",
      installed: null,
    }[pkg.state.kind];
    actions = (
      <>
        {primary ? (
          <Button variant="primary" onPress={() => run("ocrModels.download")}>
            {t(`ocrModels.actions.${primary}`)}
          </Button>
        ) : null}
        {pkg.state.kind !== "notInstalled" ? (
          <Button onPress={() => setConfirming(true)}>{t("ocrModels.actions.delete")}</Button>
        ) : null}
      </>
    );
  }

  return (
    <li className={styles.row} data-testid={`ocr-model-${pkg.id}`}>
      <div className={styles.details}>
        {/* A proper name in Latin script, whatever the UI's direction. */}
        <bdi className={styles.name}>{pkg.name}</bdi>
        <span className={styles.role}>{t(`ocrModels.role.${pkg.role}`)}</span>
        <span className={styles.meta}>
          {t("ocrModels.size", { size: size(pkg.downloadSize) })}
          {" · "}
          {t("ocrModels.licence")} <bdi>{pkg.licence}</bdi>
        </span>
        {job ? (
          <ProgressBar
            className={styles.progress}
            value={job.received}
            maxValue={Math.max(job.total, 1)}
            aria-label={t("ocrModels.progress", { name: isolate(pkg.name) })}
            valueLabel={jobText}
            isIndeterminate={job.phase === "unpacking" || job.phase === "installing"}
          >
            {({ percentage, isIndeterminate }) => (
              <>
                <span className={styles.status}>{jobText}</span>
                <span className={styles.track}>
                  <span
                    className={styles.fill}
                    data-indeterminate={isIndeterminate ? "" : undefined}
                    style={{ inlineSize: isIndeterminate ? undefined : `${percentage ?? 0}%` }}
                  />
                </span>
              </>
            )}
          </ProgressBar>
        ) : (
          <span className={styles.status}>{stateText}</span>
        )}
        {failureText ? (
          <span className={styles.failure} role="alert">
            {failureText}
          </span>
        ) : null}
        {confirming && !job ? (
          <span className={styles.confirm}>
            {t("ocrModels.confirmDelete", {
              name: isolate(pkg.name),
              size: size(pkg.downloadSize),
            })}
          </span>
        ) : null}
      </div>
      <div className={styles.actions}>{actions}</div>
    </li>
  );
}
