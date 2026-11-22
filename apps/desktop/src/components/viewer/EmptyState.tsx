import { FileText, FolderOpen, X } from "lucide-react";
import { useEffect } from "react";
import { Button as AriaButton } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { refreshRecents, runCommand } from "../../commands/registry";
import { formatTag } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import styles from "./EmptyState.module.css";

/** The start screen: open a file, or pick up a recent one. */
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
                  label={t("recents.forget", { name: recent.name })}
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
