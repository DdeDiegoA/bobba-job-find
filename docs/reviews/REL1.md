# REL1 review: version bump to 1.0.0-beta.1 and CHANGELOG

**Reviewer:** reviewer-1 · **Commit reviewed:** `79e71ce` (`chore(release): bump to 1.0.0-beta.1`)
**Verdict:** BLOCKED. The version bump and gate are clean. The updater endpoint guard fails against this checkout's origin, and the release repository is not confirmed. Diego must decide before the tag.

## Version fields (release.yml guard)

| Field | Value | Result |
|---|---|---|
| `package.json` `version` | `1.0.0-beta.1` | ok |
| `src-tauri/tauri.conf.json` `version` | `1.0.0-beta.1` | ok |
| `Cargo.toml` `[workspace.package] version` | `1.0.0-beta.1` | ok |
| `Cargo.lock` workspace crates | 13 entries at `1.0.0-beta.1`, 0 at `0.1.0` | ok, regenerated |
| `check-version.mjs v1.0.0-beta.1` | matches all three | ok |

## Other release guards

- `verify-updater-pubkey.mjs`: ok.
- `verify-updater-endpoint.mjs`: **fails**. Output: `the updater endpoint points at diegoarenas/boba-job-find, but this repository is DdeDiegoA/bobba-job-find`. `origin` is `git@github.com:DdeDiegoA/bobba-job-find.git`. In CI, `GITHUB_REPOSITORY` is the slug the tag is pushed to. If that is `DdeDiegoA/bobba-job-find`, the guard fails before the build. If it is `diegoarenas/boba-job-find`, the local origin is wrong. Either way the endpoint and the publishing repo disagree. This is the open residual `R-S10.5-F5`.
- macOS identity pin and secrets check: not run here (CI-only secrets).

## CHANGELOG accuracy

- `[1.0.0-beta.1] - 2026-10-18` with the pre-release note: accurate.
- Known limitations: macOS self-signed and Gatekeeper bypass, Windows unsigned with SmartScreen, and BYO OAuth client. These match `docs/install/RELEASING.md`, `macos.md`, `windows.md` and `README.md`. The bundled Graph and Gmail client IDs are `option_env!` at build time (`boba-mail`), so "not registered yet" is accurate. All linked docs exist.
- The new "Fixes after v1" entries cite CH15–CH18, which are merged in `main`.

## Gate (in `.hermad/worktrees/REL1`, `CARGO_INCREMENTAL=0`)

- `pnpm install --frozen-lockfile --ignore-scripts`: ok
- `pnpm lint`: ok, 3 pre-existing warnings (`noExplicitAny` in Tauri event mapping). Not introduced by REL1.
- `pnpm typecheck`: ok
- `pnpm test`: 77 files, 567 tests passed
- `check-i18n`, `check-conventions`, `check-workflow-policy`: ok
- `gen-bindings --check`: ok
- `cargo fmt --all --check`: ok
- `cargo clippy --workspace --all-targets --locked -D warnings`: ok
- `cargo test --workspace --locked`: all suites pass, 0 failures
- `cargo deny check`: advisories, bans, licenses, sources ok

## Required before tagging

1. Diego confirms the release repository (`diegoarenas/boba-job-find` or `DdeDiegoA/bobba-job-find`).
2. Set `tauri.conf.json` `plugins.updater.endpoints[0]`, `Cargo.toml` `repository`, and the `docs/install` links to that slug. Then re-run `verify-updater-endpoint.mjs` with `GITHUB_REPOSITORY` set.
3. Close `R-S10.5-F5` in `docs/security/residuals.md`.
