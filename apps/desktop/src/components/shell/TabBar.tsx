import { Command, FolderOpen, Languages, Moon, Search, Sun, X } from "lucide-react";
import { type PointerEvent as ReactPointerEvent, useMemo, useRef } from "react";
import { Tab, TabList } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { resolvedTheme, runCommand, shortcutLabel } from "../../commands/registry";
import { directionOf, isolate, otherLocale } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import { Tooltip } from "../ui/Tooltip";
import styles from "./TabBar.module.css";
import { DRAG_THRESHOLD, dropIndex } from "./tabDrag";

export function TabBar() {
  const { t } = useTranslation();
  // Ids and names only, so scrolling (current page) and zooming do not re-render the tabs.
  const ids = useAppStore(useShallow((state) => state.documents.map((open) => open.id)));
  const names = useAppStore(useShallow((state) => state.documents.map((open) => open.name)));
  const modified = useAppStore(useShallow((state) => state.documents.map((open) => open.modified)));
  const documents = useMemo(
    () =>
      ids.map((id, index) => ({
        id,
        name: names[index] ?? "",
        modified: modified[index] ?? false,
      })),
    [ids, names, modified],
  );
  const theme = useAppStore((state) => state.theme);
  const isDark = resolvedTheme(theme) === "dark";
  const locale = useAppStore((state) => state.locale);
  const drag = useRef<{ id: number; startX: number; dragging: boolean } | null>(null);

  // Dragging a tab reorders it, live, through `tabs.move` (keyboard:
  // Ctrl+Shift+PageUp/PageDown). Tabs are found by their data attribute.
  const onPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    const tab = (event.target as HTMLElement).closest<HTMLElement>("[data-document-id]");
    if (event.button !== 0 || !tab) return;
    drag.current = { id: Number(tab.dataset.documentId), startX: event.clientX, dragging: false };
  };
  const onPointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    const current = drag.current;
    if (!current) return;
    if (!current.dragging) {
      if (Math.abs(event.clientX - current.startX) < DRAG_THRESHOLD) return;
      current.dragging = true;
      event.currentTarget.setPointerCapture(event.pointerId);
    }
    const tabs = [...event.currentTarget.querySelectorAll<HTMLElement>("[data-document-id]")];
    const centres = tabs.map((tab) => {
      const box = tab.getBoundingClientRect();
      return box.left + box.width / 2;
    });
    const from = tabs.findIndex((tab) => Number(tab.dataset.documentId) === current.id);
    const to = dropIndex(centres, from, event.clientX, directionOf(locale));
    if (to !== from) void runCommand("tabs.move", { documentId: current.id, index: to });
  };
  const onPointerEnd = (event: ReactPointerEvent<HTMLElement>) => {
    if (drag.current?.dragging) event.currentTarget.releasePointerCapture(event.pointerId);
    drag.current = null;
  };

  return (
    <header
      className={styles.bar}
      data-region="tabs"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
    >
      <TabList
        aria-label={t("tabs.label")}
        items={documents}
        // React Aria keeps rendered items while `items` is unchanged: re-render them on a
        // language switch, or the close buttons keep their old labels.
        dependencies={[t]}
        className={styles.tabs}
      >
        {(document: { id: number; name: string; modified: boolean }) => (
          <Tab
            id={document.id}
            data-document-id={document.id}
            className={styles.tab}
            // Middle-click closes a tab, as in browsers.
            onAuxClick={(event: React.MouseEvent) => {
              if (event.button === 1) void runCommand("file.close", { documentId: document.id });
            }}
          >
            {/* A file name can be in either script regardless of UI language. */}
            {/* Cut short with an ellipsis when tabs are many or the text large: the title shows it whole. */}
            <bdi className={styles.name} title={document.name}>
              {document.name}
            </bdi>
            {document.modified && (
              <>
                <span className={styles.modified} aria-hidden="true" />
                {/* A tab's content is its name: "contract.pdf, unsaved changes". */}
                <span className="visually-hidden">
                  {t("a11y.separator")}
                  {t("tabs.modified")}
                </span>
              </>
            )}
            {/*
             * Not a button: a tab's content is presentational, so a control
             * inside it is unreachable for screen readers and nests one
             * interactive element in another. The keyboard and screen readers
             * close the tab with Cmd/Ctrl+W (`file.close`); this is the
             * pointer's shortcut to the same command.
             */}
            <span
              className={styles.close}
              aria-hidden="true"
              title={t("tabs.close", { name: isolate(document.name) })}
              data-testid="tab-close"
              // Pressing here must not select the tab or start dragging it.
              onPointerDown={(event) => event.stopPropagation()}
              onClick={(event) => {
                event.stopPropagation();
                void runCommand("file.close", { documentId: document.id });
              }}
            >
              <X />
            </span>
          </Tab>
        )}
      </TabList>
      <div className={styles.actions}>
        <CommandButton command="search.focus">
          <Search />
        </CommandButton>
        <CommandButton command="app.commandPalette">
          <Command />
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
        {/* Named in the other language ("العربية", "English"): spoken in that language's voice. */}
        <CommandButton command="app.toggleLanguage" labelLang={otherLocale(locale)}>
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
