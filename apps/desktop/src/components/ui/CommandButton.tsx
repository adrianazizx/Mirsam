import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  ariaShortcut,
  type CommandId,
  isCommandEnabled,
  runCommand,
  shortcutLabel,
} from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import { IconButton, type IconButtonProps } from "./IconButton";

export interface CommandButtonProps
  extends Pick<IconButtonProps, "mirrorInRtl" | "tooltipPlacement" | "labelLang"> {
  command: CommandId;
  /** Overrides the command's title, e.g. to name the state a toggle leads to. */
  label?: string;
  children: ReactNode;
}

/** An icon button whose label, shortcut, enabled state, and action come from the registry. */
export function CommandButton({ command, label, children, ...props }: CommandButtonProps) {
  const { t } = useTranslation();
  const enabled = useAppStore((state) => isCommandEnabled(command, state));
  return (
    <IconButton
      {...props}
      label={label ?? t(`commands.${command}`)}
      shortcut={shortcutLabel(command)}
      keyShortcuts={ariaShortcut(command)}
      isDisabled={!enabled}
      onPress={() => void runCommand(command)}
    >
      {children}
    </IconButton>
  );
}
