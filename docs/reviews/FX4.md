# FX4 review — Run now + Kill Switch: real handler, real run state, visible errors

> **Reviewer:** reviewer-2 (qa) · **Dev:** dev-2 · **Date:** 2026-10-18
> **Branch:** `hermad/FX4` · **Reviewed commit:** `7854501` (`fix(shell): wire Run now to the run lifecycle and always show the Kill Switch`)
> **Spec:** FX4 in `beta-fix-plan.md`; root cause issue 6 in `beta1-issues.md`
> **Verdict:** CHANGES REQUESTED (1 bug, medium)

## Scope

Frontend only. No Rust changed, so no cargo gate. Files: `src/app/shell/use-run-now.ts` (new), `topbar.tsx`, `index.tsx`, `sidebar.tsx`, `nav.ts` (`RunState` and `IDLE_RUN_STATE` removed), `shell.test.tsx`.

## Acceptance criteria

| AC | Result |
|---|---|
| Run now calls `jobs_run_now` once and shows a running state | Covered: "calls jobs_run_now once and shows the running state (AC1)" |
| A live run gives `joined`, the UI says it is already running, no second run | Covered: AC2 test. Core: `scheduler::run_now` returns `joined: true` for `Joined`/`Busy` |
| Locked or recovery-backup error shows a translated inline alert | Locked covered (AC3 test, `role="alert"`). Recovery-backup not tested (see Low 1) |
| Run lifecycle events update the button and the last-run text; no `IDLE_RUN_STATE` | Started/finished covered by AC4 test. `IDLE_RUN_STATE` grep: none left. **Race, see Bug 1** |
| `<KillSwitch />` always rendered, press calls `kill_switch_set` | Render covered (AC5). Press → `killSwitchSet(true/false)` covered by the existing `src/app/header/kill-switch/index.test.tsx` (tests 1 and 3) |
| Sidebar chip and topbar reflect real run state | Covered by the AC1 test (`Running…` in the sidebar) |

## Bug 1 (medium): button stuck on "Running…" when `finished` arrives before the command response

**Where:** `src/app/shell/use-run-now.ts`, `runNow` → `.then((outcome) => { ... setRun({ running: true }) })`.

**What happens:** the core's `run_now` spawns the run and returns `{ run_id, joined: false }`. If the run emits `run_report:progressed` `finished` before the JS `.then` runs, the listener sets `running: false`, and then the `.then` sets `running: true`. Nothing sets it back, so the button is disabled with "Running…" until the next run's events arrive (a scheduled run or an app restart). A run with zero sources, or one that fails fast, is the realistic trigger.

**Repro (temporary probe, deleted after the run):** mock `jobsRunNow` to fire `started` then `finished` through the captured listener, then resolve `{ run_id: "run-1", joined: false }`. Click Run now, wait 50 ms.
- Actual: buttons `System | en | Search or run a command | Running… | Kill Switch`; `expected <button> to be null` (the "Running…" button is still there).
- Expected: the button reads "Run now" and is enabled.

**Expected fix (dev's call):** the events are authoritative, so do not let the command response overwrite them. For example, keep a ref of run ids that have already reported `finished` and skip `running: true` for those; or set `running` only from `started`/`source` events and treat the command response as a no-op when the id is already finished. Add a test: fire `finished` inside the `jobsRunNow` mock before it resolves, assert "Run now" is enabled.

## Low

1. **Recovery-backup path not tested.** AC3 names two codes. Only `locked` has a test. Add a case for `errors.backup.recovery_required` (mock the error code, assert the `role="alert"` text).
2. **Last-run errors are swallowed.** `refresh()` ends in `.catch(() => {})`. If `run_report_latest` fails (for example the store is locked), the topbar shows "No runs yet" with no message. The spec says nothing silently fails. Surface the error through `setMessage`, or at least not show "No runs yet" on failure.
3. **Mount during a live run.** `running` starts `false`. A run started from the tray before the window opened does not show as running until its next event. The button is not silent: clicking it gives "A run is already in progress", so the gap is display-only. Note for the record; no change required in this slice.

## Gate (worktree `hermad/FX4` at `7854501`, web half only)

| Check | Result |
|---|---|
| `pnpm install --frozen-lockfile --ignore-scripts` | exit 0 |
| `pnpm lint` | exit 0. 3 warnings `noExplicitAny` in generated `src/lib/ipc/bindings.ts`, not in this diff |
| `pnpm typecheck` | exit 0 |
| `pnpm test` | exit 0. 80 files, 612 tests passed |
| `node scripts/check-i18n.mjs` | exit 0 |
| `node scripts/ci/check-conventions.mjs` | exit 0 |
| `node scripts/ci/check-workflow-policy.mjs` | exit 0 |
| `node scripts/gen-bindings.mjs --check` | exit 0 (no command changed) |
| Cargo | not run (no Rust changed) |

Probe file `src/app/shell/race-probe.test.tsx` was created for the repro and removed. `git status` is clean apart from this review doc.

## Verified by reading

- `errors.unknown` (fallback in `use-run-now.ts`) exists in `locales/en/errors/unknown.json`.
- `run_report:progressed` phases are `started` | `source` | `finished` (`src-tauri/src/commands/daily_search.rs:141`); the listener treats any non-`finished` phase as running, which is correct.
- The Kill Switch no longer depends on `autoEnabled`. Its own query and `killSwitchChanged` listener keep it live.

## Next

Dev-2 fixes Bug 1 with a test, optionally takes Low 1 and 2, re-runs the gate, and re-commits. Re-review then runs on the new commit.
