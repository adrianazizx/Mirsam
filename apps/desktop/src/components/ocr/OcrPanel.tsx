import { useEffect, useRef, useState } from "react";
import {
  Checkbox,
  type Key,
  ProgressBar,
  ToggleButton,
  ToggleButtonGroup,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { backend } from "../../backend";
import type {
  OcrJob,
  OcrLanguage,
  OcrLayerCheck,
  OcrReviewPage,
  OcrReviewWord,
} from "../../backend/types";
import {
  isCommandEnabled,
  OCR_JOB_ERRORS,
  refreshOcrJobs,
  refreshOcrModels,
  runCommand,
  shortcutLabel,
} from "../../commands/registry";
import { formatBytes, formatNumber, isolate } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { isLive, jobsOf, reviewQueue, useOcrJobs, wordAt } from "../../state/ocrJobs";
import { bytesToDownload, missingRequired, useOcrModels } from "../../state/ocrModels";
import { Button } from "../ui/Button";
import styles from "./OcrPanel.module.css";

/** How tall the image beside a word is rendered, in device pixels (the panel scales it). */
const CROP_PIXELS = 160;
const LANGUAGES: OcrLanguage[] = ["auto", "arabic", "english"];

/**
 * The OCR tool's panel in the inspector: whether the model is ready, the ways
 * to recognize text (the document, or this page), the background jobs with
 * their progress (ADR 0035), review mode and the pages-needing-review list
 * (spec §5.8, ADR 0037), reading an area again and replacing another tool's
 * poor text layer (ADR 0038), and converting pages to editable text (ADR 0039).
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
          <ReviewCard documentId={documentId} />
          <Jobs documentId={documentId} />
          <Review documentId={documentId} />
          <Region documentId={documentId} />
          <Layers documentId={documentId} />
          <Convert />
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
      if (job.options.kind.kind === "region")
        return t("ocrPanel.job.region", { page: n(page + 1) });
      return t(`ocrPanel.job.${pass}`, { page: n(page + 1) });
    }
    if (job.state === "failed") {
      const reason = job.error && OCR_JOB_ERRORS.includes(job.error) ? job.error : "generic";
      return t("ocrPanel.job.failed", { reason: t(`errors.${reason}`) });
    }
    return t(`ocrPanel.job.${job.state}`);
  };
  const label = (job: OcrJob) => {
    const name = isolate(job.name);
    switch (job.options.kind.kind) {
      case "region":
        return t("ocrPanel.job.labelRegion", { name });
      case "check":
        return t("ocrPanel.job.labelCheck", { name });
      default:
        return job.options.replace
          ? t("ocrPanel.job.labelReplace", { name })
          : t("ocrPanel.job.label", { name });
    }
  };
  const progress = (job: OcrJob) =>
    job.options.kind.kind === "check"
      ? t("ocrPanel.job.checked", { finished: n(job.finished), pages: n(job.pages) })
      : t("ocrPanel.job.progress", {
          finished: n(job.finished),
          drafted: n(job.drafted),
          pages: n(job.pages),
        });

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
              aria-label={label(job)}
              valueLabel={progress(job)}
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
      {jobs.some((job) => job.drafted > job.finished && job.options.kind.kind === "recognize") ? (
        <p className={styles.note}>{t("ocrPanel.draftNote")}</p>
      ) : null}
    </section>
  );
}

/** What the pipeline did to a word, for its label. */
function useStatusText() {
  const { t } = useTranslation();
  return (word: OcrReviewWord) => {
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
        return word.reasons.includes("lowConfidence")
          ? t("ocrPanel.review.status.lowConfidence")
          : t("ocrPanel.review.status.number");
    }
  };
}

/** What a person decided about a word, for its label ("" when nothing yet). */
function useDecisionText() {
  const { t } = useTranslation();
  return (word: OcrReviewWord) => {
    switch (word.review.kind) {
      case "accepted":
        return t("ocrPanel.review.decided.accepted");
      case "edited":
        return t("ocrPanel.review.decided.edited", { read: isolate(word.review.read) });
      case "checked":
        return t("ocrPanel.review.decided.checked");
      default:
        return "";
    }
  };
}

/**
 * Review mode's card (ADR 0037): the word's image beside its text, the
 * readings to choose from, and a field whose Enter accepts or corrects.
 * ⌘/Ctrl+Enter marks it checked, ⌘/Ctrl+Shift+Enter checks the page, F8 skips.
 */
