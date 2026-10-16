# Boba Job Find

Local-first, AI-assisted desktop job search. Tauri v2 Rust core + React 19 UI.
Personal use, free and open source. One SQLCipher database, no cloud.

This repo is the **wave 1 scaffold** (slice S1.1): the crate workspace, the
module skeleton, pinned dependencies, the command registry, the locale
catalogues and placeholder screens. Later slices fill the stubs in place.

## Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.99 (pinned by `rust-toolchain.toml`; MSRV 1.92) | install via [rustup](https://rustup.rs) |
| Node.js | 24 LTS | build only, not shipped |
| pnpm | 11 | `corepack enable pnpm` |
| Tauri Linux deps | see [Tauri prerequisites](https://tauri.app/start/prerequisites/) | `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev` on Ubuntu 24.04 |

`rustup` fetches the pinned toolchain automatically on the first `cargo` call.

## Dev command

```sh
pnpm install          # install JS deps (generates pnpm-lock.yaml)
pnpm tauri dev        # opens the Boba window (runs Vite on http://localhost:1420)
```

The first run compiles the Rust workspace; later runs are incremental.

## Build and check

```sh
pnpm typecheck        # tsc --noEmit
pnpm build            # typecheck + vite build -> dist/
cargo build           # whole Rust workspace
cargo test            # unit + dependency-rule tests (AC2)
cargo deny check      # supply chain / licences / advisories (optional locally)
```

## Layout

- `src/` — React UI (thin typed-IPC client). Screens in `src/features/<area>/`,
  router in `src/app/routes.tsx`, design tokens in `src/styles/tokens.css`.
- `src-tauri/` — Tauri app (`src/lib.rs` registers every MVP plugin) and the
  Rust core crates under `src-tauri/crates/`.
- `locales/{en,es}/` — shared i18n catalogues, one file per namespace (AD-21);
  core error keys sit one file per area under `errors/` (CH7).
- `agent-pack/` — the portable Boba agent: `system.md` + `skills/<task>/SKILL.md`.
- `templates/` — Typst CV/cover templates and fonts.
- `tools/` — dev tools (fake agent CLI).

Source of truth for architecture: `docs/architecture/ARCHITECTURE-SPINE.md`.
Backlog and ownership conventions: `docs/product/backlog.md`.

## Dependency rules (enforced by `cargo test`)

- `boba-domain` depends only on `serde`, `thiserror`, `time`, `ulid`.
- Adapters never depend on each other; cross-adapter work goes through `boba-app`.
- `boba-app` never depends on an adapter or on Tauri.

See `src-tauri/tests/dependency_rules.rs`.

## Contributing

Wave/slice ownership is fixed in `docs/product/backlog.md`. New commands go in
`src-tauri/src/commands/<area>.rs` and append one path per line to
`src-tauri/src/commands/registry.rs` (`.gitattributes` marks it `merge=union`).
