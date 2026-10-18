# REL5 review (reviewer-1)

Commit reviewed: `51f5d4b` (branch `hermad/REL5`), on top of `59ff527`.

**Verdict: APPROVED.**

## Scope

Release run 39094995728, job 117586968853: "Installers and latest.json" failed
because `latest.json` pointed at draft asset API ids (`releases/assets/<id>`)
that are not published assets. Fix the urls to the public
`releases/download/<tag>/<file>` form before verify and publish, without touching
signatures or any other field, and fail if an asset is missing.

## Checks

- **Root cause confirmed from the log.** Four platforms (`darwin-aarch64`,
  `darwin-x86_64`, `windows-x86_64`, `linux-x86_64`) report
  `points at 652477145 ...` (asset ids), and the release is still a draft.
- **Live draft, read-only.** `gh release view v1.0.0-beta.1 --json assets` exposes
  `apiUrl` ending in the numeric id that `latest.json` uses, so the id-to-name
  mapping resolves on a draft. Downloaded `latest.json` (13 platform entries) and
  ran `fix-latest-json.mjs` on a copy in scratchpad: every `signature` identical,
  all non-platform fields identical, all 13 urls now public download urls.
- **Missing asset fails.** Unknown asset id (`.../assets/1`): `points at asset 1,
  which is not in release`, exit 1. Public url naming an absent file
  (`Missing.tar.gz`): `which is not an asset of release`, exit 1.
- **Verify logic for a draft.** `checkLatestJson` with `repo` passes on the fixed
  file and rejects the original draft urls (`not a public download URL`). The
  draft check still runs: `release.isDraft` is evaluated from the same
  `gh release view` call that lists the assets.
- **Workflow order.** In the `publish` job: download (250) → fix + `gh release
  upload --clobber` (262-269) → verify (274) → publish `gh release edit --draft=false`
  (293). Fix runs before verify and before publish. Only `latest.json` is
  re-uploaded.
- **Policy and conventions.** `check-workflow-policy` ok, `check-conventions` ok.
  `GH_TOKEN` is scoped to the two steps that call `gh`.
- **Tests.** `pnpm vitest run scripts/release`: 7 files, 81/81. Full gate on the
  REL5 tree: `pnpm lint` exit 0, `pnpm typecheck` exit 0, `pnpm test` 79 files /
  599 tests pass, `check-i18n` ok, `gen-bindings --check` up to date.
- **Not run:** the live release workflow itself. The re-upload and publish happen
  only in CI. I did not modify the live release (read-only downloads and `gh
  release view` only).

## Notes (non-blocking)

- `verify-release.mjs` calls `gh release view` without `--repo`. It works in CI
  because the checkout's remote resolves the repo. `fix-latest-json.mjs` passes
  `--repo`. Consider passing it to both.
- Verify checks the local `latest.json`, not the copy attached to the release. The
  `--clobber` upload fails loudly on error, so this is acceptable.
- Signatures are copied verbatim and not cross-checked against the `.sig` assets.
  This is unchanged from before REL5, so it is not a REL5 regression.
