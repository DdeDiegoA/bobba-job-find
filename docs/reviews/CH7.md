# CH7 review — core error keys one file per area

Reviewer: reviewer-2 (paired with dev-2). Reviewed commit: `94a9b66` (round 2; round 1 at `0cf1a06`).

## Verdict

APPROVED.

## Scope checked

- Core error keys moved from `locales/{en,es}/common.json` to `locales/{en,es}/errors/<area>.json` (8 areas, 49 keys). Key strings unchanged (`errors.<area>.<x>`); `common.json` is `{}`.
- Runtime: `src/lib/i18n/index.ts` merges fragments into the default `common` namespace; `boba-app/src/i18n/mod.rs` embeds them via `ERROR_FRAGMENTS`.
- Gate: `scripts/check-i18n.mjs` rejects an `errors.*` key left in a plain catalog, a key filed under the wrong area, a fragment missing on one side, and a missing EN/ES fragment file.
- AD-21 and README updated to match.

## Findings and resolution

1. **Medium (round 1, fixed in `94a9b66`):** `ERROR_FRAGMENTS` is hand-listed. A fragment file missing from the list passed both gates and left its keys unresolvable at runtime. Reproduced on `0cf1a06`: a `zzz` area referenced from core code passed `check-i18n` (exit 0) while `translated()` returned no value. Fix: `error_fragment_list_matches_the_locales_directory` (Rust) reads `locales/{en,es}/errors/` and asserts it equals the list. Passes with the real areas; fails with a stray area.

## Probes (reproduced on `0cf1a06`)

- Drop an ES fragment key: fails, naming `locales/es/errors/tracker.json`.
- Leave `errors.tracker.x` in `locales/en/common.json`: fails with "belongs in locales/en/errors/tracker.json".
- Key under the wrong area (`errors.tracker.oops` in `store.json`): fails, "not in this file's area".
- Delete an ES fragment file: fails, "file missing (in en)".

## Gate on `94a9b66`

| Step | Result |
|---|---|
| `cargo metadata --locked` | pass |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --locked -D warnings` | pass |
| `cargo test --workspace --locked --no-fail-fast` | pass, 465 passed, 0 failed |
| `cargo deny check` | pass |
| `pnpm lint` | pass |
| `pnpm typecheck` | pass |
| `pnpm test` (vitest) | pass on a clean run, 109/109 (first run had 6 load-induced 5 s timeouts at load avg ~80) |
| `check-i18n` (CI and local) | pass, 40 namespaces + 8 error areas |
| `check-workflow-policy` | pass |
| `gen_stories --check` | pass |
| `gen-bindings` + bindings diff | pass, no diff |

## Notes (non-blocking)

- AGENTS.md still says `errors.<area>.*` lives in `common.json`. Orquestador must update it (dev-2 flagged this; in-flight slices appending `errors.*` to `common.json` now fail the gate).
- The vitest timeouts were environmental (concurrent cargo builds on the machine); they are not CH7 code.
