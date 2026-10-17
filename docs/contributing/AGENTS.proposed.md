# AGENTS.md

Boba Job Find is a local-first, free and open-source desktop app (Tauri v2 +
Rust core + React UI) that runs a daily job search, ranks offers
deterministically, tailors CVs and cover letters, and applies within limits the
user sets. No cloud, no accounts, no telemetry.

## Start here

- Architecture map and hexagon rules: `docs/contributing/ai-agents.md`; formatting: `docs/contributing/conventions.md`
- Decisions: `docs/architecture/ARCHITECTURE-SPINE.md` (AD-1..22) and `adr/`; requirements: `docs/product/`

## Build and test

```sh
pnpm install --frozen-lockfile --ignore-scripts   # run this BEFORE gen-bindings
pnpm tauri dev                                     # app
pnpm lint && pnpm typecheck && pnpm test           # web half of the gate
node scripts/ci/check-conventions.mjs              # repo conventions
node scripts/gen-bindings.mjs                      # after adding a command/event
CARGO_INCREMENTAL=0 cargo fmt --all --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked
cargo deny check
```

A fresh worktree without `pnpm install` may write an empty bindings file; install first.

## Architecture rules (short form)

- Hexagon: `boba-domain` (pure types/rules) → `boba-app` (use cases + ports) →
  adapters (`boba-store`, `boba-sources`, `boba-agent`, `boba-mail`,
  `boba-docs`, `boba-browser`, `boba-embed`) → `src-tauri` shell → `src` UI.
- `boba-domain` depends only on serde, thiserror, time, ulid. Adapters never
  depend on adapters. `boba-app` never depends on an adapter or on Tauri.
- Only `boba-store` opens the SQLCipher database. Everything else goes through
  core use cases.
- Every type that crosses a port lives in `boba-domain`.
- Enforced by `src-tauri/tests/dependency_rules.rs`.

## The composition seams (ADR-012)

Add registration lines only where the repo says so:

- Commands/events: one line in `src-tauri/src/ipc/command_list.rs`.
- Ports: one line in `src-tauri/src/wiring/port_list.rs`.
- Never edit `src-tauri/src/lib.rs` or `src-tauri/src/ipc/mod.rs` to register
  anything; the composition root and the specta builder live there and are
  shared by every feature.

## Hard rules

- Tauri event names use `:` between segments (`task:progressed`) and never a
  dot; Tauri 2.12 rejects dotted names.
- The UI calls generated bindings, never `invoke()` directly. Never hand-edit
  `src/lib/ipc/bindings.ts`; regenerate it.
- Core error keys live in `locales/{en,es}/errors/<area>.json`, never in
  `common.json`, and a new area needs one line in `ERROR_FRAGMENTS`
  (`boba-app/src/i18n/mod.rs`).
- Migrations (`boba-store/migrations/NNNN_name.sql`) are append-only, with
  unique, contiguous numbers. Never edit a shipped migration.
- Tests never touch the real keychain, the real app-data directory or the
  network. Use temp dirs, injected clocks and recorded fixtures. Do not call
  `wiring::mount*` under `tauri::test::mock_app()`.
- Lockfiles are never hand-merged: rebase and regenerate.

## Security invariants

Never weaken these; a change touching one needs explicit review:
untrusted-content tool lockdown (fail closed), LLM-proposes/core-disposes,
sanitized untrusted content, server-side apply gate, kill switch and caps,
single outbound network policy, no secrets in logs/exports, audit-chain
canonical form. Details in `docs/contributing/ai-agents.md`.

## Style

English everywhere. Biome for TS/JSON/CSS, rustfmt for Rust (both in CI).
Commit subjects: `type(scope): imperative summary`. Run
`git config core.hooksPath .githooks` once for the optional pre-commit gate.
