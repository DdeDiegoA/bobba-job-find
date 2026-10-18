# REL4 review (reviewer-1)

Commit reviewed: `76c89d4` (branch `hermad/REL4`), on top of `e9f7e8b`.

**Verdict: APPROVED.**

## Scope

Release run 39091506560, job 117570923420: the Windows build failed at "MSI (WiX)
version from the tag". Fix the empty `$TAG` under pwsh and check that no other
Windows-run step in `release.yml` or `ci.yml` has the same problem.

## Checks

- **Root cause confirmed from the log.** The step printed only the `wix-version.mjs`
  usage line, which is what happens when no tag argument arrives. On
  `windows-latest` a `run` step without `shell:` uses pwsh, where `$TAG` is an
  unset PowerShell variable (the env var is `$env:TAG`). With `TAG` set, the same
  command under bash prints `1.0.0.1`.
- **Fix covers the job.** `defaults.run.shell: bash` is set on the `build` job in
  both `release.yml` and `ci.yml`. It applies to every `run` step in that job,
  including the Windows leg. Action steps (`tauri-apps/tauri-action`,
  `dtolnay/rust-toolchain`, `Swatinem/rust-cache`) are unaffected.
- **Other Windows-run steps audited.** `release.yml`: "Install" and "Build the
  boba-mcp sidecar" use no shell variables (the sidecar step passed in the log).
  `ci.yml` Windows leg: "MCP proto tests", "Install" and "Tauri build" use no shell
  variables. The shell-variable steps in both files are macOS-only, Linux-only or in
  the `guard`/`publish` jobs, which run on Ubuntu/macOS and are not affected.
- **Clearer failure.** `wix-version.mjs` now prints `missing tag argument` before the
  usage line, and the new CLI tests cover the missing-tag and happy paths.
- **Gates run in the worktree at `76c89d4`:**
  - `scripts/release/lib/wix-version.test.mjs`: 13/13 pass (vitest).
  - `node scripts/release/wix-version.mjs` with no tag: clear error, exit 1.
  - `node scripts/release/wix-version.mjs v1.0.0-beta.1`: `1.0.0.1`.
  - `pnpm lint`: exit 0. The 3 warnings are `noExplicitAny` in
    `src/lib/ipc/bindings.ts`, which this commit does not touch.
  - `node scripts/ci/check-workflow-policy.mjs`: ok.
  - `node scripts/ci/check-conventions.mjs`: ok.
- **Not run locally:** the Windows runner itself. The pwsh failure is confirmed from
  the log, and `shell: bash` on `windows-latest` is the same Git Bash the other
  Windows steps already used in run 39091506560.

## Notes (non-blocking)

- `check-workflow-policy` has no rule for a Windows-run step that reads `$VAR`
  without `shell: bash`. The job-level default fixes this instance, but a future
  workflow could regress without the check noticing. Consider adding the rule in a
  follow-up.
