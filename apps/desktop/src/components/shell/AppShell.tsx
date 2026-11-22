import type { Key } from "react-aria-components";
import { TabPanel, Tabs } from "react-aria-components";
import { Gallery } from "../../gallery/Gallery";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { DocumentView } from "../viewer/DocumentView";
import { EmptyState } from "../viewer/EmptyState";
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

  return (
    <Tabs
      className={styles.shell}
      selectedKey={activeDocumentId}
      onSelectionChange={(key: Key) => {
        if (typeof key === "number") activateDocument(key);
      }}
    >
      <TabBar />
      <ToolRail />
      <main className={styles.main}>
        <ErrorBanner />
        {galleryOpen ? (
          <Gallery />
        ) : activeDocumentId !== null ? (
          <TabPanel id={activeDocumentId} className={styles.panel}>
            <DocumentView documentId={activeDocumentId} />
          </TabPanel>
        ) : (
          <EmptyState />
        )}
      </main>
      <Inspector />
      <StatusBar />
    </Tabs>
  );
}
