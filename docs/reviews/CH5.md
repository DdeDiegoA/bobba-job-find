# Review — CH5 agent runtime and sandbox wiring (round 1)

> **Reviewer:** reviewer-2 (qa) · **Date:** 2026-10-16 · **Branch:** `hermad/CH5` @ `8067422` (base `main` 5c93535) · **Security-gated (Diego).**
> **Verdict: CHANGES REQUIRED — 1 blocker (P1).** Full CI gate not run yet; it runs after the fix.

## Findings

### P1 (BLOCKER) — live Claude/Gemini runs on untrusted input have no tool lockdown

- **Evidence:** `wiring/agent.rs` `command_builder` gives `claude_code` and `gemini_cli` a real argv (`--mcp-config <mcp.json> <prompt>`) and `agent_runtime` mounts the runtime as `Arc<dyn AgentPort>`. Nothing checks `task.kind.reads_untrusted()` (boba-app `ports/agent.rs:51`). `boba-agent/src/lockdown/mod.rs` is still a one-line stub (S2.3b). So a run for an untrusted task (`NarrateScores`, `TailorApplication`, `ExtractPostings`, …) starts the CLI with its native shell, file-write and web tools enabled.
- **Why it matters:** PRD/AGENTS hard gate: "untrusted-content tasks only on CLIs with verified tool lockdown; no override." The MCP config only adds tools; it does not remove native ones. Today no caller invokes `AgentPort::run`, so this is latent, but CH5 makes it live through the mounted port.
- **Also:** if the `boba-mcp` sidecar is missing, `resolve_shim` only logs a warning and the run still starts. The CLI then has no MCP tools and still has native tools.
- **Expected:** a run whose `kind.reads_untrusted()` is true must fail closed (`FailureReason`, recorded and reported) until S2.3b provides verified lockdown for that CLI. Also fail closed when the shim is missing for any run.
- **Fix (minimal, in this slice):** in `command_builder`, or in the runtime launch path, return the existing fail-fast unsupported command when `task.kind.reads_untrusted()` is true, and make a missing shim a refusal for every run. Add tests: an untrusted task on `claude_code` does not get the argv; a trusted task still does; a missing shim refuses the run. S2.3b lifts the untrusted refusal per CLI.

## Non-blocking notes

- **P3:** `command_builder` passes `task.prompt` as the last positional argument. A prompt starting with `-` could be read as a CLI option. Consider `--` before the prompt if the CLI supports it (verify against the fixture first).
- **P3:** tests `the_command_points…` and `a_cli_without…` both create `01RUN` under the same hardcoded `/tmp/boba-ch5/runs`. Parallel tests race on that directory. Use a unique temp root per test.
- **P3:** `pending_slice` panics for an unmapped catalog tool. The test guards it, but a panic inside a connection thread leaves the request unanswered. Prefer a refusal string plus the test.
- **P3:** `resolve_shim`'s error message says "will start without tools", which is the fail-open case in the blocker above.

## Gate

Not run on `8067422`. It runs after the fix, with the full list from my round-2 review.

## Round 2 — ebceab9 (reviewer-2, 2026-10-16)

**Verdict: APPROVED, pending Diego's security gate.** P1 closed: untrusted task kinds and untrusted input are refused while lockdown is a stub, and a missing sidecar refuses every run. Test-only and P3 notes closed (`--print`/`--prompt` argv from fixtures, unique temp roots, no panic in `pending_slice`).

Gate on ebceab9 (reviewer-2): metadata 0, fmt 0, clippy `--workspace --all-targets -D warnings` 0, `cargo test --workspace --locked` 0 (196 passed, 0 failed across all suites), `cargo deny check` 0, pnpm install/lint/typecheck/test 0, check-i18n 0, workflow-policy 0, gen-bindings --check 0, gen_stories --check 0.

Residuals for Diego: lockdown and S2.3b must lift the untrusted refusal per CLI before any untrusted run is enabled; Codex/OpenCode/Aider argv and lock-down flags stay with S2.3b/S2.4.
