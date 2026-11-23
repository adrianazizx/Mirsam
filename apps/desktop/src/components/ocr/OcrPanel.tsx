import { useEffect } from "react";
import { ProgressBar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { OcrJob, OcrReviewPage, OcrReviewWord } from "../../backend/types";
import {
  isCommandEnabled,
  OCR_JOB_ERRORS,
  refreshOcrJobs,
  refreshOcrModels,
  runCommand,
} from "../../commands/registry";
import { formatBytes, formatNumber, isolate } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { isLive, jobsOf, useOcrJobs } from "../../state/ocrJobs";
import { bytesToDownload, missingRequired, useOcrModels } from "../../state/ocrModels";
import { Button } from "../ui/Button";
import styles from "./OcrPanel.module.css";

/**
 * The OCR tool's panel in the inspector: whether the model is ready, the ways
 * to recognize text (the document, or this page), the background jobs with
 * their progress (ADR 0035), and the pages-needing-review list, whose flagged
 * words open on their page (spec §5.8).
 */
export function OcrPanel() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const canRecognize = useAppStore((state) => isCommandEnabled("ocr.recognize", state));
  const documentId = useAppStore((state) => selectActiveDocument(state)?.id ?? null);
  const models = useOcrModels((state) => state.models);
  const downloads = useOcrModels((state) => state.jobs);

  useEffect(() => {
    if (useOcrModels.getState().models) return;
    refreshOcrModels().catch((error: unknown) => console.warn("OCR models unavailable", error));
  }, []);
  useEffect(() => {
    if (documentId === null) return;
    refreshOcrJobs(documentId).catch((error: unknown) =>
      console.warn("OCR jobs unavailable", error),
    );
  }, [documentId]);

  const missing = missingRequired(models);
  const download = missing.map((pkg) => downloads[pkg.id]).find((running) => running !== undefined);
  const size = (bytes: number) => formatBytes(bytes, locale);
  let status: string;
  if (!models) status = t("ocrModels.loading");
  else if (!models.available) status = t("ocrModels.unavailable");
  else if (download)
    status = t(`ocrModels.status.${download.phase}`, {
      received: size(download.received),
      total: size(download.total),
    });
  else if (missing.length > 0)
    status = t("ocrPanel.missing", { size: size(bytesToDownload(missing)) });
  else status = t("ocrPanel.ready");

  return (
    <div className={styles.panel} data-testid="ocr-panel">
      <p className={styles.status} role="status">
        {status}
      </p>
      <div className={styles.actions}>
        <Button
          variant="primary"
          isDisabled={!canRecognize}
          onPress={() => void runCommand("ocr.recognize")}
        >
          {t("ocrPanel.recognize")}
        </Button>
        <Button isDisabled={!canRecognize} onPress={() => void runCommand("ocr.recognizePage")}>
          {t("ocrPanel.recognizePage")}
        </Button>
        <Button onPress={() => void runCommand("app.settings")}>{t("ocrPanel.manage")}</Button>
      </div>
      {documentId !== null ? (
        <>
          <Jobs documentId={documentId} />
          <Review documentId={documentId} />
        </>
      ) : null}
    </div>
  );
}

