# AGENTS.md

Boba Job Find is a local-first, free and open-source desktop app (Tauri v2 +
Rust core + React UI). It runs a daily job search, ranks offers
deterministically, tailors CVs and cover letters, and applies within the limits
the user sets. No cloud, no accounts, no telemetry. It productizes
https://github.com/MadsLorentzen/ai-job-search.

All AI agents and contributors read this file first. Write code, docs and
commits in **English**.

## Status

**v1 feature-complete (2026-10-17).** All 22 build waves are merged, plus the
final QA pass (S10.6), the accessibility fixes (CH14) and the docs and linters
(FD1–FD3, CH13). The release is not tagged yet: see "Release checklist" below.

## Start here

| Need | Read |
|---|---|
| Architecture map, hexagon rules, seams, security invariants | `docs/contributing/ai-agents.md` |
| Formatting, naming, commits | `docs/contributing/conventions.md` |
| Decisions | `docs/architecture/ARCHITECTURE-SPINE.md` (AD-1..22), `docs/architecture/adr/` (ADR-001..017) |
| Requirements | `docs/product/` (PRD, epics, backlog, slices) |
| Security model + accepted residuals | `docs/security/security-model.md`, `docs/security/residuals.md` |
| Design system and voice | `docs/design/DESIGN.md`, `docs/design/voice-and-tone.md` |
| Users | `README.md`, `docs/user-guide/`, `docs/install/` |
| Release | `docs/install/RELEASING.md`, `docs/qa/platforms.md`, `docs/qa/accessibility.md` |
| Per-slice review history | `docs/reviews/` |

## Locked product decisions

- Personal use first; free and MIT-licensed; no accounts and no billing.
- AI engine = the user's own agent CLI (Claude Code, Codex, OpenCode, Gemini CLI…), driving a specialized ai-job-search agent through `boba-mcp`.
- Applying: human-in-the-loop review and full auto, configurable per source and score threshold.
- Sources: official/aggregator APIs, RSS and public ATS JSON by default. Session-board browser automation only with a per-source opt-in that carries a ToS/ban warning.
- Data: local-only and encrypted (SQLCipher + OS keychain). Manual export and backup.
- CV output: ATS-friendly PDF (Typst) and DOCX, bilingual ES/EN. UI in EN + ES. Light and dark themes follow the OS.
- Aesthetic: playful-warm boba; tone: encouraging coach.

## Build and test (the CI gate)

```sh
pnpm install --frozen-lockfile --ignore-scripts   # run this BEFORE gen-bindings
pnpm tauri dev                                     # app (dev builds use a separate keychain service + data dir)
pnpm lint && pnpm typecheck && pnpm test           # web half of the gate (Biome, tsc, vitest)
node scripts/check-i18n.mjs                        # en/es locale parity
node scripts/ci/check-conventions.mjs              # repo conventions
node scripts/ci/check-workflow-policy.mjs          # GitHub workflow policy
node scripts/gen-bindings.mjs --check              # bindings up to date (run without --check after adding a command/event)
CARGO_INCREMENTAL=0 cargo fmt --all --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked
cargo deny check
```

- A fresh worktree without `pnpm install` may write an empty bindings file; install first.
- Optional pre-commit gate: `git config core.hooksPath .githooks`.
- If `boba-mcp` tests fail with "Permission denied", delete `target/debug/boba-mcp` and rebuild.
- Each `target/` is 6–12 GB. Keep `CARGO_INCREMENTAL=0` and clean worktrees you no longer use.

## Architecture rules (short form)

- Hexagon: `boba-domain` (pure types and rules) → `boba-app` (use cases + ports) →
  adapters (`boba-store`, `boba-sources`, `boba-agent`, `boba-mail`,
  `boba-docs`, `boba-browser`, `boba-embed`) → `src-tauri` shell → `src` UI.
- `boba-domain` depends only on serde, thiserror, time and ulid.
  Adapters never depend on adapters. `boba-app` never depends on an adapter or on Tauri.
- Only `boba-store` opens the SQLCipher database. Everything else goes through core use cases.
- Every type that crosses a port lives in `boba-domain`.
- Enforced by `src-tauri/tests/dependency_rules.rs`.
- The agent CLI talks to the app only through the `boba-mcp` stdio shim: the LLM proposes, the core disposes.

## The composition seams (ADR-012)

Add registration lines only where the repo says so:

- Commands and events: one line in `src-tauri/src/ipc/command_list.rs`.
- Ports: one line in `src-tauri/src/wiring/port_list.rs`.
- Window, tray and single-instance lifecycle hooks: extend `lifecycle::install`.
- Never edit `src-tauri/src/lib.rs` or `src-tauri/src/ipc/mod.rs` to register
  anything. The composition root and the specta builder live there and are
  shared by every feature.

## Hard rules

- Tauri event names use `:` between segments (`task:progressed`) and never a
  dot; Tauri 2.12 rejects dotted names (ADR-017). Emit errors are logged, never swallowed.
- The UI calls generated bindings, never `invoke()` directly. Never hand-edit
  `src/lib/ipc/bindings.ts`; regenerate it.
- Core error keys live in `locales/{en,es}/errors/<area>.json`, never in
  `common.json`. A new area needs one line in `ERROR_FRAGMENTS`
  (`boba-app/src/i18n/mod.rs`) (ADR-016). Every UI string ships in en + es.
- Migrations (`boba-store/migrations/NNNN_name.sql`) are append-only, with
  unique, contiguous numbers. Take the next free number at rebase time, and never
  edit a shipped migration (ADR-015).
- Tests never touch the real keychain, the real app-data directory or the
  network. Use temp dirs, injected clocks and recorded fixtures. Never call
  `wiring::store_config` or `wiring::mount*` under `tauri::test::mock_app()`:
  it resolves to the real `~/Library/Application Support`.
