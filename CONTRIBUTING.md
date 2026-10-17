# Contributing to Boba Job Find

Thanks for helping. Boba is a local-first Tauri v2 app: a Rust core with a React UI. Small,
focused pull requests that map to one idea are the easiest to review.

This page is the short version; deeper guides are in the [docs index](docs/README.md).

## Setup

You need Rust 1.99 (pinned in `rust-toolchain.toml`; `rustup` installs it on the first `cargo`
call), Node.js 24 LTS, pnpm 11 (`corepack enable pnpm`), and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```sh
pnpm install
pnpm tauri dev
```

Dev builds use their own keychain service and data directory, so they never read or overwrite
your real Boba data. Tests must do the same: use a mock keychain and temp directories, never
the real user data directory.

## The CI gate

A pull request must pass everything CI runs. Run it locally first:

```sh
# Repo hygiene
node scripts/ci/check-workflow-policy.mjs
python3 scripts/gen_stories.py --check

# Web
pnpm install --frozen-lockfile --ignore-scripts
pnpm lint                      # Biome: lint + format check
pnpm typecheck
pnpm test                      # Vitest
node scripts/ci/check-i18n.mjs # EN and ES catalogues must match
node scripts/gen-bindings.mjs && git diff --exit-code -- src/lib/ipc/bindings.ts

# Rust
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check               # licences, advisories, supply chain
```

CI also does a Tauri build on Linux, macOS and Windows. A new `cargo deny` advisory ignore is a
security decision: explain it in the PR.

## Conventions in brief

- **Architecture.** The Rust core is a hexagonal modular monolith. `boba-domain` depends only on
  a few small crates; adapters never depend on each other; `boba-app` never depends on an adapter
  or on Tauri. `cargo test` enforces this (`src-tauri/tests/dependency_rules.rs`). Types that
  cross a port live in `boba-domain`.
- **The LLM proposes, the core disposes.** The agent CLI reaches the app only through the
  `boba-mcp` shim. Anything that sends, spends or deletes is decided and executed by the core,
  never by an LLM tool.
- **IPC.** Add commands and events by appending one line to
  `src-tauri/src/ipc/command_list.rs`. `src/lib/ipc/bindings.ts` is generated; don't edit it.
- **Composition root.** Mount a new port by appending one line to
  `src-tauri/src/wiring/port_list.rs`; don't edit `lib.rs`.
- **Migrations.** One `NNNN_name.sql` file per change; take the next free number on `main`.
- **i18n.** Every user-facing string exists in `locales/en/` and `locales/es/` in the same PR.
  Core error keys live in `locales/{en,es}/errors/<area>.json`.
- **Lockfiles.** Never hand-merge `Cargo.lock` or `pnpm-lock.yaml`: rebase onto `main` and
  regenerate. Workspace dependency additions are append-only.
- **Security-sensitive changes** (unlock and keys, audit log, sandbox and tool lockdown, OAuth,
  session cookies and browser automation, signing, any CSP or permission loosening) get extra
  review. Say so in the PR description.
- **Voice.** UI copy and docs follow the encouraging-coach tone in
  [docs/design/voice-and-tone.md](docs/design/voice-and-tone.md) and the binding vocabulary
  from the [PRD](docs/product/PRD.md).

## Where to read next

[Architecture spine](docs/architecture/ARCHITECTURE-SPINE.md) ·
[Tech stack](docs/architecture/tech-stack.md) ·
[ADRs](docs/architecture/adr/README.md) ·
[Backlog and ownership rules](docs/product/backlog.md)

Please report vulnerabilities privately, as described in [SECURITY.md](SECURITY.md).
