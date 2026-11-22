import {
  CircleCheck,
  Highlighter,
  LineSquiggle,
  Pen,
  StickyNote,
  Strikethrough,
  TextCursorInput,
  Underline,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import {
  Button as AriaButton,
  Input,
  type Key,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
  TextArea,
  TextField,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { type AnnotationKind, backend, type DocumentComment } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatNumber, formatTag, type Locale } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import {
  authorsOf,
  type CommentFilter,
  type CommentSort,
  filterComments,
  MAX_AUTHOR,
  sortComments,
} from "../../state/comments";
import { Button } from "../ui/Button";
import selectStyles from "../ui/CommandSelect.module.css";
import styles from "./CommentsPanel.module.css";

const KIND_ICONS: Record<AnnotationKind, ReactNode> = {
  highlight: <Highlighter />,
  underline: <Underline />,
  strikeOut: <Strikethrough />,
  squiggly: <LineSquiggle />,
  ink: <Pen />,
  note: <StickyNote />,
  textBox: <TextCursorInput />,
};

interface Option<T extends string> {
  id: T;
  label: string;
}

/** A filter or the order: choosing an option runs its command. */
function ChoiceSelect<T extends string>({
  label,
  value,
  options,
  onChoose,
}: {
  label: string;
  value: T;
  options: Option<T>[];
  onChoose: (value: T) => void;
}) {
  const current = options.find((option) => option.id === value);
  return (
    <Select
      aria-label={label}
      value={value}
      onChange={(key: Key | null) => {
        const option = options.find((candidate) => candidate.id === key);
        if (option) onChoose(option.id);
      }}
      className={selectStyles.select}
    >
      <AriaButton className={`${selectStyles.button} ${styles.choice}`}>
        <SelectValue className={styles.choiceValue}>
          {() => <bdi>{current?.label ?? ""}</bdi>}
        </SelectValue>
      </AriaButton>
      <Popover placement="bottom start" className={selectStyles.popover}>
        <ListBox className={selectStyles.list}>
          {options.map((option) => (
            <ListBoxItem
              key={option.id}
              id={option.id}
              textValue={option.label}
              className={selectStyles.item}
            >
              <bdi>{option.label}</bdi>
            </ListBoxItem>
          ))}
        </ListBox>
      </Popover>
    </Select>
  );
}

/** Who and when, above a comment or a reply. */
function Byline({
  author,
  modified,
  dates,
}: {
  author: string | null;
  modified: number | null;
  dates: Intl.DateTimeFormat;
}) {
  const { t } = useTranslation();
  return (
    <>
      {/* A name can be in either script whatever the UI language. */}
      <bdi className={author ? styles.author : styles.unnamed}>
        {author ?? t("comments.noAuthor")}
      </bdi>
      {modified !== null && (
        <time className={styles.time} dateTime={new Date(modified * 1000).toISOString()}>
          {dates.format(modified * 1000)}
        </time>
      )}
    </>
  );
}

function ReplyForm({ comment, onDone }: { comment: DocumentComment; onDone: () => void }) {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const send = () => {
    if (text.trim() === "" || busy) return;
    setBusy(true);
    void runCommand("comment.reply", { page: comment.page, index: comment.index, text }).finally(
      () => {
        setBusy(false);
        onDone();
      },
    );
  };
  return (
    <form
      className={styles.replyForm}
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <TextField
        value={text}
        onChange={setText}
        aria-label={t("comments.replyLabel")}
        className={styles.replyField}
        autoFocus
      >
        {/* The text's own direction, whatever the UI language. */}
        <TextArea
          dir="auto"
          className={styles.replyText}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              send();
            } else if (event.key === "Escape") {
              event.preventDefault();
              onDone();
            }
          }}
        />
      </TextField>
      <div className={styles.actions}>
        <Button type="submit" variant="primary" isDisabled={busy || text.trim() === ""}>
          {t("comments.send")}
        </Button>
        <Button variant="ghost" onPress={onDone}>
          {t("comments.cancel")}
        </Button>
      </div>
    </form>
  );
}

