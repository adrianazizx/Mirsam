import { useEffect, useState } from "react";
import {
  Dialog,
  Heading,
  Input,
  Label,
  Modal,
  ModalOverlay,
  NumberField,
  Radio,
  RadioGroup,
  Text,
} from "react-aria-components";
import { Trans, useTranslation } from "react-i18next";
import { type BookmarkPart, backend, type SplitBy } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatTag } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./SplitDialog.module.css";

type Method = SplitBy["kind"];

/** Bytes in the dialog's megabyte, as file managers count them. */
const MEGABYTE = 1_000_000;

/** Asks how to split: pages per file, at top-level bookmarks, or largest file size (ADR 0016). */
export function SplitDialog() {
  const { t } = useTranslation();
  const open = useAppStore((state) => state.splitPrompt);
  const active = useAppStore((state) => selectActiveDocument(state));
  if (!open || !active || active.pageCount < 2) return null;
  return <SplitForm documentId={active.id} pageCount={active.pageCount} title={t("split.title")} />;
}

function SplitForm({
  documentId,
  pageCount,
  title,
}: {
  documentId: number;
  pageCount: number;
  title: string;
}) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const [method, setMethod] = useState<Method>("pages");
  const [size, setSize] = useState(Math.ceil(pageCount / 2));
  const [megabytes, setMegabytes] = useState(10);
  const [bookmarks, setBookmarks] = useState<BookmarkPart[] | null>(null);
  const cancel = () => useAppStore.getState().setSplitPrompt(false);

  useEffect(() => {
    let live = true;
    backend.bookmarkParts(documentId).then(
      (parts) => {
        if (live) setBookmarks(parts);
      },
      () => {
        if (live) setBookmarks([]);
      },
    );
    return () => {
      live = false;
    };
  }, [documentId]);

  const canSplitAtBookmarks = (bookmarks?.length ?? 0) >= 2;
  const valid =
    method === "pages"
      ? Number.isInteger(size) && size >= 1 && size < pageCount
      : method === "bytes"
        ? Number.isFinite(megabytes) && megabytes >= 0.1
        : canSplitAtBookmarks;
  const split = (): SplitBy =>
    method === "pages"
      ? { kind: "pages", size }
      : method === "bytes"
        ? { kind: "bytes", limit: Math.round(megabytes * MEGABYTE) }
        : { kind: "bookmarks" };
  const shown = (bookmarks ?? []).slice(0, 5).map((part) => part.title);
  const titles =
    new Intl.ListFormat(formatTag(locale), { type: "unit" }).format(shown) +
    ((bookmarks?.length ?? 0) > shown.length ? "…" : "");

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) cancel();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog}>
          <form
            className={styles.form}
            onSubmit={(event) => {
              event.preventDefault();
              if (valid) void runCommand("pages.splitInto", { split: split() });
            }}
          >
            <Heading slot="title" className={styles.title}>
              {title}
            </Heading>
            <RadioGroup
              value={method}
              onChange={(value) => setMethod(value as Method)}
              className={styles.methods}
            >
              <Label className={styles.label}>{t("split.method")}</Label>
              <Radio value="pages" className={styles.method}>
                {t("split.byPages")}
              </Radio>
              <Radio value="bookmarks" className={styles.method} isDisabled={!canSplitAtBookmarks}>
                {t("split.byBookmarks")}
              </Radio>
              <Radio value="bytes" className={styles.method}>
                {t("split.bySize")}
              </Radio>
            </RadioGroup>
            {method === "pages" && (
              <NumberField
                className={styles.field}
                value={size}
                onChange={setSize}
                minValue={1}
                maxValue={pageCount - 1}
                step={1}
                formatOptions={{ maximumFractionDigits: 0 }}
                isInvalid={!valid}
                autoFocus
                aria-describedby="split-help"
              >
                <Label className={styles.label}>{t("split.size")}</Label>
                <Input className={styles.input} lang={locale} />
                <Text slot="description" id="split-help" className={styles.help}>
                  {t("split.help")}
                </Text>
              </NumberField>
            )}
            {method === "bytes" && (
              <NumberField
                className={styles.field}
                value={megabytes}
                onChange={setMegabytes}
                minValue={0.1}
                step={0.5}
                formatOptions={{ maximumFractionDigits: 1 }}
                isInvalid={!valid}
                autoFocus
                aria-describedby="split-size-help"
              >
                <Label className={styles.label}>{t("split.megabytes")}</Label>
                <Input className={styles.input} lang={locale} />
                <Text slot="description" id="split-size-help" className={styles.help}>
                  {t("split.sizeHelp")}
                </Text>
              </NumberField>
            )}
            {method === "bookmarks" && (
              // Titles can be in either script whatever the UI language.
              <p className={styles.help} data-testid="split-bookmarks">
                <Trans
                  i18nKey="split.bookmarksHelp"
                  values={{ titles }}
                  components={{ bdi: <bdi /> }}
                />
              </p>
            )}
            {bookmarks !== null && !canSplitAtBookmarks && (
              <p className={styles.help}>{t("split.noBookmarks")}</p>
            )}
            <div className={styles.actions}>
              <Button variant="primary" type="submit" isDisabled={!valid}>
                {t("split.confirm")}
              </Button>
              <Button variant="ghost" onPress={cancel}>
                {t("split.cancel")}
              </Button>
            </div>
          </form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
