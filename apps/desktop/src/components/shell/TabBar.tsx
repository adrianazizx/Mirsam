import { FolderOpen, Languages, Moon, Sun, X } from "lucide-react";
import { Tab, TabList } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { resolvedTheme, runCommand, shortcutLabel } from "../../commands/registry";
import { type OpenDocument, useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import { Tooltip } from "../ui/Tooltip";
import styles from "./TabBar.module.css";

export function TabBar() {
  const { t } = useTranslation();
  const documents = useAppStore((state) => state.documents);
  const theme = useAppStore((state) => state.theme);
  const isDark = resolvedTheme(theme) === "dark";

  return (
    <header className={styles.bar}>
      <TabList aria-label={t("tabs.label")} items={documents} className={styles.tabs}>
        {(document: OpenDocument) => (
          <Tab id={document.id} className={styles.tab}>
            {/* A file name can be in either script regardless of UI language. */}
            <bdi className={styles.name}>{document.name}</bdi>
          </Tab>
        )}
      </TabList>
      <div className={styles.actions}>
        <CommandButton command="file.close">
          <X />
        </CommandButton>
        <Tooltip label={t("commands.file.open")} shortcut={shortcutLabel("file.open")}>
          <Button
            variant="ghost"
            icon={<FolderOpen />}
            onPress={() => void runCommand("file.open")}
          >
            {t("commands.file.open")}
          </Button>
        </Tooltip>
        <span className={styles.divider} aria-hidden="true" />
        <CommandButton command="app.toggleLanguage">
          <Languages />
        </CommandButton>
        <CommandButton
          command="app.toggleTheme"
          label={`${t("commands.app.toggleTheme")}: ${t(isDark ? "theme.light" : "theme.dark")}`}
        >
          {isDark ? <Sun /> : <Moon />}
        </CommandButton>
      </div>
    </header>
  );
}
