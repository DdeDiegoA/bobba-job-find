import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * 200% zoom (S10.6 AC1, NFR-15, EXPERIENCE §7: "Zoom to 200% without loss of
 * content or function").
 *
 * Zoom halves the CSS viewport. A11Y-6: the shell used to replace itself with a
 * resize notice below 960 CSS px, so a 200% zoom on a 1440-px window (720 CSS
 * px) and the window's own 940-px minimum both lost the app. It now reflows into
 * a compact layout at every width; these assertions pin that (the physical
 * visual pass stays manual, script D in `docs/qa/accessibility.md`).
 */

const CSS = readFileSync(join(process.cwd(), "src/styles/index.css"), "utf8").replace(
  /\/\*[\s\S]*?\*\//g,
  "",
);
const CONF = JSON.parse(
  readFileSync(join(process.cwd(), "src-tauri/tauri.conf.json"), "utf8"),
) as { app?: { windows?: { minWidth?: number }[] } };

/** The compact-layout rule: its widest width and the rail column it sets. */
function compactRule(): { floor: number; columns: string } {
  const match = /@media \(max-width: ([\d.]+)px\)\s*\{\s*\.shell\s*\{([^}]*)\}/.exec(CSS);
  expect(match, "the compact shell rule exists").not.toBeNull();
  const columns = /grid-template-columns:\s*([^;]+);/.exec(match?.[2] ?? "")?.[1].trim() ?? "";
  return { floor: Number(match?.[1]), columns };
}

/** The compact sidebar rail in CSS px (`3.5rem` → 56). */
function railWidth(columns: string): number {
  const rem = /^([\d.]+)rem/.exec(columns);
  expect(rem, `the rail column is set in rem: ${columns}`).not.toBeNull();
  return Number(rem?.[1]) * 16;
}

const { floor: FLOOR, columns: COLUMNS } = compactRule();
const MIN_WINDOW = CONF.app?.windows?.[0]?.minWidth ?? 0;
const RAIL = railWidth(COLUMNS);
/** 200% zoom on a 1440-px window. */
const ZOOMED = 1440 / 2;

describe("the shell at 200% zoom (A11Y-6)", () => {
  it("never hides the shell at any width", () => {
    // The old defect: `.shell { display: none }` below 960 px.
    expect(CSS).not.toMatch(/\.shell\s*\{[^}]*display:\s*none/);
  });

  it("reflows into a compact layout below the 960-px breakpoint", () => {
    expect(FLOOR).toBeGreaterThan(0);
    expect(FLOOR).toBeLessThan(1000);
    expect(COLUMNS).toContain("minmax(0, 1fr)");
    expect(RAIL).toBeGreaterThan(0);
  });

  it("stays in the compact layout at the window's own minimum size", () => {
    expect(MIN_WINDOW).toBeGreaterThan(0);
    // The window cannot open narrower than the breakpoint: no width the user
    // can reach unzoomed falls outside the reflowing range.
    expect(MIN_WINDOW).toBeLessThanOrEqual(FLOOR);
  });

  it("keeps content and function at 200% zoom on a 1440-px window", () => {
    // 1440 / 2 = 720 CSS px: the compact layout, not a resize notice.
    expect(ZOOMED).toBeLessThanOrEqual(FLOOR);
    // The rail leaves a usable content column even at the window minimum.
    expect(ZOOMED - RAIL).toBeGreaterThanOrEqual(320);
    expect(MIN_WINDOW).toBeGreaterThanOrEqual(940);
    expect(MIN_WINDOW / 2 - RAIL).toBeGreaterThanOrEqual(320);
  });
});
