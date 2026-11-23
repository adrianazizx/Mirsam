import { CaseSensitive, Regex, Replace, ReplaceAll, WholeWord } from "lucide-react";
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";
import {
  Input,
  ListBox,
  ListBoxItem,
  ListLayout,
  SearchField,
  type Selection,
  TextField,
  ToggleButton,
  Toolbar,
  Virtualizer,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { SearchHit, SearchOptions } from "../../backend";
import {
  type CommandId,
  REPLACE_INPUT_ID,
  runCommand,
  SEARCH_INPUT_ID,
  shortcutLabel,
} from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { IDLE_REPLACE, IDLE_SEARCH, useAppStore } from "../../state/appStore";
import { tokenPx } from "../../styles/tokens";
import { CommandButton } from "../ui/CommandButton";
import { Tooltip } from "../ui/Tooltip";
import { ReplacePreview } from "./ReplacePreview";
import styles from "./SearchPanel.module.css";

/** Typing searches after this pause, so each keystroke does not start a search. */
const TYPING_DELAY_MS = 180;

const OPTIONS: {
  key: keyof SearchOptions;
  command: CommandId;
  icon: ReactNode;
  /** Text shown in place of an icon: part of the button's name too (WCAG 2.5.3, label in name). */
  text?: string;
}[] = [
  { key: "caseSensitive", command: "search.toggleCase", icon: <CaseSensitive /> },
  { key: "wholeWord", command: "search.toggleWholeWord", icon: <WholeWord /> },
  { key: "regex", command: "search.toggleRegex", icon: <Regex /> },
  // Arabic normalization: "أ ≈ ا"; ta marbuta: "ة ≈ ه". Shown as text, not icons.
  {
    key: "arabicNormalize",
    command: "search.toggleArabic",
    icon: <bdi lang="ar">أ≈ا</bdi>,
    text: "أ≈ا",
  },
  {
    key: "taMarbutaAsHa",
    command: "search.toggleTaMarbuta",
    icon: <bdi lang="ar">ة≈ه</bdi>,
    text: "ة≈ه",
  },
];

/**
 * Search in the document: query, options, and the list of hits with their
 * context; and replace (ADR 0052): the hit shown, or all of them after a preview.
 */
export function SearchPanel({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const options = useAppStore((state) => state.searchOptions);
  const search = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.search ?? IDLE_SEARCH,
  );
  const replaceOpen = useAppStore((state) => state.replaceOpen);
  const replacement = useAppStore((state) => state.replacement);
  const previewing = useAppStore(
    (state) =>
      (state.documents.find((open) => open.id === id)?.replace ?? IDLE_REPLACE).status !== "idle",
  );
  const [draft, setDraft] = useState(search.query);
  const [rowHeight, setRowHeight] = useState(() => tokenPx("--size-search-result", 56));
  // The row height follows the interface size (ADR 0070). The root's scale is set
  // by the app's own effect, which runs after this one: read it on the next frame.
  const uiScale = useAppStore((state) => state.uiScale);
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-read when the interface size changes
  useEffect(() => {
    const frame = requestAnimationFrame(() => setRowHeight(tokenPx("--size-search-result", 56)));
    return () => cancelAnimationFrame(frame);
  }, [uiScale]);
  const timer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const submit = (query: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    void runCommand("search.run", { query });
  };

  const onChange = (value: string) => {
    setDraft(value);
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => submit(value), TYPING_DELAY_MS);
  };

  const count = search.hits.length;
  const status =
    search.status === "searching"
      ? t("search.searching")
      : search.status === "error"
        ? t(`errors.${search.error ?? "searchFailed"}`)
        : search.status === "done"
          ? count === 0
            ? t("search.none")
            : t(search.truncated ? "search.countTruncated" : "search.count", {
                count,
                formatted: formatNumber(count, locale),
              })
          : "";

  return (
    <div className={styles.panel}>
      <div className={styles.controls}>
        <SearchField
          aria-label={t("search.label")}
          value={draft}
          onChange={onChange}
          onSubmit={(value: string) => {
            // Enter on an unchanged query moves to the next hit.
            if (value === search.query && search.status === "done") void runCommand("search.next");
            else submit(value);
          }}
          onClear={() => submit("")}
          className={styles.field}
        >
          <Input
            id={SEARCH_INPUT_ID}
            className={styles.input}
            placeholder={t("search.placeholder")}
            data-testid="search-input"
            onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
              if (event.key === "Enter" && event.shiftKey) {
                event.preventDefault();
                void runCommand("search.previous");
              }
            }}
          />
        </SearchField>
        <Toolbar className={styles.options} aria-label={t("search.options")}>
          {OPTIONS.map((option) => (
            <Tooltip
              key={option.key}
              label={t(`commands.${option.command}`)}
              shortcut={shortcutLabel(option.command)}
            >
              <ToggleButton
                className={styles.option}
                isSelected={options[option.key]}
                onChange={() => void runCommand(option.command)}
                aria-label={
                  option.text
                    ? `${t(`commands.${option.command}`)}: ${option.text}`
                    : t(`commands.${option.command}`)
                }
              >
                <span className={styles.glyph} aria-hidden="true">
                  {option.icon}
                </span>
              </ToggleButton>
            </Tooltip>
          ))}
          <Tooltip
            label={t(`commands.${replaceOpen ? "search.replaceClose" : "search.replaceOpen"}`)}
            shortcut={replaceOpen ? null : shortcutLabel("search.replaceOpen")}
          >
            <ToggleButton
              className={styles.option}
              isSelected={replaceOpen}
              onChange={() =>
                void runCommand(replaceOpen ? "search.replaceClose" : "search.replaceOpen")
              }
              aria-label={t("commands.search.replaceOpen")}
              data-testid="replace-toggle"
            >
              <span className={styles.glyph} aria-hidden="true">
                <Replace />
              </span>
            </ToggleButton>
          </Tooltip>
        </Toolbar>
        {replaceOpen ? (
          <div className={styles.replaceRow}>
            <TextField
              aria-label={t("replace.label")}
              value={replacement}
              onChange={(text: string) => void runCommand("search.setReplacement", { text })}
              className={styles.field}
            >
              <Input
                id={REPLACE_INPUT_ID}
                className={styles.input}
                placeholder={t(options.regex ? "replace.placeholderRegex" : "replace.placeholder")}
                data-testid="replace-input"
                onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
                  // Enter replaces the hit shown, as Enter in the search field moves to the next.
                  if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    void runCommand("search.replace");
                  }
                }}
              />
            </TextField>
            <CommandButton command="search.replace">
              <Replace />
            </CommandButton>
            <CommandButton command="search.replaceAll">
              <ReplaceAll />
            </CommandButton>
          </div>
        ) : null}
        <p className={styles.status} role="status" aria-live="polite" data-testid="search-status">
          {status}
        </p>
      </div>
      {previewing ? (
        <ReplacePreview documentId={id} />
      ) : count > 0 ? (
        <Virtualizer layout={ListLayout} layoutOptions={{ rowHeight }}>
          <ListBox
            aria-label={t("search.results")}
            items={search.hits.map((hit, index) => ({ id: index, hit }))}
            selectionMode="single"
            disallowEmptySelection
            selectedKeys={search.active >= 0 ? [search.active] : []}
            onSelectionChange={(keys: Selection) => {
              if (keys === "all") return;
              const [index] = [...keys];
              if (typeof index === "number") void runCommand("search.goTo", { index });
            }}
            className={styles.results}
            data-testid="search-results"
          >
            {({ id: index, hit }: { id: number; hit: SearchHit }) => (
              <ListBoxItem
                id={index}
                className={styles.result}
                textValue={`${hit.context.before}${hit.context.matched}${hit.context.after}`}
              >
                <span className={styles.page}>
                  {t("search.page", { page: formatNumber(hit.page + 1, locale) })}
                </span>
                {/* The context reads in its own direction, whatever the UI language. */}
                <span className={styles.context} dir={hit.direction}>
                  {hit.context.before}
                  <mark className={styles.mark}>{hit.context.matched}</mark>
                  {hit.context.after}
                </span>
              </ListBoxItem>
            )}
          </ListBox>
        </Virtualizer>
      ) : null}
    </div>
  );
}
