# FX9 review — re-run the gap interview from the Profile

Reviewer: reviewer-3 · Dev: dev-3 · Reviewed commit: `dd86f87` (branch `hermad/FX9`, on main `e95008b`)

## Verdict

**APPROVED** at `dd86f87`. All three ACs are met and covered by tests. No blocking findings. Three low-severity notes below; none needs a fix round.

## Gate (run in `.hermad/worktrees/FX9`, web half only, no cargo)

| Step | Result |
|---|---|
| `pnpm lint` | pass, 0 errors. 3 warnings, all `noExplicitAny` in `src/lib/ipc/bindings.ts:3234-3245` (generated, pre-existing, not in FX9 files) |
| `pnpm typecheck` | pass |
| `pnpm vitest run src/features/profile` | pass, 2 files, 33/33 (includes `gaps-entry.test.tsx` 10 tests) |
| `node scripts/check-i18n.mjs` | pass (43 namespaces, EN/ES aligned, ICU valid, no unused keys) |
| `node scripts/ci/check-conventions.mjs` | pass |

Not re-run by me: the full `pnpm test` (741 tests as the dev reports) and `e2e/a11y`. The dev reports both green; I did not reproduce them.

## AC check

- **AC1 — run and review.** `Review gaps` calls `commands.profileGapsStart(null, language)`. Questions render from the run's `text_en`/`text_es` as React text, so they stay verbatim. Proposals render through the shared `ProposalReviewCard` with `drafts.untrusted` labelled. Covered by tests at lines 102 and 112.
- **AC2 — per-item save only.** Answer calls `profileGapsAnswer(key, answer, language)` with one key. Confirm calls `profileGapsConfirm(id, null)` for one id. Discard calls `profileGapsDiscard(id)`. The others stay pending. `profileSaveFact` is never called on any path. Covered by tests at lines 138, 163 and 182. Saving an answer is the user's explicit click on that question's Save button, so nothing persists without confirmation.
- **AC3 — Limited mode.** `useCapability("profile_gaps")` drives `disabled` on the button. The no-engine copy comes from `gated` (`limitedTitle` / `unsupportedTitle`) and links to `/settings/ai-engine`. Covered by tests at lines 198 and 207.
- **i18n.** All 8 `review.*` keys exist in EN and ES. `check-i18n` passes.
- **a11y.** Questions sit in a labelled `<ul>`, each input has a `<label htmlFor>`, the Limited notice is `role="status"`, errors are `role="alert"`, and the run announces via `role="status"`. The button uses the shared `Button`, so it is keyboard-operable.

## Notes (low, non-blocking)

- **L-1 (copy).** `review.empty` ("Nothing left to ask — your profile covers every Gate.") shows whenever `asked && questions = 0 && proposals = 0`. That also covers the case where the user just discarded or confirmed every proposal, or answered every question. The copy claims more than the state knows. Consider "Nothing left to review from this run."
- **L-2 (unverified).** A new run replaces the visible question list with `outcome.questions`. Questions from an earlier run that the user has not answered disappear from the UI. I did not check whether the core still treats them as pending. If it does, they are hidden but not lost; if not, they are only shown once.
- **L-3 (unverified).** Eligibility, notice and writing-sample Save have no client-side required check. An empty region or text is sent to the core. I did not verify the core's rejection path, so the error text and the `fail` path are not exercised for that case.

## Commit

Review doc only. Code is unchanged in this commit.
