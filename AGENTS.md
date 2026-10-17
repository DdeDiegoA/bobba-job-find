# AGENTS.md — shared project context

> Internal memory. ALL agents read this at startup. **Only the orquestador writes it** (workers only read). The orquestador updates it after worker reports.

Communication: talk to Diego and write all documents in **English**.

## What this project is
**Boba Job Find** — an open-source, local-first, cross-platform desktop app (Windows/macOS/Linux) that productizes https://github.com/MadsLorentzen/ai-job-search (manual: `/Users/diegoarenas/Downloads/ai-job-search-manual.md`). It onboards the user's job-board + email connections and personal profile, runs a daily search job, ranks every offer (fit vs requirements + offer quality), tailors CV/cover letter per application, and applies (human-in-the-loop or full auto, user-configurable).

## Locked product decisions (from user grilling, 2026-10-04)

| Topic | Decision |
|---|---|
| Audience / model | Personal use first, **free & open source**. No accounts, no billing. |
| AI engine | **Agent CLI**: detect every agent CLI installed (Claude Code, Codex, OpenCode, Gemini CLI, Aider…), user picks CLI + model; ship a **specialized ai-job-search agent** (prompts/skills/tools) that runs on the chosen CLI. |
| Apply automation | **Both modes**, configurable per source/score threshold: human-in-the-loop (review → send) and full auto-apply. |
| Markets (MVP) | Global remote (EN), LATAM (ES), US/Canada, Europe + **user-defined custom sources** (RSS/URL/ATS board/custom adapter) where feasible. |
| Non-API boards (LinkedIn, Indeed…) | **Both, per-source opt-in**: official/aggregator APIs + RSS + public ATS JSON by default; user-session browser automation behind explicit opt-in with ToS-risk warning. |
| Data | **Local-only, encrypted**: SQLite (encrypted) + OS keychain for secrets; manual export/backup. No cloud. |
| CV output | ATS-friendly PDF, DOCX export, multiple templates/themes, **bilingual ES/EN** auto-matched to offer language. |
| Scheduler | In-app + **system tray** background run while logged in; native notifications with top matches. |
| UI language | **EN + ES from day 1** (i18n built in). |
| Brand | Name **Boba Job Find** (UX may propose refinements); no assets exist — UX proposes logo, palette, fonts. |
| Aesthetic | **Playful-warm (boba)**: soft rounded shapes, warm tea/tapioca palette, friendly mascot, calm against job-search stress. |
| Tone | **Encouraging coach**: warm, motivating, honest about fit. |
| Theme | Light + dark, follow OS. |
| Shell | **Tauri v2** (architect validates). User strengths: TypeScript/React, Python, Rust. |
| Deliverable | Repo docs under `docs/` + a published visual overview page linking it all. |
| Signing | Free OSS path first: macOS = one persistent self-signed cert (no ad-hoc, see ADR-009) + documented Gatekeeper bypass; Windows = SignPath Foundation; Apple Developer later. |
| OAuth redirect | Loopback-only + PKCE S256 + single-use state; deep-link fallback removed (ADR/§17.1). |
| OAuth | Bundled public desktop clients (PKCE, loopback) + BYO client-ID fallback. |
| Gmail | MVP `gmail.send` only; reply parsing via MS Graph / IMAP / BYO client; CASA deferred. |
| Embeddings | Optional opt-in local model download; deterministic scoring by default. |
| Session-board auto-apply | **In MVP**, per-source opt-in with ToS/ban warning, caps, kill switch, audit log; executed by core, never by LLM tools. |
| DB/Security | SQLCipher + keychain + boba-mcp shim approved; audit fixes all approved. |
| macOS key safety | Every macOS build signed with ONE persistent self-signed cert (pinned DR) so Keychain survives updates. |
| Prompt injection | Hard gate: untrusted-content tasks only on CLIs with verified tool lockdown; no override. |
| Passphrase fallback | Unlock once per login, zeroized memory + idle timeout; locked → run skipped + notification. |

## Stack (architect-approved)

