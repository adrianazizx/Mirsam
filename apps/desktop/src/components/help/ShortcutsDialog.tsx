import { Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { allCommands, runCommand, shortcutLabel } from "../../commands/registry";
import { otherLocale } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./ShortcutsDialog.module.css";

/**
 * Every keyboard shortcut, from the command registry (ADR 0070's second pass):
 * the commands the palette leaves out (F6 between areas, F8 between flagged
 * words) are here too, so keyboard and screen-reader users can find them.
 */
export function ShortcutsDialog() {
  const open = useAppStore((state) => state.shortcutsOpen);
  if (!open) return null;
  return <ShortcutsList />;
}

function ShortcutsList() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const close = () => void runCommand("app.closeShortcuts");
  const rows = allCommands().flatMap((command) => {
    const keys = shortcutLabel(command.id);
    return keys ? [{ id: command.id, title: t(`commands.${command.id}`), keys }] : [];
  });

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) close();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="shortcuts">
          <Heading slot="title" className={styles.title}>
            {t("shortcuts.title")}
          </Heading>
          <p className={styles.hint}>{t("shortcuts.intro")}</p>
          <section
            className={styles.body}
            aria-label={t("shortcuts.title")}
            // biome-ignore lint/a11y/noNoninteractiveTabindex: it scrolls, so the keyboard must reach it
            tabIndex={0}
          >
            <table className={styles.table}>
              <thead>
                <tr>
                  <th scope="col">{t("shortcuts.command")}</th>
                  <th scope="col">{t("shortcuts.keys")}</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr key={row.id}>
                    <td
                      // The language switch is named in the other language.
                      lang={row.id === "app.toggleLanguage" ? otherLocale(locale) : undefined}
                    >
                      {row.title}
                    </td>
                    <td>
                      {/* Key names read left to right in both languages. */}
                      <kbd className={styles.keys} dir="ltr">
                        {row.keys}
                      </kbd>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
          <div className={styles.footer}>
            <Button variant="primary" onPress={close}>
              {t("shortcuts.done")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
