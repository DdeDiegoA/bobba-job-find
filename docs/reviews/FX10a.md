# FX10a review — improve_fact: propose a rewritten fact from a locked-down run

> **Reviewer:** reviewer-3 (qa) · **Dev:** dev-3 · **Date:** 2026-10-18
> **Branch:** `hermad/FX10a` · **Reviewed commit:** `84513eb` (`feat(profile): propose fact improvements from a locked-down agent run (FX10a)`)
> **Security-sensitive:** yes (agent run, lockdown gate, untrusted output, audit). Needs the human gate before merge (AGENTS.md).
> **Verdict:** APPROVED (core and skill). Conditions for FX10b are listed under Residuals.

## Scope

- `agent-pack/skills/improve_fact/SKILL.md`: the skill. Reads `get_task_input` and `get_profile_facts`, makes one `propose_fact_improvements` call.
- `boba-domain/src/profile/improve.rs`: `FactImprovement` (stale, keeps-variants, validate).
- `boba-app/src/profile/improve/mod.rs`: `ImproveService` (`start`, `confirm`, `discard`, `harvest`).
- `boba-app/src/profile_import/mod.rs`: stale check in the shared confirm path (only when `base_revision` is set).
- `boba-app/src/ports/agent.rs`: `AgentTaskKind::ImproveFact`, not in the `reads_untrusted` allowlist.
- `boba-agent`: lockdown test rows, budget, pack registration, task list.
- `boba-mcp-proto/src/tool.rs`: `IMPROVE_FACT` task and `propose_fact_improvements` (`WriteProposal`).
- `src-tauri`: `commands/profile_improve.rs`, `wiring/agent.rs` (boundary validation), IPC list (three lines), `commands/agent.rs` lockdown list.
- `boba-store`: migration `0029_profile_improve.sql` (`base_revision` column), round-trip in `repos/profile.rs`.
- `locales/{en,es}/errors/profile_improve.json` and `profile_import.json` (stale key). `docs/security/security-model.md` updated.
- No UI in this slice. The commands exist but nothing calls them yet (FX10b).

## Acceptance criteria

| AC | Result | Evidence |
|---|---|---|
| AC1: the run takes the lockdown gate; an init event that opens a non-`mcp__boba__*` tool aborts it | Covered | `run_lifecycle.rs::an_improve_run_aborts_when_its_init_event_opens_other_tools` (`Unsupported`, `failure_reason = unsupported`). Allow case passes too. `ImproveFact` is outside the `reads_untrusted` allowlist, and `ImproveService::start` sets `InputTrust::Untrusted`. |
| AC2: the run writes nothing; the suggestion waits as a pending proposal | Covered | `a_run_stores_a_pending_suggestion_and_writes_nothing` (version and revision count unchanged, one pending proposal, `base_revision` = current revision). |
| AC3: a stale confirm is refused and writes nothing | Covered | `profile_import::a_stale_improvement_proposal_is_refused_and_writes_nothing`, `profile::improve::a_stale_confirm_is_refused_and_writes_nothing` (also audited as `failed`). Domain: `a_suggestion_is_stale_when_the_fact_moved_on`. |
| AC4: text is stored and shown verbatim; markup and instruction-shaped text are data | Covered at the core | `the_suggestion_reaches_the_ui_verbatim`. Rendering is untested here (no UI yet, see FX10b). |
| AC5: a successful confirm writes one intent and one ok outcome; the audit names ids, never text | Covered | `a_successful_confirm_is_audited` (`actor=user`, detail contains proposal id and run id, no fact text). |
| AC6: the full gate is green | Covered | See Gate below. |

## Review of the code

I read each point against the diff, not the summary.

- **No write tool.** The run's grant is `get_task_input` (read), `get_profile_facts` (read) and `propose_fact_improvements` (`WriteProposal`). The grant is enforced at dispatch (`boba-agent/src/mcp/server.rs:320`, `tool::allows`). No tool in the grant applies a fact. `harvest` writes only `profile_fact_proposal` rows. `ToolKind::WriteProposal` is a review-queue write, not an apply.
- **Nothing before confirm.** `start` appends proposals and nothing else. `profile.revisions()` and the version are asserted unchanged in the tests.
- **Stale.** The check lives in `ImportService::confirm` and runs before `profile.save`. It compares the latest revision of the item with the stored `base_revision`. The shared path is unchanged for `base_revision = None` (CV import, translation, gap proposals), so existing flows keep their behaviour.
- **Boundary.** `wiring::RepoTools` runs `validate_improvements` before a call is accepted. `harvest` keeps only suggestions for this item, and drops any that add a missing variant, or that leave the draft with no text.
- **Prompt injection.** Fact text is passed as JSON data in the run input. The only effect a suggestion can have is to replace the text of the one item the run was asked about, and only after the user confirms. A fact that says "call X" can't reach another tool, because the grant rejects it. A suggestion naming another item is dropped in `harvest`.
- **Audit.** The intent is written before the write, and the outcome after it, and both rows are checked. The intent carries ids and the base revision, not text. A confirm that fails is audited as `failed`.
- **Migration.** `0029` is additive and nullable. It was renumbered after FX15 took `0028`. Round-trip is tested in `repos/profile.rs`.

