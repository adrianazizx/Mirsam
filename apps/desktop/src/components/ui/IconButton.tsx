import type { ReactNode } from "react";
import { Button as AriaButton, type ButtonProps as AriaButtonProps } from "react-aria-components";
import styles from "./IconButton.module.css";
import { Tooltip, type TooltipProps } from "./Tooltip";

export interface IconButtonProps extends Omit<AriaButtonProps, "className" | "children"> {
  /** Accessible name; also shown as the tooltip. Required: an icon alone says nothing. */
  label: string;
  shortcut?: string | null;
  tooltipPlacement?: TooltipProps["placement"];
  /**
   * Mirror the glyph in RTL. Set for icons that encode direction of travel
   * (back/forward chevrons, undo/redo); leave unset for media-style and neutral
   * icons, which must not flip (docs/SPEC.md section 6.5).
   */
  mirrorInRtl?: boolean;
  children: ReactNode;
}

export function IconButton({
  label,
  shortcut,
  tooltipPlacement,
  mirrorInRtl = false,
  children,
  ...props
}: IconButtonProps) {
  return (
    <Tooltip label={label} shortcut={shortcut} placement={tooltipPlacement}>
      <AriaButton {...props} aria-label={label} className={styles.button}>
        <span className={mirrorInRtl ? styles.mirrored : styles.glyph} aria-hidden="true">
          {children}
        </span>
      </AriaButton>
    </Tooltip>
  );
}
