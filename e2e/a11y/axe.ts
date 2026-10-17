import axe from "axe-core";
import type { Violation } from "./audit";

/**
 * axe-core pass over a rendered screen (S10.6).
 *
 * It complements [`audit.ts`]: the hand-written rules state the floor in terms
 * this repo controls (and run in any environment), while axe brings the wider
 * WCAG 2.2 A/AA rule set — ARIA validity, roles, list structure, table markup,
 * duplicates and so on.
 *
 * `color-contrast` is disabled: jsdom has no layout or canvas, so axe cannot
 * compute painted colours and would report every text node as "needs review".
 * Contrast is covered from the shipped tokens in `contrast.test.ts`.
 */
export async function axeAudit(container: HTMLElement): Promise<Violation[]> {
  const results = await axe.run(container, {
    runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"] },
    rules: { "color-contrast": { enabled: false } },
  });

  return results.violations.flatMap((violation) =>
    violation.nodes.map((node) => ({
      rule: violation.id,
      element: node.target.join(" "),
      detail: `${violation.help}${node.failureSummary ? ` — ${node.failureSummary}` : ""}`,
    })),
  );
}
