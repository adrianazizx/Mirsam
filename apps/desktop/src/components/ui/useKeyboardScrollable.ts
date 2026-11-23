import { type RefObject, useEffect } from "react";

/**
 * A region that scrolls must be scrollable from the keyboard (WCAG 2.1.1).
 * While its content overflows, the region itself joins the tab order, so arrow
 * keys and Page Up/Down scroll it: text between or after its controls (the
 * inspector's document details, a signature's report) is otherwise out of a
 * keyboard user's reach. When everything fits it leaves the tab order again.
 */
export function useKeyboardScrollable(ref: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const update = () => {
      const overflows =
        element.scrollHeight > element.clientHeight + 1 ||
        element.scrollWidth > element.clientWidth + 1;
      if (overflows) {
        if (element.tabIndex !== 0) element.tabIndex = 0;
      } else if (element.hasAttribute("tabindex") && document.activeElement !== element) {
        element.removeAttribute("tabindex");
      }
    };
    update();
    const resize = new ResizeObserver(update);
    resize.observe(element);
    const mutation = new MutationObserver(() => {
      // New content may be taller: measure its children too.
      for (const child of element.children) resize.observe(child);
      update();
    });
    mutation.observe(element, { childList: true, subtree: true });
    for (const child of element.children) resize.observe(child);
    return () => {
      resize.disconnect();
      mutation.disconnect();
    };
  }, [ref]);
}