function ReviewCard({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const reviewing = useOcrJobs((state) => state.reviewing);
  const pages = useOcrJobs((state) => state.review[documentId]);
  const focus = useOcrJobs((state) =>
    state.focus?.documentId === documentId ? state.focus : null,
  );
  const status = useStatusText();
  if (!reviewing) return null;
  const n = (value: number) => formatNumber(value, locale);
  const queue = reviewQueue(pages ?? []);
  const word = focus?.at ? wordAt(pages ?? [], focus.at) : undefined;
  const page = pages?.find((entry) => entry.page === focus?.page);
  const position = focus
    ? queue.findIndex((item) => item.page === focus.page && item.word === focus.word)
    : -1;

  return (
    <section
      className={styles.card}
      aria-label={t("ocrPanel.reviewCard.heading")}
      data-testid="ocr-review-card"
    >
      <div className={styles.pageHeader}>
        <h3 className={styles.heading}>{t("ocrPanel.reviewCard.heading")}</h3>
        <Button onPress={() => void runCommand("ocr.stopReview")}>
          {t("ocrPanel.reviewCard.exit")}
        </Button>
      </div>
      <p className={styles.meta} role="status">
        {queue.length === 0
          ? t("ocrPanel.reviewCard.done")
          : position >= 0
            ? t("ocrPanel.reviewCard.position", { at: n(position + 1), count: n(queue.length) })
            : t("ocrPanel.reviewCard.left", { count: n(queue.length) })}
      </p>
      {focus && page && word ? (
        <WordReview
          key={`${focus.page}:${word.index}:${word.text}:${word.review.kind}`}
          documentId={documentId}
          page={page}
          word={word}
          statusText={status(word)}
        />
      ) : focus && page && focus.word === null ? (
        <div className={styles.cardBody}>
          <p className={styles.note}>
            {t("ocrPanel.reviewCard.truncated", { page: n(page.page + 1) })}
          </p>
          <div className={styles.controls}>
            <Button variant="primary" onPress={() => void runCommand("ocr.reviewCheckPage")}>
              {t("ocrPanel.reviewCard.checkPage")}
            </Button>
            <Button onPress={() => void runCommand("ocr.nextFlag")}>
              {t("ocrPanel.reviewCard.skip")}
            </Button>
          </div>
        </div>
      ) : null}
    </section>
  );
}

function WordReview({
  documentId,
  page,
  word,
  statusText,
}: {
  documentId: number;
  page: OcrReviewPage;
  word: OcrReviewWord;
  statusText: string;
}) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const decision = useDecisionText();
  const [text, setText] = useState(word.text);
  const field = useRef<HTMLInputElement>(null);
  useEffect(() => {
    field.current?.focus();
    field.current?.select();
  }, []);
  const n = (value: number) => formatNumber(value, locale);
  const percent = (value: number) =>
    formatNumber(value, locale, { style: "percent", maximumFractionDigits: 0 });
  const readOnly = page.converted || page.stale;
  const changed = text.trim() !== word.text;
  const readings = [
    ...(word.status.kind === "uncertain" ? word.status.alternatives : []),
    ...(word.status.kind === "corrected" || word.status.kind === "reordered"
      ? [word.status.read]
      : []),
    ...(word.review.kind === "edited" ? [word.review.read] : []),
  ].filter((reading, i, all) => reading !== word.text && all.indexOf(reading) === i);
  const hint = (id: "ocr.reviewCheck" | "ocr.reviewCheckPage" | "ocr.nextFlag") =>
    shortcutLabel(id) ?? "";

  return (
    <div className={styles.cardBody}>
      <span className={styles.meta}>
        {t("ocrPanel.review.page", { page: n(page.page + 1) })} ·{" "}
        {t("ocrPanel.review.confidence", { value: percent(word.probability) })}
      </span>
      <WordImage documentId={documentId} page={page.page} word={word} />
      <p className={styles.reasons}>
        {statusText}
        {word.review.kind !== "open" ? ` · ${decision(word)}` : ""}
      </p>
      {readOnly ? (
        <p className={styles.note}>
          {page.converted ? t("ocrPanel.reviewCard.converted") : t("ocrPanel.reviewCard.stale")}
        </p>
      ) : null}
      <form
        className={styles.correction}
        onSubmit={(event) => {
          event.preventDefault();
          void runCommand("ocr.reviewConfirm", { reviewText: readOnly ? word.text : text });
        }}
      >
        <label className={styles.fieldLabel} htmlFor="ocr-review-text">
          {t("ocrPanel.reviewCard.text")}
        </label>
        <input
          id="ocr-review-text"
          ref={field}
          className={styles.field}
          dir="auto"
          value={text}
          readOnly={readOnly}
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              void runCommand("ocr.stopReview");
            }
          }}
          data-testid="ocr-review-text"
        />
        <div className={styles.controls}>
          <Button type="submit" variant="primary">
            {changed && !readOnly
              ? t("ocrPanel.reviewCard.correct")
              : t("ocrPanel.reviewCard.accept")}
          </Button>
          <Button
            aria-keyshortcuts={hint("ocr.reviewCheck")}
            onPress={() => void runCommand("ocr.reviewCheck")}
          >
            {t("ocrPanel.reviewCard.check")}
          </Button>
          <Button
            aria-keyshortcuts={hint("ocr.reviewCheckPage")}
            onPress={() => void runCommand("ocr.reviewCheckPage")}
          >
            {t("ocrPanel.reviewCard.checkPage")}
          </Button>
          <Button
            aria-keyshortcuts={hint("ocr.nextFlag")}
            onPress={() => void runCommand("ocr.nextFlag")}
          >
            {t("ocrPanel.reviewCard.skip")}
          </Button>
        </div>
      </form>
      {readings.length > 0 && !readOnly ? (
        <fieldset className={styles.readings} aria-label={t("ocrPanel.reviewCard.readings")}>
          {readings.map((reading) => (
            <Button
              key={reading}
              onPress={() => void runCommand("ocr.reviewChoose", { reviewText: reading })}
            >
              {t("ocrPanel.reviewCard.use", { text: isolate(reading) })}
            </Button>
          ))}
        </fieldset>
      ) : null}
      <p className={styles.note}>
        {t("ocrPanel.reviewCard.keys", {
          check: hint("ocr.reviewCheck"),
          page: hint("ocr.reviewCheckPage"),
          skip: hint("ocr.nextFlag"),
        })}
      </p>
    </div>
  );
}

