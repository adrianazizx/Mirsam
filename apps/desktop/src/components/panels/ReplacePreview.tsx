import { useState } from "react";
import {
  Checkbox,
  GridList,
  GridListItem,
  type Key,
  ListLayout,
  type Selection,
  Virtualizer,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { ReplaceOccurrence } from "../../backend";
import { isCommandEnabled, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { IDLE_REPLACE, useAppStore } from "../../state/appStore";
import { tokenPx } from "../../styles/tokens";
import { Button } from "../ui/Button";
import styles from "./ReplacePreview.module.css";

/**
 * The replace preview (ADR 0052): every match with what it becomes, ticked
 * to be replaced, or with why it cannot be. Nothing changes until the
 * replace button is pressed; that is one undo step.
 */
export function ReplacePreview({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const replace = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.replace ?? IDLE_REPLACE,
  );
  const canConfirm = useAppStore((state) => isCommandEnabled("search.replaceConfirm", state));
  const canCancel = useAppStore((state) => isCommandEnabled("search.replaceCancel", state));
  const [rowHeight] = useState(() => tokenPx("--size-replace-result", 72));

  const occurrences = replace.plan?.occurrences ?? [];
  const ready = occurrences.flatMap((occurrence, index) => (occurrence.target ? [index] : []));
  const chosen = ready.filter((index) => !replace.excluded.includes(index));
  const skipped = occurrences.length - ready.length;
  const count = (value: number) => formatNumber(value, locale);

  const status =
    replace.status === "loading"
      ? t("replace.loading")
      : replace.status === "replacing"
        ? t("replace.replacing")
        : replace.status === "error"
          ? t(`errors.${replace.error ?? "replaceFailed"}`)
          : occurrences.length === 0
            ? t("replace.none")
            : t("replace.summary", { ready: count(ready.length), skipped: count(skipped) });

  const onSelectionChange = (keys: Selection) => {
    const next = new Set(keys === "all" ? ready : [...keys].map(Number));
    for (const index of ready) {
      // Each match ticked or unticked is a command of its own.
      if (next.has(index) === replace.excluded.includes(index)) {
        void runCommand("search.replaceToggle", { index });
      }
    }
  };

  return (
    <section className={styles.preview} aria-label={t("replace.preview")}>
      <p className={styles.status} role="status" aria-live="polite" data-testid="replace-status">
        {status}
        {replace.plan?.truncated ? (
          <> {t("replace.truncated", { formatted: count(occurrences.length) })}</>
        ) : null}
      </p>
      {occurrences.length > 0 ? (
        <Virtualizer layout={ListLayout} layoutOptions={{ rowHeight }}>
          <GridList
            aria-label={t("replace.preview")}
            className={styles.list}
            items={occurrences.map((occurrence, index) => ({ id: index, occurrence }))}
            selectionMode="multiple"
            selectionBehavior="toggle"
            selectedKeys={new Set<Key>(chosen)}
            onSelectionChange={onSelectionChange}
            disabledKeys={occurrences.flatMap((occurrence, index) =>
              occurrence.target ? [] : [index],
            )}
            disabledBehavior="selection"
            onAction={(key) => void runCommand("search.replaceShow", { index: Number(key) })}
            data-testid="replace-preview"
          >
            {({ id: index, occurrence }: { id: number; occurrence: ReplaceOccurrence }) => (
              <GridListItem
                id={index}
                className={styles.item}
                textValue={`${occurrence.context.before}${occurrence.context.matched}${occurrence.context.after}`}
              >
                {occurrence.target ? (
                  <Checkbox
                    slot="selection"
                    className={styles.check}
                    aria-label={t("replace.include")}
                  >
                    <span className={styles.box} aria-hidden="true" />
                  </Checkbox>
                ) : (
                  <span className={styles.spacer} aria-hidden="true" />
                )}
                <span className={styles.body}>
                  <span className={styles.meta}>
                    {t("replace.page", { page: count(occurrence.page + 1) })}
                    {occurrence.skip ? (
                      <span className={styles.reason} title={t(`replace.skip.${occurrence.skip}`)}>
                        {t("replace.skipped", { reason: t(`replace.skip.${occurrence.skip}`) })}
                      </span>
                    ) : null}
                  </span>
                  {/* The context reads in its own direction, whatever the UI language. */}
                  <span className={styles.context} dir={occurrence.rtl ? "rtl" : "ltr"}>
                    {occurrence.context.before}
                    <del className={styles.old}>{occurrence.context.matched}</del>
                    {occurrence.skip ? null : (
                      <ins className={styles.new}>{occurrence.replacement}</ins>
                    )}
                    {occurrence.context.after}
                  </span>
                </span>
              </GridListItem>
            )}
          </GridList>
        </Virtualizer>
      ) : null}
      <div className={styles.actions}>
        <Button
          variant="primary"
          isDisabled={!canConfirm}
          onPress={() => void runCommand("search.replaceConfirm")}
          data-testid="replace-confirm"
        >
          {t("replace.confirm", { count: count(chosen.length) })}
        </Button>
        <Button isDisabled={!canCancel} onPress={() => void runCommand("search.replaceCancel")}>
          {t("replace.cancel")}
        </Button>
      </div>
    </section>
  );
}
