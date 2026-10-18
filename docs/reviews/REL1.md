# REL1 review: version bump to 1.0.0-beta.1, CHANGELOG, updater repo slug

**Reviewer:** reviewer-1 · **Commits reviewed:** `79e71ce` (version bump, CHANGELOG), `ca363a3` (repo slug and guard)
**Verdict:** APPROVED at `ca363a3`. Earlier BLOCKED verdict (`68c3613`) is superseded: the endpoint now names the release repo Diego chose, `DdeDiegoA/bobba-job-find`.

## Version fields (release.yml guard)

| Field | Value | Result |
|---|---|---|
| `package.json` `version` | `1.0.0-beta.1` | ok |
| `src-tauri/tauri.conf.json` `version` | `1.0.0-beta.1` | ok |
| `Cargo.toml` `[workspace.package] version` | `1.0.0-beta.1` | ok |
| `Cargo.lock` workspace crates | all at `1.0.0-beta.1`, none at `0.1.0` | ok, regenerated |
| `check-version.mjs v1.0.0-beta.1` | matches all three | ok |

## Updater endpoint and repo slug

- `tauri.conf.json` endpoint: `https://github.com/DdeDiegoA/bobba-job-find/releases/latest/download/latest.json`.
- Same slug in `Cargo.toml` `repository`, the `boba-sources` `REPO_URL` and the `boba-mail` user agent, and in `docs/install/{README,macos,windows,updates}.md`. No live `diegoarenas/boba-job-find` references remain. The only hits are in historical reviews (`S10.5`, `FD2`) and this file.
- `release.yml` now passes `--repo "${{ github.repository }}"`. That is the same value as `GITHUB_REPOSITORY`, so the CI guard compares against the publishing repo.
- `verify-updater-endpoint.mjs` locally falls back to `origin` when no `--repo` or `GITHUB_REPOSITORY` is set. `repoFromRemote` handles the ssh, ssh:// and https forms, and returns null for non-GitHub remotes. Unit tests cover all of these.
- Run with `GITHUB_REPOSITORY=DdeDiegoA/bobba-job-find`: `updater endpoint ok`.
- `verify-updater-pubkey.mjs`: ok.
- macOS identity pin and secrets check: not run here (CI-only secrets).

## CHANGELOG accuracy

- `[1.0.0-beta.1] - 2026-10-18`, marked as a pre-release that never becomes `/releases/latest`: accurate.
- Known limitations match the install docs: macOS self-signed (Gatekeeper bypass), Windows unsigned (SmartScreen), and BYO OAuth client. The bundled Graph and Gmail client IDs are `option_env!` at build time, so "not registered yet" is accurate.
- "Fixes after v1" entries cite CH15–CH18, which are merged in `main`.

## Gate (in `.hermad/worktrees/REL1`, `CARGO_INCREMENTAL=0`, at `ca363a3`)

- `pnpm install --frozen-lockfile --ignore-scripts`: ok
- `pnpm lint`: ok, 3 pre-existing `noExplicitAny` warnings in Tauri event mapping
- `pnpm typecheck`: ok
- `pnpm test`: 77 files, 569 tests passed (including the endpoint guard tests)
- `check-i18n`, `check-conventions`, `check-workflow-policy`: ok
- `gen-bindings --check`: ok
- `cargo fmt --all --check`: ok
- `cargo clippy --workspace --all-targets --locked -D warnings`: ok
- `cargo test --workspace --locked`: all suites pass, 0 failures
- `cargo deny check`: ok
- Note: the test build prints a `__eh_frame section too large` linker warning. It is non-fatal and was also present on the `79e71ce` run.

## Release reminders (not blockers for this commit)

- `R-S10.5-F5` should be closed in `docs/security/residuals.md`. The endpoint now matches the chosen repo, but the residual still says "unconfirmed".
- The first tag must be a `-beta` pre-release, per the release checklist.
