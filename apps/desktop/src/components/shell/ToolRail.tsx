import {
  FormInput,
  LayoutGrid,
  type LucideIcon,
  MessageSquare,
  MousePointer2,
  PenLine,
  ScanText,
  ShieldCheck,
  Signature,
} from "lucide-react";
import { type Key, ToggleButton, ToggleButtonGroup } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { TOOLS, type ToolId, useAppStore } from "../../state/appStore";
import { Tooltip } from "../ui/Tooltip";
import styles from "./ToolRail.module.css";

// Fixed order and fixed icons: tools never move between releases (docs/SPEC.md section 1).
const ICONS: Record<ToolId, LucideIcon> = {
  select: MousePointer2,
  edit: PenLine,
  comment: MessageSquare,
  pages: LayoutGrid,
  forms: FormInput,
  sign: Signature,
  protect: ShieldCheck,
  ocr: ScanText,
};

export function ToolRail() {
  const { t } = useTranslation();
  const activeTool = useAppStore((state) => state.activeTool);
  const setActiveTool = useAppStore((state) => state.setActiveTool);

  return (
    <nav
      className={styles.rail}
      aria-label={t("tools.label")}
      data-testid="tool-rail"
      data-region="tools"
    >
      <ToggleButtonGroup
        className={styles.group}
        orientation="vertical"
        selectionMode="single"
        disallowEmptySelection
        aria-label={t("tools.label")}
        selectedKeys={[activeTool]}
        onSelectionChange={(keys: Set<Key>) => {
          const [next] = keys;
          if (typeof next === "string") setActiveTool(next as ToolId);
        }}
      >
        {TOOLS.map((tool) => {
          const Icon = ICONS[tool];
          return (
            <Tooltip key={tool} label={t(`tools.${tool}`)} placement="end">
              <ToggleButton id={tool} aria-label={t(`tools.${tool}`)} className={styles.tool}>
                <Icon aria-hidden="true" />
              </ToggleButton>
            </Tooltip>
          );
        })}
      </ToggleButtonGroup>
    </nav>
  );
}
