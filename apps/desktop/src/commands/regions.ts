/**
 * The window's areas, in the order the eye meets them (and mirrored with the
 * layout in Arabic): F6 and Shift+F6 move between them, as in most desktop
 * apps. Tab keeps the document order, which puts the page area right after the
 * tabs; F6 reaches the tools, the side panel and the inspector directly.
 *
 * Each area is marked `data-region` in the shell's components.
 */
export const REGIONS = ["tabs", "tools", "panel", "document", "inspector", "status"] as const;
export type Region = (typeof REGIONS)[number];

const TABBABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Where focus lands in an area: the element marked for it, else the one that
 * shows the area's state (the selected tab, the active tool, the current
 * thumbnail), else its first control, else the area itself.
 */
function focusIn(region: HTMLElement): void {
  const target =
    region.querySelector<HTMLElement>("[data-region-focus]") ??
    region.querySelector<HTMLElement>(
      '[role="tab"][aria-selected="true"], [role="radio"][aria-checked="true"][tabindex="0"], [aria-current="page"][tabindex="0"]',
    ) ??
    region.querySelector<HTMLElement>(TABBABLE);
  if (target) {
    target.focus();
    return;
  }
  // Nothing to operate (the inspector with no document): screen readers still land there.
  if (!region.hasAttribute("tabindex")) region.tabIndex = -1;
  region.focus();
}

/** Moves focus to the next (1) or previous (-1) area; false when a dialog holds focus. */
export function moveToRegion(step: 1 | -1): boolean {
  if (document.querySelector('[aria-modal="true"]')) return false;
  const present = REGIONS.flatMap((name) => {
    const element = document.querySelector<HTMLElement>(`[data-region="${name}"]`);
    return element ? [element] : [];
  });
  if (present.length === 0) return false;
  const current = present.findIndex((element) => element.contains(document.activeElement));
  const index =
    current === -1
      ? step === 1
        ? 0
        : present.length - 1
      : (current + step + present.length) % present.length;
  const region = present[index];
  if (!region) return false;
  focusIn(region);
  return true;
}
