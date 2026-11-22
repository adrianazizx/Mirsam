import { Eraser, Pen, StickyNote, TextCursorInput } from "lucide-react";
import { type Key, ToggleButton, ToggleButtonGroup } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { type CommentTool, useAppStore } from "../../state/appStore";
import { Tooltip } from "../ui/Tooltip";
import styles from "./CommentToolbar.module.css";

const TOOLS: { id: CommentTool; icon: typeof Pen }[] = [
  { id: "note", icon: StickyNote },
  { id: "textBox", icon: TextCursorInput },
  { id: "pen", icon: Pen },
  { id: "eraser", icon: Eraser },
];

/**
 * The Comment tool's modes (ADRs 0018, 0019): place notes and text boxes,
 * draw, erase ink. Markup comes from the toolbar over selected text.
 */
export function CommentToolbar() {
  const { t } = useTranslation();
  const tool = useAppStore((state) => state.commentTool);
  return (
    <div className={styles.bar}>
      <ToggleButtonGroup
        className={styles.group}
        selectionMode="single"
        disallowEmptySelection
        aria-label={t("comment.tools")}
        selectedKeys={[tool]}
        onSelectionChange={(keys: Set<Key>) => {
          const [next] = keys;
          if (next === "note" || next === "textBox" || next === "pen" || next === "eraser") {
            void runCommand(`comment.${next}`);
          }
        }}
        data-testid="comment-toolbar"
      >
        {TOOLS.map(({ id, icon: Icon }) => (
          <Tooltip key={id} label={t(`commands.comment.${id}`)} placement="bottom">
            <ToggleButton id={id} aria-label={t(`commands.comment.${id}`)} className={styles.tool}>
              <Icon aria-hidden="true" />
            </ToggleButton>
          </Tooltip>
        ))}
      </ToggleButtonGroup>
    </div>
  );
}
