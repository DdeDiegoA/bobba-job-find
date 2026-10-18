# FX5 review (reviewer-3)

Commit reviewed: `e90f24b` (branch `hermad/FX5`), on top of `aa337c9`.

**Verdict: APPROVED** (one non-blocking test gap, see N1).

## Gate (run in the worktree, web half only; no Rust changed, so no cargo)

| Check | Result |
|---|---|
| `pnpm install --frozen-lockfile --ignore-scripts` | pass |
| `pnpm lint` | pass (3 warnings, all `noExplicitAny` in generated `src/lib/ipc/bindings.ts`, not touched by this slice) |
| `pnpm typecheck` | pass |
| `pnpm test` | pass (81 files, 618 tests) |
| `node scripts/check-i18n.mjs` | pass (EN/ES aligned, ICU valid, no unused keys) |
| `node scripts/ci/check-conventions.mjs` | pass |
| `vitest run src/features/onboarding` | pass (5 files, 74 tests) |
| `vitest run e2e/a11y` | pass (5 files, 74 tests) |

## Acceptance points checked

- **Empty address (AC1).** Helper text `email.addressHint` renders only while the address is empty. Outlook and Gmail are disabled, and each button's `aria-describedby` points at the hint's id. Typing an address removes the hint and enables both buttons. Covered by "explains why the one-click buttons are disabled…".
- **No bundled client (AC2).** `errors.email.no_bundled_client` is matched by its `message_key`. It renders as a `role="alert"` block with the body "This build has no one-click sign-in. Skip for now, or add your own client under Connections > Email" and a link to `/connections`. The generic error line is suppressed for this key, so the same message isn't shown twice. Buttons stay enabled. Covered by "turns 'no bundled client' into a next action…".
- **Never dead-ends.** Skip is the wizard's `skip()`, which is allowed for optional steps. The email step is optional, so Skip is never disabled. Next is only blocked for an invalid required step (`wizard/index.tsx:182`), and this step is never invalid. `/connections` is a child of the Shell, outside the first-run gate (`routes.tsx:51`). Only the index route redirects. So the link opens the page. Wizard progress persists in `localStorage` (`wizard/progress.ts`), so returning through Today resumes the wizard.
- **Error copy names a real place (AC3).** The `Advanced` wording is gone from the error catalogues (`errors/email.json`, both locales). A test scans both catalogues for `Advanced|Avanzado` and checks for "Connections > Email" / "Conexiones > Correo".
- **Pending state (AC4).** While a connect is pending, `role="status"` shows "Finish signing in in your browser…" and the Skip/Next wiring is unchanged. Covered by "says what a pending sign-in is waiting for".
- **i18n.** `email.addressHint`, `email.waiting`, `email.noBundledClient.*` exist in en and es (`onboarding_finish.json`), and the two error keys are updated in both locales. `check-i18n` passes.
- **a11y.** Hint is linked by `aria-describedby`, the alert uses `role="alert"`, the pending text uses `role="status"`, and the link has visible text. `e2e/a11y` passes.

## Notes (not blocking)

- `N1` (test gap, AC4): the pending test asserts the copy only. It does not assert that Skip and Next are enabled while a connect is pending. The code keeps them enabled, as read above. A later edit could break this without a test failing. Suggested follow-up: one assertion on the wizard's Skip button in that test.
- `N2` (copy nit): the en label "Connections > Email" doesn't exactly match the page heading "Email connections". The es label "Conexiones > Correo" likewise doesn't match "Conexiones de correo". Both name a real place, so AC3 holds. Consider aligning the wording with the headings in a later pass.
- `N3`: the link leaves the full-window wizard for the Shell. The way back is through Today, which redirects into the wizard and shows the resume banner. This is a path, not a dead end, but it is indirect.
