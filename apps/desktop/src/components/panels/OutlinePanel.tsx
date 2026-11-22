import { ChevronRight } from "lucide-react";
import { useEffect, useMemo } from "react";
import {
  Button,
  Collection,
  type Key,
  Tree,
  TreeItem,
  TreeItemContent,
  type TreeItemContentRenderProps,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { backend, type OutlineItem } from "../../backend";
import { runCommand } from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import styles from "./OutlinePanel.module.css";

interface Node {
  id: string;
  item: OutlineItem;
  children: Node[];
}

function toNodes(items: OutlineItem[], prefix = ""): Node[] {
  return items.map((item, index) => {
    const id = `${prefix}${index}`;
    return { id, item, children: toNodes(item.children, `${id}.`) };
  });
}

function findNode(nodes: Node[], id: string): Node | undefined {
  for (const node of nodes) {
    if (node.id === id) return node;
    const found = findNode(node.children, id);
    if (found) return found;
  }
  return undefined;
}

function OutlineNode({ node }: { node: Node }) {
  const { t } = useTranslation();
  return (
    <TreeItem id={node.id} textValue={node.item.title} className={styles.item}>
      <TreeItemContent>
        {({ hasChildItems }: TreeItemContentRenderProps) => (
          <div className={styles.row}>
            {hasChildItems ? (
              // "Expand" points along the reading direction, so it mirrors in RTL.
              <Button slot="chevron" className={styles.chevron} aria-label={t("outline.toggle")}>
                <ChevronRight aria-hidden="true" />
              </Button>
            ) : (
              <span className={styles.spacer} aria-hidden="true" />
            )}
            {/* Titles can be in either script whatever the UI language. */}
            <bdi className={styles.title}>{node.item.title}</bdi>
          </div>
        )}
      </TreeItemContent>
      <Collection items={node.children}>{(child: Node) => <OutlineNode node={child} />}</Collection>
    </TreeItem>
  );
}

/** The document's bookmarks; choosing one goes to its page. */
export function OutlinePanel({ documentId: id }: { documentId: number }) {
  const { t } = useTranslation();
  const outline = useAppStore(
    (state) => state.documents.find((open) => open.id === id)?.outline ?? null,
  );

  useEffect(() => {
    if (outline !== null) return;
    let live = true;
    backend.documentOutline(id).then(
      (items) => {
        if (live) useAppStore.getState().updateDocument(id, { outline: items });
      },
      (error: unknown) => {
        console.warn("outline unavailable", error);
        if (live) useAppStore.getState().updateDocument(id, { outline: [] });
      },
    );
    return () => {
      live = false;
    };
  }, [id, outline]);

  const nodes = useMemo(() => toNodes(outline ?? []), [outline]);
  // Top-level entries start open: they are usually chapters.
  const expanded = useMemo(
    () => nodes.filter((node) => node.children.length > 0).map((node) => node.id),
    [nodes],
  );

  if (outline === null) return <p className={styles.status}>{t("outline.loading")}</p>;
  if (nodes.length === 0) return <p className={styles.status}>{t("outline.empty")}</p>;

  return (
    <Tree
      aria-label={t("panel.outline")}
      items={nodes}
      defaultExpandedKeys={expanded}
      className={styles.tree}
      data-testid="outline"
      onAction={(key: Key) => {
        const node = findNode(nodes, String(key));
        if (node?.item.page !== null && node?.item.page !== undefined) {
          void runCommand("view.goToPage", { page: node.item.page });
        }
      }}
    >
      {(node: Node) => <OutlineNode node={node} />}
    </Tree>
  );
}
