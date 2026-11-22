import { FolderOpen, Languages, Moon, Sun, X } from "lucide-react";
import { useMemo } from "react";
import { Tab, TabList } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { resolvedTheme, runCommand, shortcutLabel } from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import { Tooltip } from "../ui/Tooltip";
import styles from "./TabBar.module.css";

export function TabBar() {
  const { t } = useTranslation();
  // Ids and names only, so scrolling (current page) and zooming do not re-render the tabs.
  const ids = useAppStore(useShallow((state) => state.documents.map((open) => open.id)));
  const names = useAppStore(useShallow((state) => state.documents.map((open) => open.name)));
  const documents = useMemo(
    () => ids.map((id, index) => ({ id, name: names[index] ?? "" })),
    [ids, names],
  );
  const theme = useAppStore((state) => state.theme);
  const isDark = resolvedTheme(theme) === "dark";

  return (
    <header className={styles.bar}>
      <TabList aria-label={t("tabs.label")} items={documents} className={styles.tabs}>
        {(document: { id: number; name: string }) => (
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
