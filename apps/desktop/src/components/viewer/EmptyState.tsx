import {
  Command,
  Compass,
  FileText,
  FolderOpen,
  Languages,
  type LucideIcon,
  Settings,
  X,
} from "lucide-react";
import { useEffect, useId } from "react";
import { Button as AriaButton } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { type CommandId, refreshRecents, runCommand, shortcutLabel } from "../../commands/registry";
import { formatTag, isolate } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import styles from "./EmptyState.module.css";

/** The start screen's first actions besides opening a file (M7 onboarding). */
const FIRST_ACTIONS: { command: CommandId; icon: LucideIcon; key: string }[] = [
  { command: "help.guide", icon: Compass, key: "guide" },
  { command: "app.commandPalette", icon: Command, key: "palette" },
  { command: "app.toggleLanguage", icon: Languages, key: "language" },
  { command: "app.settings", icon: Settings, key: "settings" },
];

/** The start screen: open a file, pick up a recent one, or a first step (the guide, the palette, the language, Settings). */
export function EmptyState() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const recents = useAppStore((state) => state.recents);

  useEffect(() => {
    void refreshRecents();
  }, []);

  const date = new Intl.DateTimeFormat(formatTag(locale), { dateStyle: "medium" });

  return (
    <div className={styles.empty}>
      <FileText className={styles.icon} aria-hidden="true" />
      <h1 className={styles.title}>{t("viewer.emptyTitle")}</h1>
      <p className={styles.body}>{t("viewer.emptyBody")}</p>
      <Button variant="primary" icon={<FolderOpen />} onPress={() => void runCommand("file.open")}>
        {t("commands.file.open")}
      </Button>
      <FirstActions />
      {recents && recents.length > 0 ? (
        <section className={styles.recents} aria-labelledby="recents-title">
          <div className={styles.recentsHeader}>
            <h2 id="recents-title" className={styles.recentsTitle}>
              {t("recents.title")}
            </h2>
            <AriaButton
              className={styles.clear}
              onPress={() => void runCommand("file.clearRecents")}
            >
              {t("commands.file.clearRecents")}
            </AriaButton>
          </div>
          <ul className={styles.list} data-testid="recents">
            {recents.map((recent) => (
              <li key={recent.token} className={styles.recent}>
                <AriaButton
                  className={styles.open}
                  isDisabled={!recent.exists}
                  onPress={() => void runCommand("file.openRecent", { token: recent.token })}
                >
                  <bdi className={styles.name}>{recent.name}</bdi>
                  <span className={styles.meta}>
                    {recent.exists
                      ? date.format(new Date(recent.lastOpened * 1000))
                      : t("recents.missing")}
                  </span>
                </AriaButton>
                <IconButton
                  label={t("recents.forget", { name: isolate(recent.name) })}
                  onPress={() => void runCommand("file.forgetRecent", { token: recent.token })}
                >
                  <X />
                </IconButton>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}

/**
 * First steps for a new user, each a registry command. Each card's name is
 * all its text (WCAG 2.5.3). The language card names the other language in
 * that language (as the command's title does).
 */
function FirstActions() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const headingId = useId();
  return (
    <section className={styles.first} aria-labelledby={headingId}>
      <h2 id={headingId} className="visually-hidden">
        {t("onboarding.start.heading")}
      </h2>
      <ul className={styles.actions}>
        {FIRST_ACTIONS.map(({ command, icon: Icon, key }) => {
          const shortcut = shortcutLabel(command);
          const label =
            key === "language" ? (
              <span lang={locale === "ar" ? "en" : "ar"}>{t(`commands.${command}`)}</span>
            ) : (
              t(`onboarding.start.${key}`)
            );
          return (
            <li key={command}>
              <AriaButton className={styles.action} onPress={() => void runCommand(command)}>
                <Icon className={styles.actionIcon} aria-hidden="true" />
                <span className={styles.actionText}>
                  <span className={styles.actionLabel}>{label}</span>
                  <span className={styles.actionAbout}>
                    {t(`onboarding.start.${key}About`)}
                    {shortcut ? (
                      <>
                        {" "}
                        <kbd className={styles.kbd} dir="ltr">
                          {shortcut}
                        </kbd>
                      </>
                    ) : null}
                  </span>
                </span>
              </AriaButton>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
