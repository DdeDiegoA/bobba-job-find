# ADR-002 — AI engine via user-installed agent CLIs + the boba MCP tool surface

- **Status:** Accepted · Approved by Diego 2026-10-04 (§17 Q6), amended by ADR-010 · 2026-10-04
- **Spine:** AD-4, AD-5, AD-6, AD-7

## Context
Locked decision: the app detects installed agent CLIs (Claude Code, Codex, OpenCode, Gemini CLI, Aider…). The user picks the CLI and the model, and the app ships a specialized job-search agent. Upstream is Claude-Code-only. The CLIs differ in flags, event formats, MCP configuration and the ability to lock down tools. Job postings and recruiter emails are untrusted text that will reach the LLM.

## Decision
1. An **`AgentPort`** in the core with one **`CliDriver`** per CLI. Each driver handles detect, version, models, capability probe, command build and JSONL parse, and emits normalized `AgentEvent`s. Capabilities are probed and tiered: Full, Limited or Text-only.
2. The agent is a **portable agent pack** (`agent-pack/system.md` plus one Agent-Skills `SKILL.md` per task). For each run it is rendered into a fresh workspace as `CLAUDE.md`, `AGENTS.md` or `GEMINI.md`, as the driver needs.
3. The agent's **only** tools come from **boba MCP**: a stdio sidecar `boba-mcp` that proxies to the core over an owner-only local socket or named pipe, with a per-run token. Each run sees only its task's tool subset. Native shell, file-write and web tools are disabled where the CLI allows it. Otherwise the CLI cannot run tasks that read untrusted content at all (ADR-010 supersedes the earlier "degraded, no auto-apply" rule).
4. **The LLM proposes, the core disposes.** Tools only read, propose or save drafts. Scores are deterministic. Sending and submitting happen only in the apply engine after a policy check.
5. Limits: max turns, max tool calls, wall-clock, max USD (where the CLI supports it), a daily run budget, and cancellation via process-tree kill.

## Consequences
- No API keys are stored. Users bring the subscription their CLI already has.
- A CLI update can break a driver. Recorded JSONL fixtures per driver in CI, plus the capability probe at runtime, contain this risk. Drivers are small (about 200 lines each).
- Each CLI start costs a few seconds. Tasks are batched (`extract_postings` handles N postings per run), and LLM extraction is limited to the top N postings by provisional score.
- The LLM provider sees task inputs. This is disclosed during onboarding.

## Alternatives
- **Direct LLM APIs (BYO key)** behind our own client. Simpler to control, but it contradicts the locked decision and makes us store keys. It could be added later as one more `CliDriver`-like adapter (`ApiDriver`) behind the same port.
- **Local HTTP MCP server** instead of the stdio shim. Header and auth support varies between CLIs, any local process can reach a TCP port, and firewall prompts appear on Windows. Rejected.
- **Running the main app binary as the MCP server.** Windows GUI-subsystem stdio quirks and single-instance conflicts. Rejected in favour of the tiny shim.
