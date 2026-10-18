# REL3 review (reviewer-1)

Commit reviewed: `7096a45` (branch `hermad/REL3`), on top of `edfd905`.

**Verdict: CHANGES REQUESTED** (2 findings, 1 medium, 1 low). The CI fix and the
release gate are correct; the findings are in the new `concurrency` group and in
a comment that states something false.

## Scope

1. CI macOS signed bundle must not need `TAURI_SIGNING_PRIVATE_KEY` (run
   39089412177, job 117563447472).
2. Release builds gated on the full CI workflow, via `ci.yml` `on: workflow_call`.

## Checks

- **Root cause confirmed from the log.** The macOS job built the `.app` and then
  failed with "A public key has been found, but no private key. Make sure to set
  `TAURI_SIGNING_PRIVATE_KEY`". Only the updater tarball needs that key, so
  turning off `createUpdaterArtifacts` for the ADR-009 check bundle fixes it.
- **Tauri override.** `--config '{"bundle":{"createUpdaterArtifacts":false}}'` is
  a valid inline JSON override for Tauri v2. It applies only to the CI bundle
  step. The release matrix in `release.yml` still builds updater artifacts and
  still receives the minisign key.
- **Pinned identity check unchanged.** `check-macos-signature.mjs` still runs on
  the `.app`.
- **Secrets scope.** `release.yml` `ci` job has no `secrets:` and no
  `secrets: inherit`. `ci.yml` reads only `MACOS_CERTIFICATE` and
  `MACOS_CERTIFICATE_PASSWORD`, which arrive empty, so the import takes the
  "not configured" path. The minisign key never reaches the CI gate.
- **Needs graph.** `build` needs `[guard, ci]`, and `publish` needs
  `[guard, build]`, so publish is transitively gated on CI. No path builds or
  publishes without `ci` passing.
- **Permissions.** `ci.yml` requests `contents: read`. The caller grants
  `contents: write`, which covers it, so the called workflow cannot fail on
  permissions.
- **Pinned SHAs.** The only new `uses:` is the local `./.github/workflows/ci.yml`,
  which the policy does not pin. No third-party action changed.
- **Policy.** `node scripts/ci/check-workflow-policy.mjs` passes ("actions pinned,
  no board/LLM endpoints"). `node scripts/ci/check-conventions.mjs` passes.
- **Not run.** `actionlint` is not installed on this machine, and the `pnpm`
  gate was not run in the worktree. The YAML was checked by reading it.

## Findings

### F1 (medium): shared `concurrency` group can cancel a release's CI gate

`ci.yml` sets `group: ci-${{ github.workflow }}-${{ github.ref }}` with
`cancel-in-progress: true`. When `release.yml` calls it, `github.workflow` is the
caller's name, `Release`. Its per-tag group does not cover this inner group.

Failure scenario: a maintainer dispatches `Release` for `v1.0.0-beta.2` and then
for `v1.0.0-beta.3`, both from `main`. `workflow_dispatch` runs use
`refs/heads/main`, so both `ci` calls share `ci-Release-refs/heads/main`. The
second call cancels the first call's gate. The first run's `build` is skipped and
the run fails. Nothing unsafe is published (the failure is closed), but the
release is blocked for no reason.

Fix: stop inner cancellation for release callers, for example
`cancel-in-progress: ${{ github.event_name == 'pull_request' }}`, or key the group
on `github.run_id` when `workflow_call` is the trigger. Confirm with a test
dispatch.

### F2 (low): wrong claim in the macOS sign-step comment and in the docs

`ci.yml` (sign-step comment) says the gate's macOS signing is skipped because
"no `push` event in this run". A tag push is a `push` event. The condition
`github.event_name == 'push'` is true inside the release gate. The step then
takes the "not configured" notice path only because `MACOS_CERTIFICATE` arrives
empty. The behaviour is safe, but the comment is false and the release gate does
not run the pinned-identity check that `main` runs.

`RELEASING.md` also says "the release re-runs the same gate". It does not, on the
macOS signing step.

Fix: correct the comment to say the step runs on tag pushes and takes the
"not configured" path because no secrets are passed. Change the docs to say the
release gate is the same matrix without the macOS signing identity check.

## Notes (no action)

- The CI gate on a tag still runs the full hygiene, web, rust and e2e matrix, so
  the release cost is about one extra CI run per tag. This is intended.
- `publish` still needs `guard` directly, so the guard checks are unaffected.
