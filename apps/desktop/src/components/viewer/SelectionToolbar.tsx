import { Copy } from "lucide-react";
import { Toolbar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { CommandButton } from "../ui/CommandButton";
import styles from "./SelectionToolbar.module.css";

/**
 * The floating toolbar over selected text (spec section 4). Milestone 1 has
 * only Copy; highlight, comment, and redact join it in later milestones.
 */
export function SelectionToolbar() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  return (
    <div className={styles.band}>
      {/* The page area is always left to right; the toolbar follows the UI language. */}
      <Toolbar
        className={styles.toolbar}
        aria-label={t("selection.toolbar")}
        dir={directionOf(locale)}
        data-testid="selection-toolbar"
      >
        <CommandButton command="edit.copy" tooltipPlacement="top">
          <Copy />
        </CommandButton>
      </Toolbar>
    </div>
  );
}
