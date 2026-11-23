import {
  Autocomplete,
  Dialog,
  Input,
  type Key,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  SearchField,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import {
  allCommands,
  type CommandId,
  isCommandEnabled,
  runCommand,
  shortcutLabel,
} from "../../commands/registry";
import { i18n, otherLocale } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import styles from "./CommandPalette.module.css";
import { paletteMatch } from "./match";

/**
 * Every action by name (spec section 4, `Ctrl/Cmd+K`). Titles match in both
 * languages whatever the UI language, so a command can be found by its
 * English or its Arabic name.
 */
interface PaletteEntry {
  id: CommandId;
  title: string;
  /** The title in the other UI language. */
  also: string;
  shortcut: string | null;
}

export function CommandPalette() {
  const open = useAppStore((state) => state.paletteOpen);
  return open ? <PaletteDialog /> : null;
}

function PaletteDialog() {
  const { t } = useTranslation();
  const setOpen = useAppStore((state) => state.setPaletteOpen);
  // The whole state, so enabled/disabled stays current while the palette is open.
  const state = useAppStore();

  const other = i18n.getFixedT(state.locale === "ar" ? "en" : "ar");
  const commands: PaletteEntry[] = allCommands()
    .filter((command) => !command.paletteHidden)
    .map((command) => ({
      id: command.id,
      title: t(`commands.${command.id}`),
      also: other(`commands.${command.id}`),
      shortcut: shortcutLabel(command.id),
    }));
  const disabled = commands
    .filter((command) => !isCommandEnabled(command.id, state))
    .map((command) => command.id);

  return (
    <ModalOverlay isOpen isDismissable onOpenChange={setOpen} className={styles.overlay}>
      <Modal className={styles.modal}>
        <Dialog aria-label={t("palette.label")} className={styles.dialog}>
          <Autocomplete filter={paletteMatch}>
            <SearchField aria-label={t("palette.label")} autoFocus className={styles.field}>
              <Input
                // A command can be looked for in either language, whatever the UI's.
                dir="auto"
                className={styles.input}
                placeholder={t("palette.placeholder")}
                data-testid="palette-input"
              />
            </SearchField>
            <ListBox
              items={commands}
              disabledKeys={disabled}
              className={styles.list}
              renderEmptyState={() => <p className={styles.empty}>{t("palette.none")}</p>}
              onAction={(id: Key) => {
                setOpen(false);
                void runCommand(id as CommandId);
              }}
              data-testid="palette-list"
            >
              {(command: PaletteEntry) => (
                <ListBoxItem
                  id={command.id}
                  // Both languages' titles, so either finds the command.
                  textValue={`${command.title} ${command.also}`}
                  className={styles.item}
                >
                  <span
                    className={styles.title}
                    // The language switch is named in the other language.
                    lang={
                      command.id === "app.toggleLanguage" ? otherLocale(state.locale) : undefined
                    }
                  >
                    {command.title}
                  </span>
                  {command.shortcut ? (
                    // Shortcuts are key names: always left to right.
                    <kbd className={styles.shortcut} dir="ltr">
                      {command.shortcut}
                    </kbd>
                  ) : null}
                </ListBoxItem>
              )}
            </ListBox>
          </Autocomplete>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
