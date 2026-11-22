import { Copy, Highlighter, LineSquiggle, Strikethrough, Trash2, Underline } from "lucide-react";
import { Toolbar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { CommandButton } from "../ui/CommandButton";
import styles from "./SelectionToolbar.module.css";

/**
 * The floating toolbar over selected text (spec section 4): copy, and mark
 * the text as standard PDF annotations (ADR 0017). Over a selected markup
 * annotation it offers deleting it instead.
 */
export function SelectionToolbar({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const markupSelected = useAppStore((state) =>
    Boolean(state.documents.find((open) => open.id === documentId)?.selectedMarkup),
  );
  return (
    <div className={styles.band}>
      {/* The page area is always left to right; the toolbar follows the UI language. */}
      <Toolbar
        className={styles.toolbar}
        aria-label={t(markupSelected ? "selection.markupToolbar" : "selection.toolbar")}
        dir={directionOf(locale)}
        data-testid="selection-toolbar"
      >
        {markupSelected ? (
          <CommandButton command="markup.delete" tooltipPlacement="top">
            <Trash2 />
          </CommandButton>
        ) : (
          <>
            <CommandButton command="edit.copy" tooltipPlacement="top">
              <Copy />
            </CommandButton>
            <CommandButton command="markup.highlight" tooltipPlacement="top">
              <Highlighter />
            </CommandButton>
            <CommandButton command="markup.underline" tooltipPlacement="top">
              <Underline />
            </CommandButton>
            <CommandButton command="markup.strikeOut" tooltipPlacement="top">
              <Strikethrough />
            </CommandButton>
            <CommandButton command="markup.squiggly" tooltipPlacement="top">
              <LineSquiggle />
            </CommandButton>
          </>
        )}
      </Toolbar>
    </div>
  );
}
