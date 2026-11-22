import { Checkbox } from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { RedactHit, RedactPreset, SanitizeOptions, SearchHit } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { type OpenDocument, useAppStore } from "../../state/appStore";
import { isMarked, markedPages } from "../../state/redaction";
import { getViewer } from "../../viewer/registry";
import { Button } from "../ui/Button";
import styles from "./ProtectPanel.module.css";

const PRESETS: RedactPreset[] = ["email", "phone", "iban", "idNumber", "cardNumber"];
const OPTIONS: (keyof SanitizeOptions)[] = [
  "metadata",
  "scripts",
  "attachments",
  "comments",
  "hiddenContent",
];
/** Matches listed at once; "Mark all" still marks every one. */
const LISTED = 200;
export const REDACT_INPUT_ID = "redact-input";

/** Scrolls the page view to a match. */
function reveal(documentId: number, hit: RedactHit) {
  const shown: SearchHit = {
    page: hit.page,
    block: 0,
    line: 0,
    start: 0,
    end: 0,
    direction: "ltr",
    rects: hit.rects,
    context: { before: hit.before, matched: hit.text, after: hit.after },
  };
  getViewer(documentId)?.revealHit(shown);
}

/**
 * The Protect tool (ADR 0042): mark what to redact (by dragging on a page, by
 * selecting text, or by finding text and patterns), review the marks, apply
 * them; and sanitize what the file carries besides its pages. Every control
 * runs a command.
 */
