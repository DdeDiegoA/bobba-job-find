# FX2 review (reviewer-3)

Commit reviewed: `a80cc2e` (branch `hermad/FX2`), on top of `593feda`.

**Verdict: APPROVED.**

## Gate (run in the worktree, web half only; no cargo, disk-limited)

| Check | Result |
|---|---|
| `pnpm install --frozen-lockfile --ignore-scripts` | pass |
| `pnpm lint` | pass |
| `pnpm typecheck` | pass |
| `pnpm test` | pass |
| `node scripts/check-i18n.mjs` | pass |
| `node scripts/ci/check-conventions.mjs` | pass |
| `e2e/a11y` | not run: no screen changed. Only the row index values in the palette changed; no markup, role or label changed. |

## Reproduction of the original bug

Ran `src/app/palette/index.test.tsx` against the old `src/app/palette/index.tsx`
(from `main`), then restored the fix:

| Test | Old code | Fix |
|---|---|---|
| hovers the 2nd row and Enter opens the 2nd row's destination | FAIL | pass |
| clicks the 3rd row and runs the 3rd row, never the last row | FAIL | pass |
| arrows down past the page rows and Enter opens the offer under the cursor | pass | pass |
| the other 6 tests | pass | pass |

The two hover tests reproduce the bug on the old code. The arrow test passes on
both, which is expected: arrow keys never used the broken hover callback. It is a
regression guard for the offer index, not a reproduction of the bug.

## Root cause

The fix addresses the root cause, not the symptom.

- `let index = -1;` in the render body is removed (grep: 0 matches).
- Page rows use `pageRows.map((row, i) => …)` with `index={i}`,
  `selected={i === selected}` and `onHover={() => setActive(i)}`.
- Offer rows use `const index = pageRows.length + i`, so the flat index still
  matches the visual order and `selected`. The `Row` id `palette-row-${index}`
  is unchanged.
- No other `let` mutation in the render path closes over a loop variable.

## Acceptance points checked

- Hover the 2nd row, Enter runs the 2nd row: covered by the new test, and it fails on the old code.
- Click the 3rd row, the 3rd row runs, and the last row does not: covered by the new test, and it fails on the old code.
- Page rows then offer rows, arrow down to an offer, Enter opens it: covered. The `let index` is removed.
- The new test mounts routes for `/inbox`, `/studio` and `/settings`, so the destinations it asserts are real routes.

## Notes (not blocking)

- `N1`: The "clicks" test uses `mouseMove` then `mouseDown`, not a real `click`. It covers the reported path (hover, then press), which is the bug's path. A real `click` event would also fire `mouseDown` in jsdom, so the coverage holds.
- `N2`: The palette's `onChoose` path (`choose(row)` per iteration) was already correct and is unchanged.
