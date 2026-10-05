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
**BUILD PHASE — MVP (started 2026-10-05).** Human gate passed. Backlog: `docs/product/backlog.md` (76 slices, 21 waves, ≤4 parallel devs; source `docs/product/slices.yaml`). Root `./stories.yaml` = current wave only (`python3 scripts/gen_backlog.py --wave N`). Sprint status: `_bmad-output/implementation-artifacts/sprint-status.yaml`.

Build rules (Diego, 2026-10-05):
- Devs: all `opencode` DeepSeek V4.1 Flash. Branch `hermad/<story>` in a worktree per slice.
- Extra devs: the orquestador may ONLY add helper devs to support opencode when an opencode dev is consuming too much (stuck, looping, burning context/time). Those helpers run on Claude **Sonnet or Haiku** (`hermad spawn dev --name dev-<n> --kind claude --model sonnet|haiku`), never Opus. No other new agents for build work.
- Human gate = SECURITY ONLY (Diego, 2026-10-05): orquestador merges every reviewer-approved slice without asking, EXCEPT security-sensitive ones, where it asks Diego first: S1.2b (unlock/passphrase), S1.7 (audit-log integrity), S2.3a/S2.3b (MCP sandbox, tool lockdown, untrusted content), S8.1/S8.2/S8.5 (OAuth/tokens), S10.5 (code signing), S11.1a/S11.1b/S11.2/S11.3 (session cookies/browser automation), and any new security exception (advisory ignores, crypto/key changes, permission/CSP loosening). Auto-apply (S9.6–9.8) and restore/reset (S10.2) are NOT gated.
- STOP POINT (Diego, 2026-10-05): stop after Wave 4 completes (S7.1, S2.2, S1.5, S1.7, S4.1 merged). Do NOT dispatch Wave 5; report and wait.
- Cadence: waves run back-to-back; report to Diego at walking skeleton (W2–W4) and at each completed epic. Stop only for gates, conflicts, failures.
- IPC registration (AD-3, ruled 2026-10-05): add commands/events ONLY as one line in `src-tauri/src/ipc/command_list.rs` (union-merge); `ipc/mod.rs` is owned by S1.3 and never edited later; `registry.rs` retired.
- `Cargo.lock` / lockfiles: never hand-merged — rebase onto main and regenerate; workspace deps append-only.
- Port types (AD-14, ruled 2026-10-05): every type that crosses a port lives in `boba-domain` (document model moves to `boba_domain::documents` in S7.2).
- Worktree discipline: devs edit/build/commit ONLY in `.hermad/worktrees/<story>`; never in the main checkout (S7.1 incident 2026-10-05).
- Ownership conventions in backlog.md (module stubs, pinned deps, union-merge command registry, generated bindings, per-slice migrations/locale namespaces) are binding.

Wave log: W1 S1.1 ✅ merged (25ee138). W2: S1.2a ✅ c0b301e (Diego OK); S1.3 ✅ f3006a6; S1.8 ✅ 260706e. **Wave 2 complete.** W3: S1.3p ✅ b17e96b; S1.4 ✅ 6842927; S2.1 ✅ 639213d; S7.1 ✅ a83d2d1. **Wave 3 complete.** W4: S2.2 dev-2 (review fixes), S1.7 (gated) dev-3 (anchor fix), S1.5 dev-1 (in review), S4.1 dev-4. CH1 ✅ 32db67c (dev profile debuginfo trimmed).
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
