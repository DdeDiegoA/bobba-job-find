import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * WCAG 2.2 AA contrast, re-derived from the shipped tokens (S10.6 AC1, NFR-15,
 * DESIGN.md §2.4).
 *
 * DESIGN.md says every pair is checked by `assets/palette_check.py`, but that
 * script audits the design mock, not the build. This test reads the tokens the
 * app actually ships, resolves the `var()` chains per theme, and asserts the
 * same ratios — so a token edit that drifts below AA fails CI instead of
 * shipping.
 *
 * Covered: every normal-size text token, including `--fg-subtle` after A11Y-4
 * re-tuned it to clearer warm values (light `#756249`, dark `#A6907F`) so the
 * text it paints meets AA on every background it lands on — including the
 * selected-option `--primary-soft` in onboarding.
 *
 * NOT covered here (see `docs/qa/accessibility.md`):
 * - Colour *usage* (band dots, icon-only meaning): a contrast test cannot see
 *   whether a colour is the only carrier; the screen audits and the manual pass
 *   cover it.
 */

const CSS = readFileSync(join(process.cwd(), "src/styles/tokens.css"), "utf8");

/** One `{ ... }` block's custom properties. Comments are stripped first. */
function block(selector: string): Record<string, string> {
  const css = CSS.replace(/\/\*[\s\S]*?\*\//g, "");
  const start = css.indexOf(`\n${selector} {`);
  expect(start, `selector ${selector} exists in tokens.css`).toBeGreaterThan(-1);
  const open = css.indexOf("{", start);
  const close = css.indexOf("}", open);
  const out: Record<string, string> = {};
  for (const line of css.slice(open + 1, close).split("\n")) {
    const match = /^\s*(--[\w-]+)\s*:\s*([^;]+);/.exec(line);
    if (match) out[match[1]] = match[2].trim();
  }
  return out;
}

/** Resolve `var(--x)` chains for one theme. */
function resolve(theme: Record<string, string>, name: string, seen = 0): string {
  const raw = theme[name];
  if (raw === undefined) throw new Error(`token ${name} is not defined`);
  const viaVar = /^var\((--[\w-]+)\)$/.exec(raw);
  if (!viaVar) return raw;
  if (seen > 10) throw new Error(`token ${name} is circular`);
  return resolve(theme, viaVar[1], seen + 1);
}

function luminance(hex: string): number {
  const clean = hex.trim().replace("#", "");
  expect(clean, `${hex} is a 6-digit hex`).toMatch(/^[0-9a-fA-F]{6}$/);
  const channel = (offset: number) => {
    const value = Number.parseInt(clean.slice(offset, offset + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

function contrast(theme: Record<string, string>, fg: string, bg: string): number {
  const a = luminance(resolve(theme, fg));
  const b = luminance(resolve(theme, bg));
  const [high, low] = a > b ? [a, b] : [b, a];
  return (high + 0.05) / (low + 0.05);
}

const LIGHT = block(":root");
const DARK = { ...LIGHT, ...block(".dark") };

/** Pairs the app paints normal-size text with, and the AA floor for each. */
const PAIRS: [fg: string, bg: string][] = [
  ["--fg", "--bg"],
  ["--fg", "--surface"],
  ["--fg-muted", "--bg"],
  ["--fg-muted", "--surface"],
  ["--fg-muted", "--surface-muted"],
  ["--fg-subtle", "--bg"],
  ["--fg-subtle", "--surface"],
  ["--fg-subtle", "--surface-muted"],
  ["--fg-subtle", "--surface-sunken"],
  ["--fg-subtle", "--primary-soft"],
  ["--on-primary", "--primary"],
  ["--on-primary-soft", "--primary-soft"],
  ["--danger", "--bg"],
  ["--danger", "--surface"],
  ["--success", "--surface"],
  ["--warning", "--surface"],
  ["--info", "--surface"],
  ["--primary", "--bg"],
];

describe.each([
  ["light", LIGHT],
  ["dark", DARK],
] as const)("%s theme — WCAG AA 4.5:1 (NFR-15)", (_name, theme) => {
  for (const [fg, bg] of PAIRS) {
    it(`${fg} on ${bg}`, () => {
      const ratio = contrast(theme, fg, bg);
      expect(ratio, `${fg} on ${bg} = ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5);
    });
  }
});
