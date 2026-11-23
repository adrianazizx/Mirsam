import { act, fireEvent } from "@testing-library/react";

/** What Tab stops on, in document order (no positive tabindex is used in the app). */
const TABBABLE = 'a[href], button, input, select, textarea, [tabindex], [contenteditable="true"]';

function isTabbable(element: HTMLElement): boolean {
  if (element.tabIndex < 0) return false;
  if ((element as HTMLButtonElement).disabled) return false;
  if (element.closest("[inert], [hidden]")) return false;
  for (let node: HTMLElement | null = element; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    if (style.display === "none" || style.visibility === "hidden") return false;
  }
  return true;
}

/** Every element Tab reaches, in order; inside an open modal dialog, only its own. */
export function tabbables(): HTMLElement[] {
  const modal = document.querySelector<HTMLElement>('[aria-modal="true"]');
  const root = modal ?? document.body;
  return [...root.querySelectorAll<HTMLElement>(TABBABLE)].filter(isTabbable);
}

/**
 * Presses Tab (or Shift+Tab) as a browser would: the key goes to the focused
 * element first (React Aria's toolbars and focus scopes act on it), and unless
 * that prevented it, focus moves to the next element in tab order, wrapping.
 */
export function pressTab(shift = false): HTMLElement | null {
  const from = (document.activeElement as HTMLElement | null) ?? document.body;
  let prevented = false;
  act(() => {
    prevented = !fireEvent.keyDown(from, { key: "Tab", code: "Tab", shiftKey: shift });
  });
  if (!prevented) {
    const order = tabbables();
    const current = document.activeElement as HTMLElement | null;
    const index = current ? order.indexOf(current) : -1;
    const next =
      index === -1
        ? shift
          ? order.at(-1)
          : order[0]
        : order[(index + (shift ? -1 : 1) + order.length) % order.length];
    act(() => next?.focus());
    act(() => {
      fireEvent.keyUp(next ?? document.body, { key: "Tab", code: "Tab", shiftKey: shift });
    });
  }
  return document.activeElement as HTMLElement | null;
}

/** Presses a key on the focused element (or the body). */
export function press(key: string, init: KeyboardEventInit = {}): void {
  const target = (document.activeElement as HTMLElement | null) ?? document.body;
  act(() => {
    fireEvent.keyDown(target, { key, ...init });
  });
  act(() => {
    fireEvent.keyUp((document.activeElement as HTMLElement | null) ?? document.body, {
      key,
      ...init,
    });
  });
}

/** The shell area (`data-region`) an element is in. */
export function regionOf(element: Element | null): string | null {
  return element?.closest("[data-region]")?.getAttribute("data-region") ?? null;
}
