# CH8 review (reviewer-4)

Reviewed commit: `96d9b8a` on `hermad/CH8` (base `0e3cf9e`). Scope: output-retry window, InvalidOutput kill, untrusted-run tool_use gate (ADR-010 §1), e2e fake-cli.

## Verdict
APPROVED. Security-relevant chore; no new advisory ignore, no `deny.toml` change.

## Findings and resolution
1. **BLOCKER (fixed, 96d9b8a):** `output_failed` was checked before the write's schema check and handler call, not atomically. Two connections could let a valid write reach the handler after another connection's third refusal failed the run. Fixed: write tools hold the `outputs` lock across check, count and `handler.call`; `output_failed` is re-checked under the lock. Test `a_write_cannot_be_served_after_the_run_is_failed` uses a blocking handler to force the interleave.
2. **LOW (fixed, 96d9b8a):** `server_tool_use` blocks bypassed the untrusted-run gate. Now a tool call. Tests: `a_server_tool_use_block_is_a_tool_call_too`, `a_server_tool_use_after_a_clean_init_aborts_an_untrusted_run`.
3. Extra scope (orquestador): a non-`mcp__boba__*` tool_use after a clean init aborts an untrusted run as `Unsupported`, before emission. Test `a_native_tool_use_after_a_clean_init_aborts_an_untrusted_run` passes.

## Checked
- `call_tool`: after `output_failed` every `tools/call` (writes and reads) returns `CODE_UNAUTHORIZED`; no write reaches the handler after the flag (`once_the_output_is_refused_every_further_call_is_refused`).
- Runtime kill path: `a_run_whose_output_stayed_invalid_is_killed_as_invalid_output`: `InvalidOutput`, invalid writes never stored, no `Finished`.
- `lockdown::init_event_is_locked_down` unchanged; the new check reuses it.
- `fake-cli` `mcp-run` mode: test-only binary, depends on `boba-mcp-proto` via path; `Cargo.lock` change is exactly that one dependency line.

## Gate (on 96d9b8a)
| Step | Result |
|---|---|
| cargo metadata --locked | pass |
| cargo fmt --all --check | pass |
| cargo clippy --workspace --all-targets --locked -D warnings | pass |
| cargo test --workspace --locked | pass, 471 passed, 0 failed |
| cargo deny check | pass (advisories, bans, licenses, sources ok) |
| pnpm install --frozen-lockfile | pass |
| pnpm lint / typecheck / test | pass |
| check-i18n, check-workflow-policy, gen_stories --check | pass |
| gen-bindings --check | pass (no diff) |

## Notes for the orquestador
- `hermad/CH8` is behind `main` (main now includes S4.1h, `711b6e2`). Files do not overlap (`boba-sources/src/http/mod.rs`, `docs/reviews/S4.1h.md` vs CH8 files), so the merge should be clean, but rebase and re-run `cargo test --workspace --locked` on the merge result before merging.
- Writes are now serialized per run (lock held across handler). Acceptable: writes are a few per run.
