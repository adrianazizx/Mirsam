/**
 * A size token's value in CSS px, for code that lays things out itself
 * (virtualized lists, the viewer). Tokens may be `calc()` expressions (most
 * scale with the interface size), which a custom property's computed value
 * leaves unworked; a probe element's width resolves them. `scope` is where the
 * token is read (an element that overrides it), the document by default.
 * `fallback` applies where CSS is not loaded (unit tests).
 */
export function tokenPx(name: `--${string}`, fallback: number, scope?: HTMLElement): number {
  if (typeof getComputedStyle !== "function" || typeof document === "undefined") return fallback;
  const host = scope ?? document.body ?? document.documentElement;
  const probe = document.createElement("div");
  probe.style.position = "absolute";
  probe.style.visibility = "hidden";
  probe.style.width = `var(${name})`;
  host.append(probe);
  const value = Number.parseFloat(getComputedStyle(probe).width);
  probe.remove();
  return Number.isFinite(value) && value > 0 ? value : fallback;
}
