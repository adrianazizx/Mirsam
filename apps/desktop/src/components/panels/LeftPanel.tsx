import {
  ListTree,
  MessageSquare,
  PanelLeftClose,
  Search,
  ShieldCheck,
  SquareStack,
} from "lucide-react";
import { ToggleButton, ToggleButtonGroup } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { type CommandId, runCommand } from "../../commands/registry";
import { type LeftPanel as Panel, selectActiveDocument, useAppStore } from "../../state/appStore";
import { SignaturesPanel } from "../signing/SignaturesPanel";
import { CommandButton } from "../ui/CommandButton";
import { Tooltip } from "../ui/Tooltip";
import { CommentsPanel } from "./CommentsPanel";
import styles from "./LeftPanel.module.css";
import { OutlinePanel } from "./OutlinePanel";
import { SearchPanel } from "./SearchPanel";
import { ThumbnailsPanel } from "./ThumbnailsPanel";

const VIEWS: { panel: Panel; command: CommandId; icon: React.ReactNode }[] = [
  { panel: "thumbnails", command: "view.showThumbnails", icon: <SquareStack /> },
  { panel: "outline", command: "view.showOutline", icon: <ListTree /> },
  { panel: "search", command: "search.focus", icon: <Search /> },
  { panel: "comments", command: "view.showComments", icon: <MessageSquare /> },
  { panel: "signatures", command: "view.showSignatures", icon: <ShieldCheck /> },
];

/**
 * The left panel (spec section 4): thumbnails, outline, search results, and
 * comments (ADR 0041), and digital signatures (ADR 0048) for the active document. "Left" in the spec's English layout; it sits at the
 * inline start, so it is on the right in the Arabic UI.
 */
export function LeftPanel() {
  const { t } = useTranslation();
  const panel = useAppStore((state) => state.leftPanel);
  const documentId = useAppStore((state) => selectActiveDocument(state)?.id ?? null);
  if (!panel) return null;

  return (
    <aside
      className={styles.panel}
      aria-label={t("panel.label")}
      data-testid="left-panel"
      data-region="panel"
    >
      <div className={styles.header}>
        <ToggleButtonGroup
          aria-label={t("panel.views")}
          selectionMode="single"
          disallowEmptySelection
          selectedKeys={[panel]}
          className={styles.views}
        >
          {VIEWS.map((view) => (
            <Tooltip key={view.panel} label={t(`panel.${view.panel}`)}>
              <ToggleButton
                id={view.panel}
                className={styles.view}
                aria-label={t(`panel.${view.panel}`)}
                onPress={() => void runCommand(view.command)}
              >
                <span className={styles.glyph} aria-hidden="true">
                  {view.icon}
                </span>
              </ToggleButton>
            </Tooltip>
          ))}
        </ToggleButtonGroup>
        {/* The icon draws a panel on its start side, so it mirrors with the layout. */}
        <CommandButton command="view.toggleLeftPanel" label={t("panel.close")} mirrorInRtl>
          <PanelLeftClose />
        </CommandButton>
      </div>
      <div className={styles.body}>
        {documentId === null ? (
          <p className={styles.empty}>{t("panel.noDocument")}</p>
        ) : panel === "thumbnails" ? (
          <ThumbnailsPanel key={documentId} documentId={documentId} />
        ) : panel === "outline" ? (
          <OutlinePanel key={documentId} documentId={documentId} />
        ) : panel === "comments" ? (
          <CommentsPanel key={documentId} documentId={documentId} />
        ) : panel === "signatures" ? (
          <SignaturesPanel key={documentId} documentId={documentId} />
        ) : (
          <SearchPanel key={documentId} documentId={documentId} />
        )}
      </div>
    </aside>
  );
}