| Layer | Tech |
|---|---|
| Shell | Tauri v2, hexagonal modular monolith in Rust core |
| UI | TypeScript + React, Tailwind v4 + shadcn, self-hosted fonts |
| Engine | User-selected agent CLI ↔ app only via `boba-mcp` stdio shim (LLM proposes, core disposes) |
| Scoring | Deterministic offline Fit + Quality (offer-scoring.md), optional embeddings |
| Docs | Typst (PDF) + docx-rs (DOCX); no LaTeX |
| Storage | SQLCipher via rusqlite + keyring-core |
| Browser automation | chromiumoxide driving user's own Chrome (opt-in) |
| Python sidecar | None |
| Source of truth | `docs/architecture/ARCHITECTURE-SPINE.md` (AD-1..AD-22), `tech-stack.md`, `adr/` |

## Active personas (roster)

- `orquestador` — routes phases + approves plans
- `analyst` / `architect` / `pm` / `reader` — tab **producto**
- `dev` — tab **desarrollo**
- `reviewer` — tab **qa**
- `ux` — tab **diseño**

## Current phase
**BUILD PHASE — MVP (started 2026-10-05) — RESUMED 2026-10-15: Diego's goal = run all remaining waves sequentially to v1, then cleanup + project docs + AI-agent docs (rules, linters).** Human gate passed. Backlog: `docs/product/backlog.md` (76 slices, 21 waves, ≤4 parallel devs; source `docs/product/slices.yaml`). Root `./stories.yaml` = current wave only (`python3 scripts/gen_backlog.py --wave N`). Sprint status: `_bmad-output/implementation-artifacts/sprint-status.yaml`.