/**
 * The scan around a word, rendered by the shell (ADR 0011: the webview
 * decodes it), with the word's box marked. The page's geometry runs left to
 * right whatever the UI's direction.
 */
function WordImage({
  documentId,
  page,
  word,
}: {
  documentId: number;
  page: number;
  word: OcrReviewWord;
}) {
  const { t } = useTranslation();
  const box = word.bounds ?? word.line;
  if (!box) return null;
  const height = Math.max(1, box[3] - box[1]);
  const line = word.line ?? box;
  // A few words of context on either side, inside the line.
  const area: [number, number, number, number] = [
    Math.max(line[0] - height, box[0] - 4 * height),
    box[1] - 0.6 * height,
    Math.min(line[2] + height, box[2] + 4 * height),
    box[3] + 0.6 * height,
  ];
  const w = area[2] - area[0];
  const h = area[3] - area[1];
  const pct = (value: number, of: number) => `${(100 * value) / of}%`;
  return (
    <div className={styles.crop} dir="ltr" data-testid="ocr-review-image">
      <img
        className={styles.cropImage}
        src={backend.ocrCropUrl(documentId, page, area, CROP_PIXELS)}
        alt={t("ocrPanel.reviewCard.image", { text: word.text })}
      />
      {word.bounds ? (
        <span
          className={styles.cropMark}
          aria-hidden="true"
          style={{
            insetInlineStart: pct(word.bounds[0] - area[0], w),
            insetBlockStart: pct(word.bounds[1] - area[1], h),
            inlineSize: pct(word.bounds[2] - word.bounds[0], w),
            blockSize: pct(word.bounds[3] - word.bounds[1], h),
          }}
        />
      ) : null}
    </div>
  );
}