function CommentItem({
  comment,
  selected,
  showPage,
  dates,
  locale,
}: {
  comment: DocumentComment;
  selected: boolean;
  showPage: boolean;
  dates: Intl.DateTimeFormat;
  locale: Locale;
}) {
  const { t } = useTranslation();
  const [replying, setReplying] = useState(false);
  const kind = t(`comments.kinds.${comment.kind}`);
  const page = formatNumber(comment.page + 1, locale);
  return (
    <li
      className={styles.item}
      data-selected={selected || undefined}
      data-resolved={comment.resolved || undefined}
      data-testid="comment"
    >
      <div className={styles.meta}>
        <span className={styles.kind} title={kind} aria-hidden="true">
          {KIND_ICONS[comment.kind]}
        </span>
        <Byline author={comment.author} modified={comment.modified} dates={dates} />
        {showPage && <span className={styles.page}>{t("comments.page", { page })}</span>}
        {comment.resolved && (
          <span className={styles.badge}>
            <CircleCheck aria-hidden="true" />
            {t("comments.resolvedBadge")}
          </span>
        )}
      </div>
      <AriaButton
        className={styles.body}
        aria-label={t("comments.show", { kind, page })}
        onPress={() =>
          void runCommand("comment.show", {
            annotation: {
              page: comment.page,
              index: comment.index,
              kind: comment.kind,
              contents: comment.contents,
              rect: comment.rect,
            },
          })
        }
      >
        {comment.markedText && (
          <q dir="auto" className={styles.quote}>
            {comment.markedText}
          </q>
        )}
        {comment.contents && (
          <span dir="auto" className={styles.text}>
            {comment.contents}
          </span>
        )}
        {!comment.markedText && !comment.contents && (
          <span className={styles.placeholder}>{kind}</span>
        )}
      </AriaButton>
      {comment.replies.length > 0 && (
        <ol className={styles.replies} aria-label={t("comments.replies")}>
          {comment.replies.map((reply) => (
            <li key={reply.index} className={styles.reply}>
              <div className={styles.meta}>
                <Byline author={reply.author} modified={reply.modified} dates={dates} />
              </div>
              <span dir="auto" className={styles.text}>
                {reply.contents}
              </span>
            </li>
          ))}
        </ol>
      )}
      {replying ? (
        <ReplyForm comment={comment} onDone={() => setReplying(false)} />
      ) : (
        <div className={styles.actions}>
          <Button variant="ghost" onPress={() => setReplying(true)}>
            {t("comments.reply")}
          </Button>
          <Button
            variant="ghost"
            onPress={() =>
              void runCommand("comment.resolve", {
                page: comment.page,
                index: comment.index,
                resolved: !comment.resolved,
              })
            }
          >
            {t(comment.resolved ? "comments.reopen" : "comments.resolve")}
          </Button>
        </div>
      )}
    </li>
  );
}

/** The name written on new comments; stored when the field is left or Enter pressed. */
function AuthorField() {
  const { t } = useTranslation();
  const stored = useAppStore((state) => state.commentAuthor);
  const [name, setName] = useState(stored);
  useEffect(() => setName(stored), [stored]);
  const commit = () => {
    if (name.trim() !== stored) void runCommand("comments.setAuthor", { text: name });
  };
  return (
    <TextField
      value={name}
      onChange={setName}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") commit();
      }}
      maxLength={MAX_AUTHOR}
      className={styles.authorField}
    >
      <Label className={styles.authorLabel}>{t("comments.yourName")}</Label>
      <Input
        dir="auto"
        className={styles.authorInput}
        placeholder={t("comments.yourNamePlaceholder")}
        data-testid="comment-author"
      />
    </TextField>
  );
}

/**
 * The document's comments (ADR 0041): filter by status, type and author;
 * sort by page, latest activity, or author; reply, resolve, and go to each.
 * Reloaded whenever the document's annotations change.
 */
