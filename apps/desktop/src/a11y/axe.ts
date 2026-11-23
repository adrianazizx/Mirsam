import axe from "axe-core";

/**
 * Runs axe-core (MPL-2.0, a dev dependency only) over what is rendered, and
 * returns its violations as readable lines, so a failing test says what and
 * where. The WCAG 2.0/2.1 A and AA rules and axe's best practices run.
 *
 * Colour contrast is off here: jsdom lays nothing out and computes no colours.
 * It is checked twice elsewhere: on the design tokens (`styles/tokens.test.ts`)
 * and on the rendered screens in Chrome (`scripts/a11y-check.mjs`).
 */
export async function axeViolations(root: Element = document.body): Promise<string[]> {
  const results = await axe.run(root, {
    runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"] },
    rules: { "color-contrast": { enabled: false } },
    resultTypes: ["violations"],
  });
  return results.violations.flatMap((violation) =>
    violation.nodes.map(
      (node) =>
        `${violation.id} (${violation.impact ?? "?"}): ${node.target.join(" ")} — ${node.failureSummary?.replace(/\s+/g, " ").trim() ?? violation.help}`,
    ),
  );
}
