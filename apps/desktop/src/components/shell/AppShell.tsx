import type { Key } from "react-aria-components";
import { TabPanel, Tabs } from "react-aria-components";
import { Gallery } from "../../gallery/Gallery";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { CommandPalette } from "../palette/CommandPalette";
import { LeftPanel } from "../panels/LeftPanel";
import { DocumentView } from "../viewer/DocumentView";
import { EmptyState } from "../viewer/EmptyState";
import { UnlockPanel } from "../viewer/UnlockPanel";
import styles from "./AppShell.module.css";
import { ErrorBanner } from "./ErrorBanner";
import { Inspector } from "./Inspector";
import { StatusBar } from "./StatusBar";
import { TabBar } from "./TabBar";
import { ToolRail } from "./ToolRail";

/**
 * The window layout from docs/SPEC.md section 4: tabs across the top, tool rail
 * at the inline start, page canvas in the middle, inspector at the inline end,
 * status bar along the bottom. Built on logical properties only, so the whole
 * arrangement mirrors when the root direction is RTL.
 */
export function AppShell() {
  // Only which document is active: the shell must not re-render as the
  // current page or zoom of that document changes.
  const activeDocumentId = useAppStore((state) => selectActiveDocument(state)?.id ?? null);
  const activateDocument = useAppStore((state) => state.activateDocument);
  const galleryOpen = useAppStore((state) => state.galleryOpen);
  const leftPanelOpen = useAppStore((state) => state.leftPanel !== null);
  const unlocking = useAppStore((state) => state.unlocks.length > 0);

  return (
    <div className={styles.shell} data-left-panel={leftPanelOpen ? "" : undefined}>
      {/*
       * Tabs wraps only the tab list and the page area. React Aria builds the
       * tab collection by rendering everything inside <Tabs>, so a list with
       * its own items (search results, the outline, the palette) placed in
       * there would have its item keys collide with the tabs'. `display:
       * contents` keeps both children in the shell's grid.
       */}
      <Tabs
        className={styles.tabs}
        selectedKey={activeDocumentId}
        onSelectionChange={(key: Key) => {
          if (typeof key === "number") activateDocument(key);
        }}
      >
        <TabBar />
        <main className={styles.main}>
          <ErrorBanner />
          {galleryOpen ? (
            <Gallery />
          ) : unlocking ? (
            <UnlockPanel />
          ) : activeDocumentId !== null ? (
            <TabPanel id={activeDocumentId} className={styles.panel}>
              <DocumentView documentId={activeDocumentId} />
            </TabPanel>
          ) : (
            <EmptyState />
          )}
        </main>
      </Tabs>
      <ToolRail />
      <LeftPanel />
      <Inspector />
      <StatusBar />
      <CommandPalette />
    </div>
  );
}
