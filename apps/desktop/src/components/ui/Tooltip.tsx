import type { ReactElement, ReactNode } from "react";
import {
  Tooltip as AriaTooltip,
  type TooltipProps as AriaTooltipProps,
  TooltipTrigger,
} from "react-aria-components";
import styles from "./Tooltip.module.css";

export interface TooltipProps {
  /** The tooltip text. */
  label: ReactNode;
  /** Keyboard shortcut shown after the label, already formatted for the platform. */
  shortcut?: string | null;
  /** Logical placement, so it mirrors in RTL. */
  placement?: AriaTooltipProps["placement"];
  /** A single focusable element (Button, IconButton, ToggleButton…). */
  children: ReactElement;
}

export function Tooltip({ label, shortcut, placement = "bottom", children }: TooltipProps) {
  return (
    <TooltipTrigger delay={500} closeDelay={0}>
      {children}
      <AriaTooltip placement={placement} offset={6} className={styles.tooltip}>
        <span>{label}</span>
        {shortcut ? (
          // Shortcuts are Latin key names in every locale; keep them LTR.
          <kbd className={styles.shortcut} dir="ltr">
            {shortcut}
          </kbd>
        ) : null}
      </AriaTooltip>
    </TooltipTrigger>
  );
}
