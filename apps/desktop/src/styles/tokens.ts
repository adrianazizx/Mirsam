/**
 * A size token's value in CSS px, for code that lays things out itself
 * (virtualized lists, the viewer). `fallback` applies where CSS is not loaded
 * (unit tests).
 */
export function tokenPx(name: `--${string}`, fallback: number): number {
  if (typeof getComputedStyle !== "function") return fallback;
  const value = Number.parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue(name),
  );
  return Number.isFinite(value) && value > 0 ? value : fallback;
}