- Lockfiles (`Cargo.lock`, `pnpm-lock.yaml`) are never hand-merged: rebase and regenerate.
- Workspace dependencies are append-only. Any new `deny.toml` advisory ignore is
  a security change and needs a written reason.
- Accessibility: every screen stays keyboard-operable and screen-reader-labelled,
  and passes `e2e/a11y`. Text colours meet 4.5:1; `--fg-subtle` is tuned for that.

## Security invariants

Never weaken these. A change touching one needs explicit security review:

- Untrusted-content runs need a CLI with a verified tool lockdown. The run's own
  init event must list only `mcp__boba__*` tools, else it aborts. It fails
  closed and has no override (ADR-010, ADR-014).
- The LLM proposes, the core disposes. Applies are executed by the core, never by LLM tools.
- Server-side Ready + verified gate before any send. `decide()`, the caps, the
  spacing, the Outbox hold, the duplicate guard, the fraud badge and the kill
  switch apply on every auto path, including session boards.
- SQLCipher + OS keychain; passphrase fallback zeroized; locked store means the run is skipped and the user is notified.
- OAuth: loopback + PKCE S256 + single-use state; least scopes.
- A single outbound network policy (https only, observed network log). No secrets in logs, exports or audit URLs.
- The audit chain uses its canonical form (ADR-011); retention never touches it.
- Updates are minisign-verified before extraction. macOS builds are signed with the one pinned self-signed identity (ADR-009).

Details: `docs/contributing/ai-agents.md`, `docs/security/security-model.md`.
Accepted residuals: `docs/security/residuals.md`.

## Style

English everywhere. Biome for TS/JSON/CSS, rustfmt and clippy (workspace
`[lints]`) for Rust. Commit subjects: `type(scope): imperative summary`.

## Release checklist (owner: Diego)

1. Move `.hermad/keys/` (the updater minisign key + password, and the macOS
   signing identity) to GitHub secrets plus an offline backup. **Never delete
   `.hermad/keys/`** before that. The steps are in `docs/install/RELEASING.md`.
2. Register the bundled public MS Graph and Google OAuth clients, and set
   `BOBA_GRAPH_CLIENT_ID`, `BOBA_GMAIL_CLIENT_ID` and `BOBA_GMAIL_CLIENT_SECRET` as CI secrets.
3. Protect `v*` tags, or require a reviewer on the release environment.
4. Run the manual QA on real hardware: the keyboard, VoiceOver, NVDA and 200% zoom
   passes (`docs/qa/accessibility.md`), and the tray, notifications, autostart and
   keychain checks on macOS, Windows and GNOME/KDE (`docs/qa/platforms.md`).
5. Make the first tag a `-beta` pre-release (`release.yml` can't be exercised
   locally), then set the CHANGELOG version.
6. Apply to the SignPath Foundation (Windows signing). Apple Developer comes later.

## Agent team workflow (Hermad)

This repo was built by a Herdr/Hermad agent team: the `orquestador` routes the
work; `analyst`, `architect`, `pm` and `reader` live in the `producto` tab;
`dev` in `desarrollo`; `reviewer` in `qa`; `ux` in `diseño`. Rules that still apply
to agent work:

- Only the orquestador writes this file. Workers read it.
- One slice = one worktree at `.hermad/worktrees/<id>`, on branch `hermad/<id>`.
  Edit, build and commit only there, never in the main checkout.
- Each dev is paired with one reviewer, who runs the full CI gate inside the slice
  worktree and writes `docs/reviews/<id>.md`. The orquestador merges only
  reviewer-approved branches (`--no-ff`).
- Security-sensitive changes (auth, tokens, crypto/keys, sandbox and lockdown,
  browser sessions, signing, advisory ignores, permission or CSP loosening) need
  the human gate.
- At most 3 slices build in parallel. Pause one if free disk drops below 3 GB.
- Agents talk peer-to-peer (`hermad send <peer> "..."`) and emit
  `HERMAD:DONE|BUG|STORIES_READY story=<id> n=<seq>` on its own line.

## Build history (condensed)

The full wave log with merge SHAs is in `git log --merges` and in
`.hermad/memory/AGENTS-build-era.md` (local). The per-slice reviews are in `docs/reviews/`.

- **W1–W4** (2026-10-05): scaffold, encrypted store, IPC, shell and UI kit, audit chain, agent runtime, sources foundation.
- **W5–W8**: composition root (ADR-012), MCP sandbox and tool lockdown (S2.3a/b, S2.4), OAuth core (S8.1/S8.2), unlock and passphrase (S1.2b).
- **W9–W12**: document studio (S7.x), scoring (S6.1–S6.4), source adapters (S4.x), scheduler (S5.1), server-side Ready + verified send gate (S9.2).
- **W13–W16**: CLI probe, email and ATS apply, session-board foundation (S11.1a/b, S11.2), auto-apply policy, Outbox and kill switch (S9.6–S9.8), backup and restore (S10.2), mail readers (S8.3).
- **W17–W20**: onboarding, Limited mode, budgets, gap interview, notifications, fraud badge, autostart, valid event names (CH10), embeddings opt-in, duplicate guard, BYO Gmail read (S8.5), data export, network log, retention.
- **W21–W22** (2026-10-17): release pipeline, signing and updater (S10.5); session-board Auto with core-only limits (S11.3); accessibility and platform QA (S10.6).
- **Final phase** (2026-10-17): architecture and security docs (FD1); README, CHANGELOG, user guide, CONTRIBUTING and SECURITY (FD2/FD3); cleanup, linters, `check-conventions` and AI-agent docs (CH13); accessibility fixes A11Y-1..8 (CH14).
