# FX7 review: switch AI engine in Settings (shared EnginePicker)

> **Reviewer:** reviewer-1 (qa) · **Dev:** dev-1 · **Date:** 2026-10-18
> **Branch:** `hermad/FX7` · **Reviewed commit:** `df04ee3` (`feat(settings): switch AI engine with the shared EnginePicker`), on main `e32544f`
> **Verdict:** CHANGES REQUESTED at `df04ee3` (1 bug, 1 gap)

## Gate (web half, run in the worktree)

| Command | Result |
|---|---|
| `pnpm install --frozen-lockfile --ignore-scripts` | ok |
| `pnpm lint` | ok |
| `pnpm typecheck` | ok |
| `pnpm test` | 87 files, 708 tests passed |
| `node scripts/check-i18n.mjs` | ok (en/es parity) |
| `node scripts/ci/check-conventions.mjs` | ok |
| `node scripts/gen-bindings.mjs --check` | ok (no command added, bindings unchanged) |
| `pnpm exec vitest run e2e/a11y` | 5 files, 76 tests passed |

No cargo run, per the orquestador's instruction. The commit touches no Rust.

## Acceptance criteria

| AC | Evidence | Result |
|---|---|---|
| AC1 Settings shows installed CLIs, model field, Test, Save | `settings/ai-engine/index.test.tsx:193` | pass |
| AC2 Save calls `agentEngineChoose(cli, model)` only after a pass, refreshes snapshot and usage | `index.test.tsx:204` (asserts no choose before Save, `toHaveBeenCalledWith("claude_code","sonnet")`, usage button appears) | pass |
| AC3 Failed test disables Save, translated error, choose never called | `index.test.tsx:233` | pass |
| AC4 Limited mode is left after Save | `index.test.tsx:248` (`LimitedProbe` goes `limited` → `ready`) | pass |
| AC5 Onboarding uses shared picker, existing tests pass | onboarding engine tests in the 708-test run; the picker is the only engine UI in onboarding | pass |

Limited mode: the Save path invalidates `["capabilities","snapshot"]`, the key `src/lib/capabilities/index.ts` uses. Verified by grep.

No LLM output is saved here. The test-call reply is shown only and never written, so the explicit-confirm rule is not touched.

## Findings

### 1. BUG (medium): a test that resolves after a selection change is attributed to the new selection

- **Where:** `src/features/engine-picker/index.tsx` — `change()` (line 103) clears `result`, but `test.onSuccess` (lines 115–118) calls `setResult(tested)` and `onTested?.(tested)` unconditionally, even when the CLI or model changed while the test was in flight.
- **Repro (by reading the code; not reproduced in a test):**
  1. Settings or onboarding: pick Claude Code, type `sonnet`, click Test.
  2. While the test is still running, switch the radio to Codex (or edit the model).
  3. The test resolves as ok for `claude_code/sonnet`.
  4. Expected: the stale pass is discarded. Save stays disabled for Codex until Codex is tested.
  5. Actual: `result` holds the Claude Code pass. The Codex radio is shown with "Answered … as sonnet", and Save is enabled. Save calls `agentEngineChoose("claude_code","sonnet")`, which is not the engine the user sees selected.
- **Onboarding is worse:** `src/features/onboarding/steps/engine/index.tsx:63–70` — `onTested` sets `answers.cli`/`answers.model` to the stale pair and calls `choose.mutate(result)`, so the stored engine changes to the stale CLI while the radio still shows the new one.
- **Fix to try:** tag each test with the pick it was started for (for example a ref or the mutation variables) and ignore `onSuccess`/`onError` when the current selection differs. Alternatively, disable the radios and model input while `test.isPending`.
- **Regression test:** hold `agentTestCall` open with a deferred promise, change the CLI, resolve the test, and assert Save stays disabled and `agentEngineChoose` is not called. Add the same case for onboarding's auto-store.

### 2. GAP (medium): the Settings AI engine screen has no a11y audit

- `e2e/a11y/screens.test.tsx` has no case for `SettingsAiEngine` (grep for `ai-engine`, `AiEngine` and `engine` finds nothing). AGENTS.md requires every screen to pass `e2e/a11y`. This screen now holds the new radio group, model input, Test and Save.
- **Fix:** add an `audit(<SettingsAiEngine />, "/settings/ai-engine", …)` case that mocks the engine snapshot with at least one installed CLI and one missing CLI, then asserts zero axe violations.
- The pre-existing screen was not audited either, so this predates FX7. The new picker raises the stakes, so it should land here.

## Observations (not blocking)

- `agent_engine_choose` (`src-tauri/src/commands/agent.rs:516`) stores any CLI and model pair. The "never without a passing test" rule is enforced only by the UI. This was true before FX7 (onboarding did the same). It is noted for the record and is not a regression.
- The picker's copy comes from the `onboarding_engine` namespace inside Settings. It works, and the strings read correctly in both locales. Moving them to a shared namespace is a possible cleanup, not a requirement.
- Coverage gap: there is no direct unit test for the picker's test-result attribution (finding 1), because the picker is only exercised through its two hosts.

## Next

- dev-1 fixes findings 1 and 2 on `hermad/FX7`, with a regression test for each, and re-runs the web gate.
- Re-review at the new commit. Approval goes to the orquestador as `APPROVED FX7 commit <sha>`.

## Re-review at b20d14f

**Verdict:** APPROVED at `b20d14f` (fixes `5f270fe`, `b20d14f`).

- **Finding 1 (stale result):** fixed. `engine-picker` tags each test with the epoch it started in and drops both the pass and the failure once the CLI or model changed. Onboarding no longer records or auto-stores the stale pair. Regression: `engine.test.tsx` "ignores a test result that lands after the model changed". The same epoch guard covers Settings, since it shares the picker; there is no separate Settings test for it.
- **Finding 2 (a11y gap):** fixed. `e2e/a11y/screens.test.tsx` audits Settings > AI engine (tree and axe) and pins the picker labels.

Web gate at `b20d14f`: lint, typecheck, `pnpm test` (87 files, 711 tests), check-i18n, check-conventions, gen-bindings `--check`, `e2e/a11y` (78 tests). All exit 0.
