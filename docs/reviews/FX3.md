# FX3 review (reviewer-3)

Commit reviewed: `7765df8` (branch `hermad/FX3`), on top of `ddd3e89`.

**Verdict: APPROVED.**

## Gate (run in the worktree, web half only; no Rust changed, so no cargo)

| Check | Result |
|---|---|
| `pnpm install --frozen-lockfile --ignore-scripts` | pass |
| `pnpm lint` | pass (3 warnings, all `noExplicitAny` in generated `src/lib/ipc/bindings.ts`, not touched by this slice) |
| `pnpm typecheck` | pass |
| `pnpm test` | pass (80 files, 612 tests) |
| `node scripts/check-i18n.mjs` | pass (EN/ES aligned) |
| `node scripts/ci/check-conventions.mjs` | pass |
| `vitest run e2e/a11y` | pass (5 files, 74 tests; includes `/connections` in the screen audit) |

## Reproduction of the placeholder failure

Ran `src/features/connections/index.test.tsx` against the placeholder from `ddd3e89`
(`index.tsx` swapped in, then restored from git):

| Test | Placeholder | Fix |
|---|---|---|
| mounts the Sources and Email leaves under exactly one h1 | FAIL | pass |
| opens the session opt-in for a session-board source with its id and name | FAIL | pass |
| shows a section empty state instead of a blank page | FAIL | pass |
| shows the existing locked state and does not crash | FAIL | pass |
| renders in Limited mode, with no AI engine connected | FAIL | pass |

All five fail on the placeholder, so the tests catch the bug.

## Acceptance points checked

- **Sources and Email render under the route.** `src/app/routes.tsx:51` mounts `@/features/connections`, which is the new container. Both leaves have their own h2 heading.
- **Exactly one h1.** The container's `h1` is the page title. `ConnectionsSources` and `ConnectionsEmail` moved from `h1` to `h2`. `SessionOptin` renders no heading of level 1 (checked by reading the component).
- **Session opt-in.** One collapsed card per session-board Source. The button names the source (`session.toggle`), and the card gets `sourceId` and `sourceName` as props. `data-source` appears once (`session-optin/index.tsx:100`). The card does not duplicate Sources.
- **Empty states.** Sources, Email and session sections each show their own empty state (test covers all three).
- **Locked store.** The shared locked message shows, and the h1 still renders. No crash.
- **Limited mode.** Sources and Email render without an engine. The container doesn't call the AI capabilities, so this is correct, though the test only proves that the page renders when `capabilitiesSnapshot` returns `limited: true`.
- **i18n.** `title`, `subtitle` and `session.*` keys exist in en and es. `check-i18n` passes.
- **a11y.** `/connections` is in the `e2e/a11y` screen audit, and it passes.

## Notes (not blocking)

- `N1`: The session opt-in opens behind a disclosure button, not on row click. This matches the spec's "one collapsed card per session-board Source" and the AC "open its row".
- `N2`: The Limited-mode test checks rendering only, not capability gating. The leaves are not AI features, so no gating is needed here.
- `N3`: No Rust changed, so the cargo gate was not run, as the orquestador asked.
