# ADR-014 — Per-run untrusted-content gate: pinned config plus the run's own init event

- **Status:** Accepted · Approved by Diego (standing OK, 2026-10-16) · built in S2.3b and S2.4
- **Spine:** AD-4, AD-6, AD-7 · refines ADR-010

## Context
ADR-010 says an `untrusted` task needs a verified `tool_lockdown`. A probe at detection time is not enough: the CLI binary can be updated between detection and the run, the generated config can fail to apply, and a driver that only checks a version string trusts a claim instead of an observation.

## Decision
A task with `input_trust = untrusted` runs only if **both** checks pass, and the second check is made **per run**:

1. **Pinned config (S2.3b).** The CLI version has a pinned lockdown config in the driver. A version outside the verified `(major, minor)` range, or an empty version, has no `tool_lockdown`. Claude Code runs with `--strict-mcp-config --tools "" --allowedTools mcp__boba__*` and `stream-json` output. Codex and OpenCode have configs, but count as untrusted-capable only when their init event is verifiable. Aider has no MCP and is refused.
2. **The run's own init event (S2.4).** Before any tool call, the runtime reads the init event of *this* run. It must list only `mcp__boba__*` tools. Otherwise the run aborts with `Unsupported`. **A missing init event fails closed.** An auth or quota failure on a run that never showed its init event keeps its own failure reason (FR-21); it is the one path that does not report `Unsupported`.

Related rules fixed in the same slices:
- The agent sandbox endpoint is owner-only: a `0700` directory on Unix and a protected owner-only DACL (`D:P(A;;GA;;;OW)`) on Windows. Stale `ipc/*.sock` and `runs/*` entries are swept at startup; symlinks are removed, never followed.
- Tools that write agent output (`save_review`, `save_answers`, `save_document_draft`) bind `app_id` to the run's own application (S7.8). The agent's `app_id` argument is not trusted.
- Probing again before an untrusted extraction (S3.2) keeps a stale detection result from deciding the gate.

## Consequences
- The gate is observed, not assumed. A CLI update that breaks lockdown stops untrusted runs on the next run, with no new release needed.
- Each driver update needs a recorded fixture of the init event (ADR-010 consequence stays).
- Only Claude Code is verified for untrusted tasks in v1. This is the safe direction and is stated in the UI (Limited mode, S3.3).

## Alternatives
- **Probe once at detection:** stale after a CLI update. Rejected.
- **Version check only:** trusts a claim. Rejected.
- **OS-level network sandbox around the CLI:** per-OS work and it blocks the CLI's own model API. Still deferred (ADR-010).
