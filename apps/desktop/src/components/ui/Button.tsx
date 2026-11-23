import type { ReactNode, Ref } from "react";
import { Button as AriaButton, type ButtonProps as AriaButtonProps } from "react-aria-components";
import styles from "./Button.module.css";
import { useKeyShortcutsRef } from "./keyShortcuts";

export interface ButtonProps extends Omit<AriaButtonProps, "className" | "children"> {
  variant?: "primary" | "secondary" | "ghost";
  /** Optional leading icon; decorative, the label carries the meaning. */
  icon?: ReactNode;
  children: ReactNode;
  /** Its shortcut in ARIA's syntax ("Meta+Enter"); React Aria drops it, so it is set by ref. */
  "aria-keyshortcuts"?: string;
  /** For moving focus to it (React 19 passes `ref` as a prop). */
  ref?: Ref<HTMLButtonElement>;
}

export function Button({
  variant = "secondary",
  icon,
  children,
  ref,
  "aria-keyshortcuts": keys,
  ...props
}: ButtonProps) {
  const shortcutsRef = useKeyShortcutsRef(keys, ref);
  return (
    <AriaButton {...props} ref={shortcutsRef} className={`${styles.button} ${styles[variant]}`}>
      {icon ? (
        <span className={styles.icon} aria-hidden="true">
          {icon}
        </span>
      ) : null}
      <span>{children}</span>
    </AriaButton>
  );
}
