# FX6 review: version 1.0.0-beta.3 and CHANGELOG

> **Reviewer:** reviewer-2 (qa) · **Dev:** dev-2 · **Date:** 2026-10-18
> **Branch:** `hermad/FX6` · **Reviewed commit:** `0d6c2c3` (`chore(release): bump version to 1.0.0-beta.3`), on main `a365040`
> **Verdict:** APPROVED at `0d6c2c3`

## Version fields

| Field | Value | Result |
|---|---|---|
| `Cargo.toml:22` (`[workspace.package]`) | `1.0.0-beta.3` | ok |
| `package.json:4` | `1.0.0-beta.3` | ok |
| `src-tauri/tauri.conf.json:4` | `1.0.0-beta.3` | ok |
| `node scripts/release/check-version.mjs v1.0.0-beta.3` | `version ok`, exit 0 | ok |
| `node scripts/release/wix-version.mjs v1.0.0-beta.3` | `1.0.0.3` | ok |

No `beta.2` reference is left in the version fields. The only remaining `beta.2`
mentions are the CHANGELOG's own beta.2 section and historical review docs.

## Cargo.lock

The diff changes exactly 13 `version` lines, all from `1.0.0-beta.2` to
`1.0.0-beta.3`. Every changed entry is a workspace crate: `boba-agent`,
`boba-app`, `boba-browser`, `boba-docs`, `boba-domain`, `boba-embed`,
`boba-fake-cli`, `boba-job-find`, `boba-mail`, `boba-mcp`, `boba-mcp-proto`,
`boba-sources`, `boba-store`. No third-party entry moved. `cargo test --locked`
and `cargo clippy --locked` both pass, so the lock matches the manifests.

## CHANGELOG against `git log --merges 83d95d5..main`

The range has 8 merges. Each one appears in the beta.3 section:

| Merge | CHANGELOG entry |
|---|---|
| FX5b | Added: email one-click setup and own-client guide |
| FX13 | Added: Needs you panel on Today, Disclosures in Settings |
| FX14 | Added: launch-at-login switch and finish step |
| FX17 | Internal: orphan tracker board removed |
| FX18 | Internal: `check-conventions` unmounted-leaf guard, route container tests |
| FX19 | Fixed: test suite robust under load |
| FX20 | Fixed: boba-mcp shim test no longer races the sidecar copy |
| FX21 | Release pipeline: publish verifies draft, publishes, then verifies |

Spot checks against the code:

- `email_bundled_clients` exists in `src-tauri/src/ipc/command_list.rs` and in the
  email connection and onboarding code, so the FX5b "hides itself" claim is backed.
- `scripts/release/workflow-order.test.mjs` covers the FX21 publish ordering,
  and it passes.
- `docs/user-guide/email-own-client.md` exists, as the CHANGELOG links.

Non-blocking: the CHANGELOG has no `Unreleased` section and does not need one
for this tag.

## Gate (worktree `hermad/FX6` at `0d6c2c3`, CARGO_INCREMENTAL=0)

| Check | Result |
|---|---|
| `pnpm lint` | exit 0 (3 warnings, all `noExplicitAny` in generated `bindings.ts`, untouched) |
| `pnpm typecheck` | exit 0 |
| `pnpm test` | exit 0, 87 files, 704 tests (includes `scripts/` tests) |
| `node scripts/check-i18n.mjs` | exit 0 |
| `node scripts/ci/check-conventions.mjs` | exit 0 |
| `node scripts/ci/check-workflow-policy.mjs` | exit 0 |
| `cargo test --workspace --locked` | exit 0 |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `node scripts/gen-bindings.mjs --check` | exit 0, bindings up to date |
| `cargo deny check` | exit 0 (advisories, bans, licenses, sources ok) |

## Notes (not blocking)

- The `scripts/release/workflow-order.test.mjs` file is a vitest suite. Run it as
  `vitest run`, not `node --test`. The repo's `pnpm test` already runs it.
