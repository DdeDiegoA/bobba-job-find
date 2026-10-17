import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * 200% zoom (S10.6 AC1, NFR-15, EXPERIENCE §7: "Zoom to 200% without loss of
 * content or function").
 *
 * Zoom halves the CSS viewport. The shell replaces itself with a resize notice
 * below 960 CSS px (`src/styles/index.css`), so a 200% zoom on a 1440-px window
 * (720 CSS px) shows the notice instead of the app — and the window itself is
 * allowed down to 940 px, which is already inside that range. This test pins
 * the numbers so the pass has evidence, and pins the defect as `it.fails` for
 * the shell's owner to fix (A11Y-6). Fixing it means either lowering the floor
 * to a width that reflows, or matching the window minimum to it.
 */

const CSS = readFileSync(join(process.cwd(), "src/styles/index.css"), "utf8").replace(
  /\/\*[\s\S]*?\*\//g,
  "",
);
const CONF = JSON.parse(
  readFileSync(join(process.cwd(), "src-tauri/tauri.conf.json"), "utf8"),
) as { app?: { windows?: { minWidth?: number }[] } };

/** The widest viewport that still triggers the resize floor. */
function floorWidth(): number {
  const match = /@media \(max-width: ([\d.]+)px\)\s*\{[\s\S]*?\.shell\s*\{\s*display:\s*none/.exec(
    CSS,
  );
  expect(match, "the shell floor rule exists").not.toBeNull();
  return Number(match?.[1]);
}

const FLOOR = floorWidth();
const MIN_WINDOW = CONF.app?.windows?.[0]?.minWidth ?? 0;

describe("the shell floor and the window minimum", () => {
  it("documents the floor the app ships", () => {
    expect(FLOOR).toBeGreaterThan(0);
    expect(FLOOR).toBeLessThan(1000);
  });

  it("hides the shell at the window's own minimum size", () => {
    // Evidence for A11Y-6: at the smallest window the user may make, the app
    // is already below its floor.
    expect(MIN_WINDOW).toBeGreaterThan(0);
    expect(MIN_WINDOW).toBeLessThanOrEqual(FLOOR);
  });

  it("loses the content at 200% zoom on a 1440-px window", () => {
    // 1440 / 2 = 720 CSS px, below the floor: the notice replaces the app.
    expect(1440 / 2).toBeLessThanOrEqual(FLOOR);
  });

  it.fails("keeps the app usable at 200% zoom (A11Y-6)", () => {
    // Passes only when the floor reflows below 720 px, or when the app opts in
    // to a zoom/scroll model that keeps content visible. Owned by the shell
    // slice (S1.5) with DESIGN.md §10.4 — delete this pin when it lands.
    const zoomed = 1440 / 2;
    expect(zoomed).toBeGreaterThan(FLOOR);
  });
});