export function CommentsPanel({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const filter = useAppStore((state) => state.commentFilter);
  const sort = useAppStore((state) => state.commentSort);
  const open = useAppStore((state) => state.documents.find((candidate) => candidate.id === id));
  const paint = open?.paint ?? 0;
  const revision = open?.revision ?? 0;
  const selected = open?.selectedAnnotation ?? null;
  const [comments, setComments] = useState<DocumentComment[] | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `paint` and `revision` say the annotations changed.
  useEffect(() => {
    let live = true;
    backend.documentComments(id).then(
      (list) => {
        if (live) setComments(list);
      },
      (error: unknown) => {
        console.warn("comments unavailable", error);
        if (live) setComments([]);
      },
    );
    return () => {
      live = false;
    };
  }, [id, paint, revision]);

  const collator = useMemo(() => new Intl.Collator(formatTag(locale)), [locale]);
  const dates = useMemo(
    () => new Intl.DateTimeFormat(formatTag(locale), { dateStyle: "medium", timeStyle: "short" }),
    [locale],
  );
  const authors = useMemo(() => authorsOf(comments ?? [], collator), [comments, collator]);
  const shown = useMemo(
    () => sortComments(filterComments(comments ?? [], filter), sort, collator),
    [comments, filter, sort, collator],
  );

  const setFilter = (patch: Partial<CommentFilter>) =>
    void runCommand("comments.filter", { filter: patch });
  const statusOptions: Option<CommentFilter["status"]>[] = [
    { id: "all", label: t("comments.statusAll") },
    { id: "open", label: t("comments.statusOpen") },
    { id: "resolved", label: t("comments.statusResolved") },
  ];
  const kindOptions: Option<CommentFilter["kind"]>[] = [
    { id: "all", label: t("comments.kindAll") },
    { id: "markup", label: t("comments.kindMarkup") },
    { id: "notes", label: t("comments.kindNotes") },
    { id: "textBoxes", label: t("comments.kindTextBoxes") },
    { id: "ink", label: t("comments.kindInk") },
  ];
  // Select keys are strings: "*" is everyone, "-" the comments without a name.
  const authorOptions: Option<string>[] = [
    { id: "*", label: t("comments.authorAll") },
    ...authors.map((name) => ({ id: `=${name}`, label: name || t("comments.noAuthor") })),
  ];
  const sortOptions: Option<CommentSort>[] = [
    { id: "page", label: t("comments.sortPage") },
    { id: "newest", label: t("comments.sortNewest") },
    { id: "author", label: t("comments.sortAuthor") },
  ];

  // In page order, comments come under their page's heading.
  const groups: { page: number | null; items: DocumentComment[] }[] = [];
  for (const comment of shown) {
    const page = sort === "page" ? comment.page : null;
    const last = groups.at(-1);
    if (last && last.page === page) last.items.push(comment);
    else groups.push({ page, items: [comment] });
  }
  const isSelected = (comment: DocumentComment) =>
    selected?.page === comment.page && selected.index === comment.index;

  return (
    <div className={styles.panel} data-testid="comments">
      <div className={styles.controls}>
        <ChoiceSelect
          label={t("comments.status")}
          value={filter.status}
          options={statusOptions}
          onChoose={(status) => setFilter({ status })}
        />
        <ChoiceSelect
          label={t("comments.kind")}
          value={filter.kind}
          options={kindOptions}
          onChoose={(kind) => setFilter({ kind })}
        />
        {authors.length > 1 && (
          <ChoiceSelect
            label={t("comments.author")}
            value={filter.author === null ? "*" : `=${filter.author}`}
            options={authorOptions}
            onChoose={(key) => setFilter({ author: key === "*" ? null : key.slice(1) })}
          />
        )}
        <ChoiceSelect
          label={t("comments.sort")}
          value={sort}
          options={sortOptions}
          onChoose={(choice) => void runCommand("comments.sort", { sort: choice })}
        />
      </div>
      <div className={styles.scroller}>
        {comments === null ? (
          <p className={styles.status}>{t("comments.loading")}</p>
        ) : comments.length === 0 ? (
          <p className={styles.status}>{t("comments.empty")}</p>
        ) : shown.length === 0 ? (
          <p className={styles.status}>{t("comments.noMatch")}</p>
        ) : (
          groups.map((group) => (
            <section
              key={`${group.page ?? "all"}-${group.items[0]?.page}-${group.items[0]?.index}`}
              className={styles.group}
            >
              {group.page !== null && (
                <h3 className={styles.heading}>
                  {t("comments.page", { page: formatNumber(group.page + 1, locale) })}
                </h3>
              )}
              <ol className={styles.list}>
                {group.items.map((comment) => (
                  <CommentItem
                    key={`${comment.page}-${comment.index}`}
                    comment={comment}
                    selected={isSelected(comment)}
                    showPage={group.page === null}
                    dates={dates}
                    locale={locale}
                  />
                ))}
              </ol>
            </section>
          ))
        )}
      </div>
      <div className={styles.footer}>
        <AuthorField />
      </div>
    </div>
  );
}