function Jobs({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const all = useOcrJobs((state) => state.jobs);
  const jobs = jobsOf(all, documentId);
  if (jobs.length === 0) return null;
  const n = (value: number) => formatNumber(value, locale);

  const stateText = (job: OcrJob) => {
    if (job.state === "running" && job.current) {
      const [page, pass] = job.current;
      return t(`ocrPanel.job.${pass}`, { page: n(page + 1) });
    }
    if (job.state === "failed") {
      const reason = job.error && OCR_JOB_ERRORS.includes(job.error) ? job.error : "generic";
      return t("ocrPanel.job.failed", { reason: t(`errors.${reason}`) });
    }
    return t(`ocrPanel.job.${job.state}`);
  };

  return (
    <section className={styles.section} aria-label={t("ocrPanel.jobs")}>
      <h3 className={styles.heading}>{t("ocrPanel.jobs")}</h3>
      <ul className={styles.list}>
        {jobs.map((job) => (
          <li key={job.id} className={styles.job} data-testid="ocr-job">
            <ProgressBar
              className={styles.progress}
              value={job.finished}
              maxValue={Math.max(job.pages, 1)}
              aria-label={t("ocrPanel.job.label", { name: isolate(job.name) })}
              valueLabel={t("ocrPanel.job.progress", {
                finished: n(job.finished),
                drafted: n(job.drafted),
                pages: n(job.pages),
              })}
            >
              {({ percentage, valueText }) => (
                <>
                  <span className={styles.status} role="status">
                    {stateText(job)}
                  </span>
                  <span className={styles.track}>
                    <span
                      className={styles.draft}
                      style={{ inlineSize: `${(100 * job.drafted) / Math.max(job.pages, 1)}%` }}
                    />
                    <span className={styles.fill} style={{ inlineSize: `${percentage ?? 0}%` }} />
                  </span>
                  <span className={styles.meta}>{valueText}</span>
                </>
              )}
            </ProgressBar>
            <div className={styles.controls}>
              {isLive(job) ? (
                <Button onPress={() => void runCommand("ocr.pauseJob", { jobId: job.id })}>
                  {t("ocrPanel.job.pause")}
                </Button>
              ) : null}
              {["paused", "cancelled", "failed"].includes(job.state) ? (
                <Button onPress={() => void runCommand("ocr.resumeJob", { jobId: job.id })}>
                  {t("ocrPanel.job.resume")}
                </Button>
              ) : null}
              {isLive(job) || job.state === "paused" ? (
                <Button onPress={() => void runCommand("ocr.cancelJob", { jobId: job.id })}>
                  {t("ocrPanel.job.cancel")}
                </Button>
              ) : null}
              {job.state !== "running" ? (
                <Button onPress={() => void runCommand("ocr.removeJob", { jobId: job.id })}>
                  {t("ocrPanel.job.remove")}
                </Button>
              ) : null}
            </div>
          </li>
        ))}
      </ul>
      {jobs.some((job) => job.drafted > job.finished) ? (
        <p className={styles.note}>{t("ocrPanel.draftNote")}</p>
      ) : null}
    </section>
  );
}

function Review({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const pages = useOcrJobs((state) => state.review[documentId]);
  const focus = useOcrJobs((state) =>
    state.focus?.documentId === documentId ? state.focus : null,
  );
  const canStep = useAppStore((state) => isCommandEnabled("ocr.nextFlag", state));
  const finished = useOcrJobs((state) =>
    jobsOf(state.jobs, documentId).some((job) => job.finished > 0),
  );
  if (!pages || (!finished && pages.length === 0)) return null;
  const n = (value: number) => formatNumber(value, locale);
  const percent = (value: number) =>
    formatNumber(value, locale, { style: "percent", maximumFractionDigits: 0 });

  const counts = (page: OcrReviewPage) =>
    [
      page.uncertain ? t("ocrPanel.review.uncertain", { count: n(page.uncertain) }) : null,
      page.corrected ? t("ocrPanel.review.corrected", { count: n(page.corrected) }) : null,
      page.reordered ? t("ocrPanel.review.reordered", { count: n(page.reordered) }) : null,
      page.truncated ? t("ocrPanel.review.truncated") : null,
    ]
      .filter(Boolean)
      .join(" · ");
  const status = (word: OcrReviewWord) => {
    switch (word.status.kind) {
      case "uncertain":
        return word.status.alternatives.length > 0
          ? t("ocrPanel.review.status.uncertain", {
              alternatives: word.status.alternatives.map(isolate).join(" / "),
            })
          : t("ocrPanel.review.status.uncertainNone");
      case "corrected":
        return t("ocrPanel.review.status.corrected", { read: isolate(word.status.read) });
      case "reordered":
        return t("ocrPanel.review.status.reordered", { read: isolate(word.status.read) });
      default:
        return "";
    }
  };

  return (
    <section
      className={styles.section}
      aria-label={t("ocrPanel.review.heading")}
      data-testid="ocr-review"
    >
      <h3 className={styles.heading}>{t("ocrPanel.review.heading")}</h3>
      {pages.length === 0 ? (
        <p className={styles.note}>{t("ocrPanel.review.empty")}</p>
      ) : (
        <>
          <div className={styles.controls}>
            <Button isDisabled={!canStep} onPress={() => void runCommand("ocr.previousFlag")}>
              {t("ocrPanel.review.previous")}
            </Button>
            <Button isDisabled={!canStep} onPress={() => void runCommand("ocr.nextFlag")}>
              {t("ocrPanel.review.next")}
            </Button>
          </div>
          <ol className={styles.list}>
            {pages.map((page, p) => (
              <li key={page.page} className={styles.page}>
                <div className={styles.pageHeader}>
                  <span className={styles.pageName}>
                    {t("ocrPanel.review.page", { page: n(page.page + 1) })}
                  </span>
                  <span className={styles.meta}>
                    {t("ocrPanel.review.confidence", { value: percent(page.confidence) })}
                  </span>
                </div>
                <span className={styles.meta}>{counts(page)}</span>
                <ul className={styles.words}>
                  {page.words.map((word, w) => {
                    const current = focus?.at[0] === p && focus.at[1] === w;
                    return (
                      <li key={word.index}>
                        <button
                          type="button"
                          className={styles.word}
                          data-kind={word.status.kind}
                          aria-current={current ? "true" : undefined}
                          aria-label={t("ocrPanel.review.word", {
                            word: isolate(word.text),
                            status: status(word),
                            confidence: percent(word.probability),
                          })}
                          onClick={() => void runCommand("ocr.showWord", { reviewAt: [p, w] })}
                        >
                          <bdi className={styles.wordText}>{word.text}</bdi>
                          <span className={styles.meta}>{status(word)}</span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </li>
            ))}
          </ol>
        </>
      )}
    </section>
  );
}