function Review({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const pages = useOcrJobs((state) => state.review[documentId]);
  const numbers = useOcrJobs((state) => state.numbers);
  const reviewing = useOcrJobs((state) => state.reviewing);
  const focus = useOcrJobs((state) =>
    state.focus?.documentId === documentId ? state.focus : null,
  );
  const canStep = useAppStore((state) => isCommandEnabled("ocr.nextFlag", state));
  const finished = useOcrJobs((state) =>
    jobsOf(state.jobs, documentId).some((job) => job.finished > 0),
  );
  const status = useStatusText();
  const decision = useDecisionText();
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
      page.open > 0
        ? t("ocrPanel.review.open", { count: n(page.open) })
        : t("ocrPanel.review.allDecided"),
      page.converted ? t("ocrPanel.review.converted") : null,
    ]
      .filter(Boolean)
      .join(" · ");

  return (
    <section
      className={styles.section}
      aria-label={t("ocrPanel.review.heading")}
      data-testid="ocr-review"
    >
      <h3 className={styles.heading}>{t("ocrPanel.review.heading")}</h3>
      <Checkbox
        className={styles.check}
        isSelected={numbers}
        onChange={(value) => void runCommand("ocr.reviewNumbers", { numbers: value })}
      >
        <span className={styles.box} aria-hidden="true" />
        {t("ocrPanel.review.numbers")}
      </Checkbox>
      {pages.length === 0 ? (
        <p className={styles.note}>{t("ocrPanel.review.empty")}</p>
      ) : (
        <>
          <div className={styles.controls}>
            {!reviewing ? (
              <Button variant="primary" onPress={() => void runCommand("ocr.startReview")}>
                {t("ocrPanel.review.start")}
              </Button>
            ) : null}
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
                    const current = focus?.at?.[0] === p && focus.at[1] === w;
                    const decided = decision(word);
                    return (
                      <li key={word.index}>
                        <button
                          type="button"
                          className={styles.word}
                          data-kind={word.status.kind}
                          data-review={word.review.kind}
                          aria-current={current ? "true" : undefined}
                          aria-label={t("ocrPanel.review.word", {
                            word: isolate(word.text),
                            status: decided ? `${status(word)}; ${decided}` : status(word),
                            confidence: percent(word.probability),
                          })}
                          onClick={() => void runCommand("ocr.showWord", { reviewAt: [p, w] })}
                        >
                          <bdi className={styles.wordText}>{word.text}</bdi>
                          <span className={styles.meta}>
                            {status(word)}
                            {decided ? ` · ${decided}` : ""}
                          </span>
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

/** Reading an area again (ADR 0038): draw it, choose the language or "numbers only". */
function Region({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const drawing = useOcrJobs((state) => state.drawingRegion);
  const region = useOcrJobs((state) =>
    state.region?.documentId === documentId ? state.region : null,
  );
  const language = useOcrJobs((state) => state.regionLanguage);
  const numbers = useOcrJobs((state) => state.regionNumbers);
  const n = (value: number) => formatNumber(value, locale);

  return (
    <section className={styles.section} aria-label={t("ocrPanel.region.heading")}>
      <h3 className={styles.heading}>{t("ocrPanel.region.heading")}</h3>
      {region ? (
        <>
          <p className={styles.status} role="status">
            {t("ocrPanel.region.drawn", { page: n(region.page + 1) })}
          </p>
          <ToggleButtonGroup
            className={styles.segments}
            selectionMode="single"
            disallowEmptySelection
            aria-label={t("ocrPanel.region.language")}
            selectedKeys={[language]}
            onSelectionChange={(keys: Set<Key>) => {
              const [next] = keys;
              const chosen = LANGUAGES.find((candidate) => candidate === next);
              if (chosen) void runCommand("ocr.setRegionOptions", { language: chosen });
            }}
          >
            {LANGUAGES.map((id) => (
              <ToggleButton key={id} id={id} className={styles.segment}>
                {t(`ocrPanel.region.languages.${id}`)}
              </ToggleButton>
            ))}
          </ToggleButtonGroup>
          <Checkbox
            className={styles.check}
            isSelected={numbers}
            onChange={(value) => void runCommand("ocr.setRegionOptions", { numbers: value })}
          >
            <span className={styles.box} aria-hidden="true" />
            {t("ocrPanel.region.numbers")}
          </Checkbox>
          <div className={styles.controls}>
            <Button variant="primary" onPress={() => void runCommand("ocr.readRegion")}>
              {t("ocrPanel.region.read")}
            </Button>
            <Button onPress={() => void runCommand("ocr.cancelRegion")}>
              {t("ocrPanel.region.cancel")}
            </Button>
          </div>
        </>
      ) : (
        <>
          <Button
            variant={drawing ? "primary" : "secondary"}
            onPress={() => void runCommand("ocr.drawRegion")}
            aria-pressed={drawing}
          >
            {t("ocrPanel.region.draw")}
          </Button>
          <p className={styles.note} role={drawing ? "status" : undefined}>
            {drawing ? t("ocrPanel.region.drawing") : t("ocrPanel.region.note")}
          </p>
        </>
      )}
    </section>
  );
}

/** Another tool's text layers, checked and replaced (ADR 0038). */
function Layers({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const checks = useOcrJobs((state) => state.checks[documentId]);
  const canReplace = useAppStore((state) => isCommandEnabled("ocr.replaceLayers", state));
  const n = (value: number) => formatNumber(value, locale);
  const percent = (value: number) =>
    formatNumber(value, locale, { style: "percent", maximumFractionDigits: 0 });
  const poor = (checks ?? []).filter((check) => check.verdict === "poor");
  const good = (checks ?? []).filter((check) => check.verdict === "good").length;
  const problems = (check: OcrLayerCheck) =>
    check.problems.map((problem) => t(`ocrPanel.layers.problem.${problem}`)).join(" · ");

  return (
    <section
      className={styles.section}
      aria-label={t("ocrPanel.layers.heading")}
      data-testid="ocr-layers"
    >
      <h3 className={styles.heading}>{t("ocrPanel.layers.heading")}</h3>
      <Button onPress={() => void runCommand("ocr.checkLayers")}>
        {t("ocrPanel.layers.check")}
      </Button>
      {checks && checks.length > 0 ? (
        <>
          <p className={styles.status} role="status">
            {poor.length > 0
              ? t("ocrPanel.layers.found", { poor: n(poor.length), good: n(good) })
              : t("ocrPanel.layers.none", { good: n(good) })}
          </p>
          <ul className={styles.words}>
            {poor.map((check) => (
              <li key={check.page} className={styles.layer}>
                <span className={styles.pageName}>
                  {t("ocrPanel.review.page", { page: n(check.page + 1) })}
                </span>
                <span className={styles.meta}>
                  {problems(check)}
                  {check.similarity !== null
                    ? ` · ${t("ocrPanel.layers.similarity", { value: percent(check.similarity) })}`
                    : ""}
                </span>
                {check.sample ? (
                  <span className={styles.meta}>
                    {t("ocrPanel.layers.sample")} <bdi dir="auto">{check.sample}</bdi>
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
          {canReplace ? (
            <Button variant="primary" onPress={() => void runCommand("ocr.replaceLayers")}>
              {t("ocrPanel.layers.replace", { count: n(poor.length) })}
            </Button>
          ) : null}
        </>
      ) : (
        <p className={styles.note}>{t("ocrPanel.layers.note")}</p>
      )}
    </section>
  );
}

/** Scan to editable text (ADR 0039). */
function Convert() {
  const { t } = useTranslation();
  const keepScan = useOcrJobs((state) => state.keepScan);
  const family = useOcrJobs((state) => state.family);
  return (
    <section className={styles.section} aria-label={t("ocrPanel.convert.heading")}>
      <h3 className={styles.heading}>{t("ocrPanel.convert.heading")}</h3>
      <ToggleButtonGroup
        className={styles.segments}
        selectionMode="single"
        disallowEmptySelection
        aria-label={t("ocrPanel.convert.scan")}
        selectedKeys={[keepScan ? "keep" : "remove"]}
        onSelectionChange={(keys: Set<Key>) => {
          const [next] = keys;
          void runCommand("ocr.setConvertOptions", { keepScan: next === "keep" });
        }}
      >
        <ToggleButton id="keep" className={styles.segment}>
          {t("ocrPanel.convert.keep")}
        </ToggleButton>
        <ToggleButton id="remove" className={styles.segment}>
          {t("ocrPanel.convert.remove")}
        </ToggleButton>
      </ToggleButtonGroup>
      <ToggleButtonGroup
        className={styles.segments}
        selectionMode="single"
        disallowEmptySelection
        aria-label={t("ocrPanel.convert.family")}
        selectedKeys={[family]}
        onSelectionChange={(keys: Set<Key>) => {
          const [next] = keys;
          if (next === "naskh" || next === "sans")
            void runCommand("ocr.setConvertOptions", { convertFamily: next });
        }}
      >
        <ToggleButton id="naskh" className={styles.segment}>
          {t("ocrPanel.convert.naskh")}
        </ToggleButton>
        <ToggleButton id="sans" className={styles.segment}>
          {t("ocrPanel.convert.sans")}
        </ToggleButton>
      </ToggleButtonGroup>
      <div className={styles.controls}>
        <Button variant="primary" onPress={() => void runCommand("ocr.convertPage")}>
          {t("ocrPanel.convert.page")}
        </Button>
        <Button onPress={() => void runCommand("ocr.convertDocument")}>
          {t("ocrPanel.convert.document")}
        </Button>
      </div>
      <p className={styles.note}>{t("ocrPanel.convert.note")}</p>
    </section>
  );
}
