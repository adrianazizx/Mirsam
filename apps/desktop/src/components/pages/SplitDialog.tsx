import { useState } from "react";
import {
  Dialog,
  Heading,
  Input,
  Label,
  Modal,
  ModalOverlay,
  NumberField,
  Text,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./SplitDialog.module.css";

/** Asks how many pages each file of a split gets (ADR 0016). */
export function SplitDialog() {
  const { t } = useTranslation();
  const open = useAppStore((state) => state.splitPrompt);
  const pageCount = useAppStore((state) => selectActiveDocument(state)?.pageCount ?? 0);
  if (!open || pageCount < 2) return null;
  return <SplitForm pageCount={pageCount} title={t("split.title")} />;
}

function SplitForm({ pageCount, title }: { pageCount: number; title: string }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const [size, setSize] = useState(Math.ceil(pageCount / 2));
  const cancel = () => useAppStore.getState().setSplitPrompt(false);
  const valid = Number.isInteger(size) && size >= 1 && size < pageCount;

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
              if (valid) void runCommand("pages.splitInto", { size });
            }}
          >
            <Heading slot="title" className={styles.title}>
              {title}
            </Heading>
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
              // Western or Arabic-Indic digits follow the UI language (ADR 0003).
              aria-describedby="split-help"
            >
              <Label className={styles.label}>{t("split.size")}</Label>
              <Input className={styles.input} lang={locale} />
              <Text slot="description" id="split-help" className={styles.help}>
                {t("split.help")}
              </Text>
            </NumberField>
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