Build rules (Diego, 2026-10-05):
- Devs: from 2026-10-16 (Diego) new devs run on **claude-deepseek** (Claude Code CLI against DeepSeek's Anthropic-compatible API, model deepseek-flash[1m]; spawned via a pane with the ANTHROPIC_* env). Earlier: `opencode` DeepSeek V4.1 Flash. Branch `hermad/<story>` in a worktree per slice.
- STANDING APPROVAL (Diego 2026-10-16): Diego pre-approves the orquestador's recommended option on every gate (security gates included). Run uninterrupted to v1; apply the recommended choice, log it in the wave log + journal as 'Diego standing OK (recommended)', and report at milestones. Still escalate only if no option is clearly safe/recommended.
- Concurrency (Diego 2026-10-16; 2026-10-17 back to 3 after Diego freed disk): MAX 3 slices building in parallel (each worktree target is ~6-12G; pause one if free disk < 3G). Queue the rest; start the next when one merges. Reviewers gate inside the slice worktree, never in extra copies.
- Review pairing (Diego 2026-10-15): one dedicated reviewer per dev (reviewer-N ↔ dev-N), reviewers run on Claude **Haiku** (`hermad spawn reviewer --name reviewer-N --kind claude --model haiku`). Fast, terse reviews. Avoid context rot: respawn devs+reviewers fresh each wave; short briefs.
- Extra devs: the orquestador may ONLY add helper devs to support opencode when an opencode dev is consuming too much (stuck, looping, burning context/time). Those helpers run on Claude **Sonnet or Haiku** (`hermad spawn dev --name dev-<n> --kind claude --model sonnet|haiku`), never Opus. No other new agents for build work.
- Human gate = SECURITY ONLY (Diego, 2026-10-05): orquestador merges every reviewer-approved slice without asking, EXCEPT security-sensitive ones, where it asks Diego first: S1.2b (unlock/passphrase), S1.7 (audit-log integrity), S2.3a/S2.3b (MCP sandbox, tool lockdown, untrusted content), S8.1/S8.2/S8.5 (OAuth/tokens), S10.5 (code signing), S11.1a/S11.1b/S11.2/S11.3 (session cookies/browser automation), and any new security exception (advisory ignores, crypto/key changes, permission/CSP loosening). Auto-apply (S9.6–9.8) and restore/reset (S10.2) are NOT gated.
- STOP POINT lifted (Diego, 2026-10-15): run W5→W22 back-to-back; stop only for security gates, conflicts, failures. Clean up dev agents + worktrees after each wave.
- Cadence: waves run back-to-back; report to Diego at walking skeleton (W2–W4) and at each completed epic. Stop only for gates, conflicts, failures.
- IPC registration (AD-3, ruled 2026-10-05): add commands/events ONLY as one line in `src-tauri/src/ipc/command_list.rs` (union-merge); `ipc/mod.rs` is owned by S1.3 and never edited later; `registry.rs` retired.
- `Cargo.lock` / lockfiles: never hand-merged — rebase onto main and regenerate; workspace deps append-only.
- Port types (AD-14, ruled 2026-10-05): every type that crosses a port lives in `boba-domain` (document model moves to `boba_domain::documents` in S7.2).
- Review gate (Diego, 2026-10-05): the reviewer runs the FULL CI gate (incl. `cargo deny check`) on every slice before approving; any new advisory ignore is a security gate.
- Worktree discipline: devs edit/build/commit ONLY in `.hermad/worktrees/<story>`; never in the main checkout (S7.1 incident 2026-10-05).
- Ownership conventions in backlog.md (module stubs, pinned deps, union-merge command registry, generated bindings, per-slice migrations/locale namespaces) are binding.

Wave log: W1 S1.1 ✅ merged (25ee138). W2: S1.2a ✅ c0b301e (Diego OK); S1.3 ✅ f3006a6; S1.8 ✅ 260706e. **Wave 2 complete.** W3: S1.3p ✅ b17e96b; S1.4 ✅ 6842927; S2.1 ✅ 639213d; S7.1 ✅ a83d2d1. **Wave 3 complete.** W4: S1.7 ✅ ba75df8 (Diego OK, residual accepted, ADR-011); S2.2 ✅ eafa3e6; S1.5 ✅ ae716cd; S4.1 ✅ 4abfd61. **All Wave 4 merged.** Full-gate verify of main: 12/13 pass; cargo deny red (6 typst-transitive advisories) → CH2 chore (dev-1, Diego: upgrade quick-xml, documented ignores for the rest). CH2 ✅ 9999bd8 (Diego OK: documented ignores, quick-xml upgrade blocked upstream). **WAVE 4 COMPLETE — STOPPED 2026-10-05 per Diego. Resume = generate Wave 5 (starts with S4.1c Send+Sync chore, S4.1h HttpClient hardening, S3.4…).** CH1 ✅ 32db67c (dev profile debuginfo trimmed). W5: S4.1c ✅ d743b20; ADR-012 ✅; CH4 ✅ 8c9cd1b (Diego OK); CH3 ✅ 919593b (Diego: unsafe-libyaml + scoped zip@4.6.1 ignores, 0BSD); S2.3a ✅ 5c93535 (Diego OK w/ residuals→S2.3b); S3.1 ✅ 6dc6eab; S3.4 ✅ fe69eeb; CH5 ✅ 37737b9 (Diego OK; untrusted runs fail-closed until S2.3b). **Wave 5 complete 2026-10-16.** W6: S3.7 ✅ fdbfcb4; S4.2 ✅ 3720ed5; S8.1 ✅ af4f3e4 (Diego standing OK); S2.3b ✅ 2f43555 (Diego standing OK; ruling (c)). **Wave 6 complete 2026-10-16.** W7: S6.1 ✅ 1ca19cd; S9.1 ✅ 7927c01; S2.4 ✅ 1061294 (per-run lockdown gate); S8.2 ✅ (Diego standing OK). Integration gate on main 1061294 green. **Wave 7 complete.** W8: S4.7 ✅ ba0d014; S7.2 ✅ 5f71d69; S1.2b ✅ edf3263 (Diego standing OK). **Wave 8 complete.** W9: S7.3 ✅ 10a78f4; S7.4 ✅ c83d71d; S7.5 ✅ 8823813. **Wave 9 complete.** W10: S7.6 ✅ 033ccf5; CH9 ✅ a35f656 (zip 4.6.1 RUSTSEC-2026-0367/0368/0369 ignores, Diego standing OK, revisit S10.5); S7.8 ✅ 67b9300 (app_id bound to run); S4.3 ✅ d47845f; S6.2 ✅ 3c61336; S6.3 ✅ 9e32b93 (AC4 reconciled to §9). **Wave 10 complete.** W11: S1.6 ✅ b672a24 (2 granted lib.rs delegation lines); S6.4 ✅ 76d29e2; S4.4 ✅ 0048285; S7.7 ✅ 7922672 (studio end-to-end; inline edits inherit taint; export of unverified drafts allowed with warning, send gated server-side in S9.2/S9.3). **Wave 11 complete.** W12: S4.5 ✅ 4ccfff7 (https-only ATS apply URLs); S5.1 ✅ a74f535 (locked store => skipped + notify); S9.2 ✅ ce925fe (server-side Ready+verified gate; empty-package mark_applied intended); S4.6 ✅ c6621a4 (https-only apply_url all sources + detail render guard; HttpClient::post hardened, secret-free errors). **Wave 12 complete.** W13: S3.2 ✅ 232a7e2 (CLI probe; re-probe before untrusted extraction); S9.3 ✅ 1362be2 (email apply, server-side Ready+verified gate); S6.5 ✅ 9575f6f; S9.4 ✅ 6a86f17 (real chromiumoxide driver on user's Chrome, opt-in, no auto-submit in review; owns answer-approval surface). Integration gate GREEN on 1362be2 (1183 rust / 210 vitest). **Wave 13 complete.** W14: S11.1a ✅ f36c53a (SECURITY, Diego standing OK; residuals -> S11.1b/S10.2); S3.5 ✅ f262fef (zip>=9 in-memory, bomb caps; PDF residual accepted); S9.6 ✅ 35264e6 (local-day caps; call sites -> S9.7); S4.8 ✅ 7835d47 (run wiring -> S5.2). **Wave 14 complete.** W15: S9.7 ✅ b7877f8 (decide()+spacing+Ready gate; R-4 kill switch mount -> S9.8); S5.2 ✅ feeab4f (run registry, scoring, N+1, top-matches notify); S11.1b ✅ b5b6f02 (SECURITY Diego standing OK; 30s pacing floor; R-a atomic cap, R-b URL without query); S6.6 ✅ 7c51b7b. **Wave 15 complete.** W16: S9.8 ✅ ff627da (kill switch mounted, all auto-send paths; closes S9.7 R-4); S10.2 ✅ 7032976 (encrypted backup, validated restore, reset wipes DB+keychain+browser sessions); S8.3 ✅ 449f729 (read-only Graph/IMAP readers, sink, email_sync job); S11.2 ✅ a25b346 (SECURITY Diego standing OK; shared daily page cap; residuals -> S11.3). **Wave 16 complete.** W17: S3.8 ✅ a861a21 (first-launch redirect, forced recovery backup, first run); S3.3 ✅ ef4863a (Limited mode = S2.4 gate); S2.5 ✅ 03d9192 (core-enforced budgets; rank before agent pass); S3.6 ✅ 3921563 (gap interview, <Gated>). **Wave 17 complete.** W18: S5.3 ✅ a38bbe4 (security notices always notify); S6.7 ✅ e7ec05b; S6.8 ✅ 35156ea (fraud badge blocks Auto on all decide() paths); S5.4 ✅ 728f632 (autostart opt-in, tray run controls). Integration gate GREEN 5c901f8 (W13-17). CH10 (event bus: dotted names illegal in Tauri 2.12) in progress. **Wave 18 complete.** W19: CH10 ✅ 89339f6 (event bus fixed: valid event names, validity test); S8.4 ✅ c274504 (reply proposals via lockdown gate); S6.9 ✅ 9ed586c (opt-in embeddings, checksum before load); S9.5 ✅ 5d3ce27 (duplicate guard on every apply path, nudges); CH11 ✅ a972745 (ammonia 4.2.4; zbus-family unmaintained ignores, Diego standing OK); S8.5 ✅ 0ae26ec (SECURITY Diego standing OK; BYO-only gmail.readonly, 401/429 handling). **Wave 19 complete.** W20: S9.9 ✅ 9afac9c; S10.1 ✅ 985a0d8 (no secrets in export); CH12 ✅ edac32a (hardening/test-stability batch); S10.4 ✅ f3c1d32 (observed network log at all choke points); S10.3 ✅ 1abaab7 (retention sweep, audit chain untouched). **Wave 20 complete. PAUSED 2026-10-17 per Diego (context compaction). Resume = integration gate on main, then W21 (S10.5, S11.3 — both SECURITY), W22 (S10.6), then cleanup + project docs + AI-agent docs.** Resumed 2026-10-17; integration gate GREEN e61de38 (2108 rust / 429 vitest). W21: S10.5 ✅ 229b1b6 (SECURITY Diego standing OK; minisign verify before extract, self-signed macOS pinned DR, keys in .hermad/keys gitignored -> Diego moves to CI secrets; residuals R1-R6); S11.3 ✅ 956c972 (SECURITY Diego standing OK; core-only enforcement, 3/day max 10, 10-min spacing, 60-min hold, 7-day challenge pause, 30-day re-confirm; no automatic board submit yet). **Wave 21 complete.**
- Composition root (ADR-012, 2026-10-15): src-tauri/src/wiring/port_list.rs is union-merge; slices append one app.manage line per port and NEVER edit lib.rs. CH4 builds it; S1.2b later calls wiring::mount_ports after unlock.
- CH5 (ruled 2026-10-15): agent runtime + sandbox wiring in src-tauri/src/wiring/ after CH4 + S2.3a merge; owner dev-2; security-gated (Diego). Wave 5 closes only after CH5.
- S2.3a Diego OK 2026-10-15 with residuals → S2.3b MUST add: Windows named-pipe owner-only SDDL; sweep stale ipc/*.sock. macOS: 0700 dir is the protection (accepted).
- i18n convention (CH7, 2026-10-16): core-emitted error keys errors.<area>.* live in locales/{en,es}/errors/<area>.json (one file per area, same key names); slices create/append their own area file without asking (a new area also needs one line in ERROR_FRAGMENTS, boba-app/src/i18n/mod.rs); never put errors.* in common.json.
- S8.1 (2026-10-16, Diego standing OK): Graph scopes add offline_access (refresh tokens for daily runs); bundled Graph client id comes from build-time BOBA_GRAPH_CLIENT_ID — registering it is a release task (S10.5); BYO client id works meanwhile.
- Untrusted-content gate (S2.3b+S2.4, DONE 2026-10-16): per run — CLI version must have a pinned lockdown config AND the run's own init event must list only mcp__boba__* tools, else abort (Unsupported) before any tool call; missing init event => fail closed. Claude: --strict-mcp-config --tools "" --allowedTools mcp__boba__*, stream-json. Aider refused (no MCP). Codex/OpenCode configured; untrusted only when their init event is verifiable.
- Migrations (ruled 2026-10-16): slices.yaml migration numbers are advisory; a slice takes the next FREE number on main at rebase time (S2.4 took 0011; S7.2 must renumber).
- Test hygiene (2026-10-16): tests never touch real user data — mock keychain only; never call wiring::store_config/mount/mount_with_key under tauri::test::mock_app() (it resolves app_data_dir to the real ~/Library/Application Support); use temp dirs.
- Deferred (ruled 2026-10-15): first-launch redirect to /onboarding (S3.1 AC1) belongs to S3.8; routes.tsx stays S1.5-owned. 
- Dispatch: orquestador creates worktrees by hand (git worktree add) + hermad spawn, max 4 live devs; plan-devs is not used (it ignores live devs and reuses names). Chore CH1 (dev-profile debuginfo) dev-1.

| Area | Docs | Status |
|---|---|---|
| Research | `docs/research/` (upstream-analysis, integrations, competitors, offer-scoring) | ✅ |
| Product | `docs/product/` (product-brief, PRD, prd-addendum, epics-and-stories: 13 epics / 80 stories, E1–E11 MVP) | ✅ |
| Design | `docs/design/` (brand, DESIGN.md, EXPERIENCE.md, voice-and-tone, preview.html, assets/) | ✅ |
| Architecture | `docs/architecture/` (ARCHITECTURE-SPINE AD-1..22, architecture.md, tech-stack, adr/001–010) | ✅ |
| Audit | `docs/reviews/planning-audit.md` — round 2 closed, 20/20 resolved | ✅ |
| Overview | `docs/OVERVIEW.html` → https://claude.ai/artifact/BQPStgykq6EZoRVyxJo97V | ✅ |

Approval-gated stories (need Diego before merge): 1.2, 2.3, 8.1, 8.2, 8.5, 9.6–9.8, 10.5, 11.x.

## Team rules

- Approval policy: auto, except auth/money/DB/security → escalate to Diego.
- Single writer to the repo; parallel stories → `git worktree`/branch.
- Peer-to-peer allowed (reviewer→dev, pm→devs); the orquestador coordinates the top.
- Only the orquestador writes this file.
