# Review: CH6 (flaky-test chore: run-lifecycle stderr race and mock keychain)

- Reviewer: reviewer-1 (paired with dev-1)
- Branch: `hermad/CH6`, reviewed at `29c13d2` (rounds: `2aa080a` → `29c13d2`), rebased on `main` `863a8e5`
- Security gate: no. Test-only chore, plus one production fix in `boba-agent` supervisor (see below). The keychain change touches S8.1 test isolation only.
- Verdict: **APPROVED**

## Scope

| File | Change |
|---|---|
| `boba-agent/src/supervisor/mod.rs` | Production: `reap()` now joins the stderr reader, bounded by `STDERR_JOIN_GRACE` (500 ms). Test: `a_grandchild_holding_stderr_does_not_hang_wait` |
| `boba-agent/tests/run_lifecycle.rs` | Regression test `auth_error_is_classified_when_stderr_closes_late` |
| `tools/fake-cli/src/main.rs` | `BOBA_FAKE_STDERR_HOLD_MS`: a grandchild that holds only stderr open |
| `boba-store/src/keys/mod.rs` | `install_platform_store()` installs the shared mock in `cfg(test)`. `platform_store` is compiled out of test builds. Test: `a_unit_test_build_never_opens_a_platform_store` |
| `boba-store/src/repos/audit.rs` | Audit test helper delegates to the shared mock |

## Production fix (supervisor)

- **Race:** `reap()` used to leave the stderr reader running, so an auth-expired run could be classified from an empty stderr and come back `Unsupported` for an untrusted task. Fixed by joining the stderr reader before classification.
- **Hang risk:** a grandchild that inherits stderr and outlives the CLI could block `reap()` forever. The stderr join is now bounded: poll `is_finished()` to a 500 ms deadline, then detach. The grandchild test proves `wait` returns inside 2 s while a grandchild holds stderr for 5 s.
- **Regression proof:** `auth_error_is_classified_when_stderr_closes_late` fails without the join (`left: Unsupported`, reported by dev-1 3/3). With the grace neutralised it still fails, so the test still bites. Reproduced by dev-1; not re-run by me against the pre-fix commit.
- **Pre-existing, not changed:** the stdout reader join in `reap()` is still unbounded. Out of CH6 scope.

## Keychain isolation

- **Before:** unit tests could reach `install_platform_store` → `platform_store` and open the real macOS keychain. The replaced-mock race was a symptom.
- **Now:** in a `cfg(test)` build, `install_platform_store()` installs `use_mock_keychain()` and `platform_store` is compiled out. The guard test asserts the default store is the shared mock.
- **Integration binary:** `boba-store/tests/db.rs` links the library without `cfg(test)`, so it takes the real path, but each test that touches the keychain installs its own mock first. I checked all three `Store::open` callers in that file (lines 72, 110, 120): each installs the mock first. The crash-child helper does not open the store.

## Residuals

1. `cfg(test)` applies per crate. A test binary outside `boba-store` (for example `src-tauri` or `boba-agent`) still compiles the platform path. Today no test in the workspace calls `Store::open` except the three in `tests/db.rs`, so there is nothing to reach. Check again if a test starts calling `Store::open`.
2. The stdout join in `reap()` stays unbounded (pre-existing).
3. dev-1 flagged a `pnpm test` 5 s timeout under load (settings-preferences, board, profile tests) when a cargo build runs in parallel. Out of CH6 scope; logged for the orquestador.

## CI gate at 29c13d2

Run in the worktree:

| Step | Result |
|---|---|
| cargo metadata --locked | pass |
| cargo fmt --all --check | pass |
| cargo clippy --workspace --all-targets --locked -D warnings | pass |
| cargo test --workspace --locked --no-fail-fast (run 1) | pass (all blocks 0 failed) |
| cargo test --workspace --locked --no-fail-fast (runs 2 and 3) | pass, exit 0 each |
| cargo deny check | pass |
| pnpm install --frozen-lockfile / lint / typecheck / test | pass |
| check-i18n | pass |
| check-workflow-policy | pass |
| gen-bindings --check | pass |

Three consecutive green full-workspace runs, as the CH6 spec requires.
