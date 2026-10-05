# ADR-010 — Untrusted-content tasks require verified tool lockdown; agent output is untrusted

- **Status:** Accepted · Approved by Diego 2026-10-05 (planning audit P0-2, P1-8, P2-19) · 2026-10-04
- **Spine:** AD-4, AD-6, AD-7
- **Source:** planning audit P0-2, P1-8, P2-19

## Context
ADR-002 tiered CLIs as Full, Limited or Text-only, and only kept "Limited" CLIs off auto-apply. But the exfiltration risk exists in the **run itself**: an injected posting can make an agent that still has native web or shell tools send the profile to an attacker's URL, whether or not anything gets auto-applied. The `untrusted_content` envelope is a prompt-level guard and does not remove that capability. The audit also showed that the agent's own output (narration, evidence spans, drafts) can carry injected text back into the UI and into later runs.

## Decision
1. **New probed capability `tool_lockdown`.** It is true only when, for that CLI version and our generated config, the agent has **no native web or fetch tool, no shell with network egress, and no file write outside the run workspace**. It is verified by one of the following, in order of preference:
   - the CLI's init event lists the available tools, and that list contains only `boba` MCP tools (Claude Code `stream-json` init)
   - a sandbox mode that blocks network and writes outside the workspace (Codex `--sandbox read-only`)
   - a per-version lockdown config pinned in the driver and covered by recorded fixtures (Gemini `excludeTools`, OpenCode permission deny)

   A CLI whose version falls outside a driver's verified range loses `tool_lockdown` until the driver is updated.
2. **Task trust classes.** Each `AgentTask` declares `input_trust`:
   - `untrusted`: `extract_postings`, `narrate_scores`, `tailor_application`, `review_application`, `classify_emails`. These **require** `tool_lockdown`. On a CLI without it they are shown as **unsupported**: a **hard gate with no user override**. The UI explains why and suggests a Full CLI. For `classify_emails`, the deterministic rules still run.
   - `user`: `profile_import`, `translate_profile`. Their input comes from the user, so they may run on Limited and Text-only CLIs, with a security badge.
3. **Agent output is untrusted.** Every string an agent writes (narration, evidence, review findings, document text) is stored with `untrusted_origin = true` when its run had `input_trust = untrusted`. It is:
   - rendered as plain text or ammonia-sanitized in the UI, and never rendered as HTML
   - passed to Typst only as JSON data through `sys.inputs`; templates print strings as text and never `eval` or interpret them as markup
   - re-wrapped in the `untrusted_content` envelope whenever a later run reads it
4. **Workspace hygiene.** At startup the core deletes every leftover `<appdata>/runs/*` directory (left behind by a crash), before the scheduler starts.

## Consequences
- Some CLIs (Aider, "Other" templates, versions not yet verified) can't run the core ranking and tailoring loop. That is the honest trade-off: a degraded badge doesn't protect anyone's PII.
- Each driver update needs a fixture showing lockdown. This is cheap and already part of driver CI.
- The data model gains `untrusted_origin` on agent-written text columns (`score_narration`, `posting_features.evidence_json`, `document.model_json`, `review`).

## Alternatives
- **Keep Limited CLIs for all tasks with a warning.** Rejected, because a warning does not stop exfiltration.
- **An OS-level network sandbox around the CLI** (sandbox-exec, AppContainer, bwrap). Per-OS work, fragile, and it would also block the CLI's own model API. Deferred as possible v2 hardening.
