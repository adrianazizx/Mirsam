import type { ReactNode } from "react";
import { Button as AriaButton, type ButtonProps as AriaButtonProps } from "react-aria-components";
import styles from "./IconButton.module.css";
import { useKeyShortcutsRef } from "./keyShortcuts";
import { Tooltip, type TooltipProps } from "./Tooltip";

export interface IconButtonProps extends Omit<AriaButtonProps, "className" | "children"> {
  /** Accessible name; also shown as the tooltip. Required: an icon alone says nothing. */
  label: string;
  shortcut?: string | null;
  /** The shortcut in `aria-keyshortcuts` syntax, for screen readers (the tooltip shows `shortcut`). */
  keyShortcuts?: string;
  tooltipPlacement?: TooltipProps["placement"];
  /**
   * Mirror the glyph in RTL. Set for icons that encode direction of travel
   * (back/forward chevrons, undo/redo); leave unset for media-style and neutral
   * icons, which must not flip (docs/SPEC.md section 6.5).
   */
  mirrorInRtl?: boolean;
  /**
   * The label's language, when it is not the UI's (the language switch is
   * named in the other language), so screen readers speak it in that voice.
   */
  labelLang?: string;
  children: ReactNode;
}

export function IconButton({
  label,
  shortcut,
  tooltipPlacement,
  mirrorInRtl = false,
  labelLang,
  keyShortcuts,
  children,
  ...props
}: IconButtonProps) {
  const shortcutsRef = useKeyShortcutsRef<HTMLButtonElement>(keyShortcuts);
  // `aria-label` cannot carry a language; hidden text inside the button can.
  const name = labelLang ? (
    <span lang={labelLang} dir="auto">
      {label}
    </span>
  ) : (
    label
  );
  return (
    <Tooltip label={name} shortcut={shortcut} placement={tooltipPlacement}>
      <AriaButton
        {...props}
        ref={shortcutsRef}
        aria-label={labelLang ? undefined : label}
        className={styles.button}
      >
        <span className={mirrorInRtl ? styles.mirrored : styles.glyph} aria-hidden="true">
          {children}
        </span>
        {labelLang ? <span className="visually-hidden">{name}</span> : null}
      </AriaButton>
    </Tooltip>
  );
}