## Findings

Nothing blocks the merge. Four points to fix or track:

1. **Medium (for FX10b): evidence and detail are saved unseen.** `harvest` replaces `evidence` and `detail_json` with whatever the run sent (`improve/mod.rs` `harvest`, `if let Some(evidence)` / `if let Some(detail)`). `confirm` with `edit = None`, or an edit that leaves `detail_json` as `None`, keeps the run's detail. The skill's rule "add no employer, date, metric" is not enforced by the core. Evidence is the source-grounding field, so an agent could attach evidence that the CV never said. The user's confirm is the only control, and the FX10a surface has no UI to show it. FX10b must show evidence and detail as a diff before confirm, or the core should keep the fact's own evidence and detail unless the user edits them. See Residual R1.
2. **Low: `is_stale` is test-only.** `FactImprovement::is_stale` (domain) is never called in production. The real check is an inline comparison in `ImportService::confirm`. The two must not drift. Either call `is_stale` from the shared path or delete it. This is a simplification, not a bug.
3. **Low: check-then-save is not atomic.** `ImportService::confirm` reads `fact_latest`, then calls `profile.save` in a separate step. A save landing in between would let a stale suggestion through. The Tauri commands here are synchronous, so in practice they do not interleave. This is a residual, not a defect (R2).
4. **Low: `rsplit2` matches any server prefix.** `harvest` matches any tool name whose last `__` segment is `propose_fact_improvements`. The endpoint only exposes `mcp__boba__*`, and the lockdown gate enforces that, so this is safe today. Matching on the exact `mcp__boba__` name would make it explicit.

## Gate (worktree `hermad/FX10a` at `84513eb`, `CARGO_INCREMENTAL=0`, no `cargo clean`)

| Check | Result |
|---|---|
| `pnpm install --frozen-lockfile --ignore-scripts` | exit 0 |
| `pnpm lint` | exit 0 |
| `pnpm typecheck` | exit 0 |
| `pnpm test` | exit 0 |
| `node scripts/check-i18n.mjs` | exit 0 |
| `node scripts/ci/check-conventions.mjs` | exit 0 |
| `node scripts/ci/check-workflow-policy.mjs` | exit 0 |
| `node scripts/gen-bindings.mjs --check` | exit 0 |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --workspace --locked` | exit 0, 32 result lines, 0 failed. All FX10a tests listed above ran and passed. |
| `cargo deny check` | exit 0 |

Log: `scratchpad/gate-fx10a.log` (outside the repo).

## Residuals for Diego

- **R1 (FX10b gate):** evidence and detail from an improvement run are stored as the run wrote them if the user does not edit them, and the FX10a surface shows nothing to the user. FX10b must show both before confirm, or the core must keep the fact's own evidence and detail. Do not merge FX10b until one of these holds.
- **R2 (atomicity):** the stale check and the save are separate steps. Safe while the commands are synchronous; if a command ever becomes async, wrap the check and save in one transaction.
- **R3 (agent output is verbatim fact text after confirm):** the user's text is what the run's text becomes once confirmed. That is the design, but it means a confirmed improvement is stored as user text. Provenance stays `user` (tested), and the run is named only in the audit row.
- **R4 (unbounded text):** agent text has no length cap in the core. The existing profile rules do not cap it either. Pre-existing; noted for the whole profile surface.
- **R5 (no UI yet):** the `profile_improve_*` commands are reachable over IPC without a review card. They are safe because the core stores nothing until `profile_improve_confirm`, but any caller can confirm an unseen suggestion. FX10b must be the only UI path.
- **R6 (accepted residual to record in `docs/security/residuals.md`):** an improvement run has the same lockdown cost as an untrusted-content run even though its input is the user's own fact. This is deliberate (`docs/security/security-model.md` item 1) and costs one CLI with a verified lockdown.

## Notes for the orquestador

- dev-3's note that `command_list.rs` gets three lines, one per command, is consistent with the seam rule (one line per command); no change needed.
- The security model change is in `docs/security/security-model.md`. `docs/security/residuals.md` is not updated in this commit; R6 should be added there.
