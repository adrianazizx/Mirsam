import {
  Copy,
  EyeOff,
  Highlighter,
  LineSquiggle,
  Link,
  Maximize2,
  Minimize2,
  Strikethrough,
  Trash2,
  Underline,
} from "lucide-react";
import { Toolbar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { FILL_KINDS } from "../../backend";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { CommandButton } from "../ui/CommandButton";
import styles from "./SelectionToolbar.module.css";

/**
 * The floating toolbar over selected text (spec section 4): copy, and mark
 * the text as standard PDF annotations (ADR 0017), or mark it for redaction
 * (ADR 0042). Over a selected
 * annotation (markup or ink) it offers deleting it instead.
 */
export function SelectionToolbar({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const selectedKind = useAppStore(
    (state) => state.documents.find((open) => open.id === documentId)?.selectedAnnotation?.kind,
  );
  const annotationSelected = selectedKind !== undefined;
  const fillSelected = selectedKind !== undefined && FILL_KINDS.includes(selectedKind);
  return (
    <div className={styles.band}>
      {/* The page area is always left to right; the toolbar follows the UI language. */}
      <Toolbar
        className={styles.toolbar}
        aria-label={t(annotationSelected ? "selection.annotationToolbar" : "selection.toolbar")}
        dir={directionOf(locale)}
        data-testid="selection-toolbar"
      >
        {annotationSelected ? (
          <>
            {fillSelected ? (
              <>
                <CommandButton command="fill.smaller" tooltipPlacement="top">
                  <Minimize2 />
                </CommandButton>
                <CommandButton command="fill.larger" tooltipPlacement="top">
                  <Maximize2 />
                </CommandButton>
              </>
            ) : null}
            <CommandButton command="annotation.delete" tooltipPlacement="top">
              <Trash2 />
            </CommandButton>
          </>
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
            <CommandButton command="link.fromSelection" tooltipPlacement="top">
              <Link />
            </CommandButton>
            <CommandButton command="redact.markSelection" tooltipPlacement="top">
              <EyeOff />
            </CommandButton>
          </>
        )}
      </Toolbar>
    </div>
  );
}
