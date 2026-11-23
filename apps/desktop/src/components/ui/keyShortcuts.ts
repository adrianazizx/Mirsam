import { type Ref, useCallback } from "react";

/**
 * A ref that writes `aria-keyshortcuts` on the element (React Aria's buttons
 * drop it from their props) and passes the element on to `ref`. `keys` is in
 * ARIA's syntax ("Meta+Shift+Enter"; `ariaShortcut` in the registry). Stable
 * while `keys` and `ref` are, so a re-render does not detach and attach it.
 */
export function useKeyShortcutsRef<T extends HTMLElement>(
  keys: string | null | undefined,
  ref?: Ref<T>,
): (node: T | null) => void {
  return useCallback(
    (node: T | null) => {
      if (node) {
        if (keys) node.setAttribute("aria-keyshortcuts", keys);
        else node.removeAttribute("aria-keyshortcuts");
      }
      if (typeof ref === "function") ref(node);
      else if (ref) ref.current = node;
    },
    [keys, ref],
  );
}