export function ProtectPanel({ document }: { document: OpenDocument }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const sanitizeOptions = useAppStore((state) => state.sanitizeOptions);
  const { redaction } = document;
  const n = (value: number) => formatNumber(value, locale);
  const { find, marks } = redaction;
  const listed = find.hits.slice(0, LISTED);

  return (
    <div className={styles.panel} data-testid="protect-panel">
      <section className={styles.section} aria-labelledby="redact-heading">
        <h3 id="redact-heading" className={styles.heading}>
          {t("protect.redactHeading")}
        </h3>
        <p className={styles.hint}>{t("protect.hint")}</p>
        <form
          className={styles.find}
          onSubmit={(event) => {
            event.preventDefault();
            void runCommand("redact.find", { query: redaction.query });
          }}
        >
          <label className={styles.label} htmlFor={REDACT_INPUT_ID}>
            {t("protect.findLabel")}
          </label>
          <div className={styles.row}>
            <input
              id={REDACT_INPUT_ID}
              className={styles.input}
              type="search"
              value={redaction.query}
              placeholder={t("protect.findPlaceholder")}
              lang={locale}
              dir="auto"
              onChange={(event) =>
                useAppStore
                  .getState()
                  .updateRedaction(document.id, { query: event.target.value, preset: null })
              }
            />
            <Button type="submit" variant="secondary">
              {t("protect.findButton")}
            </Button>
          </div>
          <Checkbox
            className={styles.check}
            isSelected={redaction.regex}
            onChange={() => void runCommand("redact.toggleRegex")}
          >
            <span className={styles.box} aria-hidden="true" />
            {t("protect.regex")}
          </Checkbox>
        </form>
        <fieldset className={styles.presets} aria-label={t("protect.presets")}>
          {PRESETS.map((preset) => (
            <Button
              key={preset}
              variant={redaction.preset === preset ? "primary" : "secondary"}
              onPress={() => void runCommand("redact.findPreset", { preset })}
            >
              {t(`protect.preset.${preset}`)}
            </Button>
          ))}
        </fieldset>

        <p className={styles.status} role="status">
          {find.status === "searching"
            ? t("protect.searching")
            : find.status === "error" && find.error
              ? t(`errors.${find.error}`)
              : find.status === "done"
                ? find.hits.length === 0
                  ? t("protect.noneFound")
                  : t(find.truncated ? "protect.foundTruncated" : "protect.found", {
                      formatted: n(find.hits.length),
                    })
                : ""}
        </p>
        {listed.length > 0 && (
          <>
            <Button variant="secondary" onPress={() => void runCommand("redact.markAllHits")}>
              {t("protect.markAll")}
            </Button>
            <ul className={styles.list} aria-label={t("protect.results")}>
              {listed.map((hit, index) => {
                const marked = isMarked(redaction, hit);
                return (
                  // biome-ignore lint/suspicious/noArrayIndexKey: hits have no identity; the list is replaced whole
                  <li key={index} className={styles.item}>
                    <button
                      type="button"
                      className={styles.hit}
                      onClick={() => reveal(document.id, hit)}
                    >
                      <span className={styles.page}>
                        {t("protect.page", { page: n(hit.page + 1) })}
                      </span>
                      <span className={styles.context} dir="auto">
                        <bdi>{hit.before}</bdi>
                        <mark className={styles.match}>
                          <bdi>{hit.text}</bdi>
                        </mark>
                        <bdi>{hit.after}</bdi>
                      </span>
                    </button>
                    <Button
                      variant="ghost"
                      isDisabled={marked}
                      onPress={() => void runCommand("redact.markHit", { index })}
                    >
                      {marked ? t("protect.marked") : t("protect.mark")}
                    </Button>
                  </li>
                );
              })}
            </ul>
          </>
        )}

        <h4 className={styles.subheading}>{t("protect.marksTitle")}</h4>
        {marks.length === 0 ? (
          <p className={styles.hint}>{t("protect.noMarks")}</p>
        ) : (
          <>
            <p className={styles.status} data-testid="mark-summary">
              {t("protect.markedAreas", { marks: n(marks.length) })} ·{" "}
              {t("protect.markedPages", { pages: n(markedPages(redaction)) })}
            </p>
            <ul className={styles.list} aria-label={t("protect.marksTitle")}>
              {marks.map((mark) => (
                <li key={mark.id} className={styles.item}>
                  <button
                    type="button"
                    className={styles.hit}
                    onClick={() =>
                      reveal(document.id, {
                        page: mark.page,
                        rects: mark.rects,
                        text: mark.text ?? "",
                        before: "",
                        after: "",
                      })
                    }
                  >
                    <span className={styles.page}>
                      {t("protect.page", { page: n(mark.page + 1) })}
                    </span>
                    <span className={styles.context} dir="auto">
                      <bdi>
                        {mark.text ??
                          t(mark.source === "selection" ? "protect.selection" : "protect.area")}
                      </bdi>
                    </span>
                  </button>
                  <Button
                    variant="ghost"
                    onPress={() => void runCommand("redact.removeMark", { markId: mark.id })}
                  >
                    {t("protect.remove")}
                  </Button>
                </li>
              ))}
            </ul>
          </>
        )}
        <div className={styles.row}>
          <Button
            variant="ghost"
            isDisabled={marks.length === 0}
            onPress={() => void runCommand("redact.undoMark")}
          >
            {t("protect.undo")}
          </Button>
          <Button
            variant="ghost"
            isDisabled={redaction.undone.length === 0}
            onPress={() => void runCommand("redact.redoMark")}
          >
            {t("protect.redo")}
          </Button>
          <Button
            variant="ghost"
            isDisabled={marks.length === 0}
            onPress={() => void runCommand("redact.clearMarks")}
          >
            {t("protect.clear")}
          </Button>
        </div>
        <Button
          variant="primary"
          isDisabled={marks.length === 0}
          onPress={() => void runCommand("redact.apply")}
        >
          {t("protect.apply")}
        </Button>
        {redaction.applied && (
          <dl
            className={styles.report}
            data-testid="redaction-report"
            aria-label={t("protect.appliedTitle")}
          >
            <dt className={styles.reportTitle}>{t("protect.appliedTitle")}</dt>
            {(
              [
                ["glyphs", redaction.applied.counts.glyphs],
                ["images", redaction.applied.counts.imagesBlanked],
                ["imagesDropped", redaction.applied.counts.imagesDropped],
                ["shapes", redaction.applied.counts.paths],
                ["annotations", redaction.applied.annotations],
                ["strings", redaction.applied.strings],
              ] as const
            ).map(([key, value]) => (
              <div key={key} className={styles.reportRow}>
                <dt>{t(`protect.applied.${key}`)}</dt>
                <dd>{n(value)}</dd>
              </div>
            ))}
          </dl>
        )}
      </section>

      <section className={styles.section} aria-labelledby="sanitize-heading">
        <h3 id="sanitize-heading" className={styles.heading}>
          {t("protect.sanitizeHeading")}
        </h3>
        <p className={styles.hint}>{t("protect.sanitizeHint")}</p>
        <div className={styles.options}>
          {OPTIONS.map((option) => (
            <Checkbox
              key={option}
              className={styles.check}
              isSelected={sanitizeOptions[option]}
              onChange={(value) => void runCommand("sanitize.setOption", { option, value })}
            >
              <span className={styles.box} aria-hidden="true" />
              {t(`protect.option.${option}`)}
            </Checkbox>
          ))}
        </div>
        <Button
          variant="secondary"
          isDisabled={!Object.values(sanitizeOptions).some(Boolean)}
          onPress={() => void runCommand("sanitize.run")}
        >
          {t("protect.sanitize")}
        </Button>
        {redaction.sanitized && (
          <dl
            className={styles.report}
            data-testid="sanitize-report"
            aria-label={t("protect.sanitizedTitle")}
          >
            <dt className={styles.reportTitle}>{t("protect.sanitizedTitle")}</dt>
            {(
              Object.entries(redaction.sanitized) as [keyof typeof redaction.sanitized, number][]
            ).map(([key, value]) => (
              <div key={key} className={styles.reportRow}>
                <dt>{t(`protect.sanitized.${key}`)}</dt>
                <dd>{n(value)}</dd>
              </div>
            ))}
          </dl>
        )}
      </section>
    </div>
  );
}
