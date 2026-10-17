import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Reduced motion (S10.6 AC1, EXPERIENCE §7): with `prefers-reduced-motion:
 * reduce`, content appears in its final state — every CSS animation and
 * transition is cut to a frame, and smooth scrolling is off.
 *
 * The app has one global rule for this (`src/styles/index.css`) rather than
 * per-component opt-outs, which is what makes a static assertion meaningful:
 * the universal selector already covers every component, including Tailwind's
 * `animate-*` utilities and the `var(--dur-*)` durations. What a change could
 * still break — and what this test guards — is the rule disappearing, being
 * narrowed to specific selectors, or being overridden with `!important`.
 */

const STYLES = readFileSync(join(process.cwd(), "src/styles/index.css"), "utf8");
const CSS = STYLES.replace(/\/\*[\s\S]*?\*\//g, "");

/** The body of the first `@media (prefers-reduced-motion: reduce)` block. */
function reducedMotionBlock(css: string): string | null {
  const start = css.indexOf("@media (prefers-reduced-motion: reduce)");
  if (start === -1) return null;
  const open = css.indexOf("{", start);
  if (open === -1) return null;
  // Walk the braces so a nested rule does not end the block early.
  let depth = 0;
  for (let i = open; i < css.length; i += 1) {
    if (css[i] === "{") depth += 1;
    else if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) return css.slice(open + 1, i);
    }
  }
  return null;
}

describe("reduced motion (EXPERIENCE §7)", () => {
  const block = reducedMotionBlock(CSS);

  it("ships a prefers-reduced-motion: reduce block", () => {
    expect(block).not.toBeNull();
  });

  it("covers every element and pseudo-element", () => {
    expect(block).toMatch(/(^|\s)\*/);
    expect(block).toContain("::before");
    expect(block).toContain("::after");
  });

  it("cuts animations and transitions to a frame", () => {
    // 0.01ms is the rule the app uses: the final state paints immediately.
    expect(block).toMatch(/animation-duration:\s*0\.0\d+ms/);
    expect(block).toMatch(/animation-iteration-count:\s*1\s*;/);
    expect(block).toMatch(/transition-duration:\s*0\.0\d+ms/);
    expect(block).toMatch(/scroll-behavior:\s*auto/);
  });

  it("is not narrowed by a later override", () => {
    // A !important animation/transition anywhere else would win over the
    // universal rule and bring movement back for the people who asked it off.
    expect(CSS).not.toMatch(/(animation|transition)-[a-z]*\s*:[^;]*!important/);
  });

  it("does not re-enable motion under no-preference", () => {
    expect(CSS).not.toContain("prefers-reduced-motion: no-preference");
  });
});
