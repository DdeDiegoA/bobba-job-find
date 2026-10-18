# FX1 review — Settings nav uses absolute links

> **Reviewer:** reviewer-2 (qa) · **Dev:** dev-2 · **Date:** 2026-10-18
> **Branch:** `hermad/FX1` · **Reviewed commit:** `3ecd2d0` (`fix(settings): resolve every nav entry against /settings`)
> **Spec:** FX1 in `beta-fix-plan.md`; root cause issue 3 in `beta1-issues.md`
> **Verdict:** APPROVED

## Scope

- `src/features/settings/index.tsx`: one line, `to={entry.id}` → `` to={`/settings/${entry.id}`} ``.
- `src/features/settings/settings.test.tsx`: new router-level test (2 cases) that mounts the real `settings/*` route.

## Root cause

`NavLink to={entry.id}` is relative. Inside the `settings/*` splat route it resolves against the full current URL, so from `/settings/privacy/audit` the "Scoring" link becomes `/settings/privacy/audit/scoring`, and clicks keep nesting. The absolute path fixes it. The fix addresses the root cause, not the symptom.

## Acceptance criteria

| AC | Result |
|---|---|
| From `/settings/privacy/audit`, clicking Scoring goes to `/settings/scoring` and renders its heading | Covered by test 1 |
| The clicked entry is `aria-current="page"` | Covered by tests 1 and 2 |
| Two different entries in a row each show their own page | Covered by test 2 |
| Router-level test on `settings/*` fails on the old relative `to` | Verified below |

## Reproduction (the test fails on the old code)

Reverted the fix line in the worktree, ran the new test, then restored it with `git checkout`.

- Test 1 fails: `expected '/settings/privacy/audit/scoring' to be '/settings/scoring'`
- Test 2 fails: `expected '/settings/scoring/preferences' to be '/settings/preferences'`

These are the nesting symptoms from the root cause. With the fix, both pass.

## Gate (worktree `hermad/FX1` at `3ecd2d0`, no cargo)

| Check | Result |
|---|---|
| `pnpm lint` | exit 0. 3 warnings: `noExplicitAny` in generated `src/lib/ipc/bindings.ts`. Not touched by this diff. |
| `pnpm typecheck` | exit 0 |
| `pnpm test` | exit 0. 80 files, 604 tests pass. |
| `pnpm vitest run src/features/settings` | 14 files / 56 tests pass before the revert check; new file 2/2 pass after |
| `node scripts/check-i18n.mjs` | exit 0 |
| `node scripts/ci/check-conventions.mjs` | exit 0 |
| `node scripts/ci/check-workflow-policy.mjs` | exit 0 |
| `e2e/a11y` (5 files) | 72/72 pass. The screen's markup changed only in the `href` value. |

## Findings

No blocking findings.

- **nit:** the `Scoring` lookup in the test scans links with `nav().contains(node)`, so it is not confused by the same label elsewhere on the page. Good as written.
- **note:** the fix hard-codes the `/settings` prefix. That matches the `settings/*` route in `src/app/routes.tsx:52`. If the route ever moves, this link moves with it, and the test would catch a break.

## Decision

APPROVED at `3ecd2d0`. The change is minimal, addresses the root cause, and its test reproduces the bug before the fix.
