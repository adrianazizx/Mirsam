import type { ReactNode } from "react";
import { Button as AriaButton, type ButtonProps as AriaButtonProps } from "react-aria-components";
import styles from "./Button.module.css";

export interface ButtonProps extends Omit<AriaButtonProps, "className" | "children"> {
  variant?: "primary" | "secondary" | "ghost";
  /** Optional leading icon; decorative, the label carries the meaning. */
  icon?: ReactNode;
  children: ReactNode;
}

export function Button({ variant = "secondary", icon, children, ...props }: ButtonProps) {
  return (
    <AriaButton {...props} className={`${styles.button} ${styles[variant]}`}>
      {icon ? (
        <span className={styles.icon} aria-hidden="true">
          {icon}
        </span>
      ) : null}
      <span>{children}</span>
    </AriaButton>
  );
}
