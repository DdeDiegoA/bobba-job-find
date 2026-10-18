# REL2 review (reviewer-1)

Commit reviewed: `0f481ad` (branch `hermad/REL2`), on top of `298b946`.

**Verdict: APPROVED.**

## Scope

Fix the release workflow so run 39087228193's two failures are gone:

1. Windows MSI rejected `1.0.0-beta.1` (WiX needs a numeric version).
2. macOS signature gate found no `.app` at
   `target/universal-apple-darwin/release/bundle/macos/Boba Job Find.app`.

## Checks

- **Log root causes confirmed.** The MSI error is "optional pre-release identifier
  in app version must be numeric-only". The macOS gate reports "no app bundle at
  ...". The matrix had `bundles: dmg` only, so the bundler removed the `.app`.
- **Tauri config.** `config.schema.json` documents that
  `tauri.windows.conf.json` is merged on Windows only. It also defines
  `bundle.windows.wix.version` as the MSI-only override. The app `version` stays
  `1.0.0-beta.1` in `tauri.conf.json` and `package.json`, so the updater and guard
  still read semver.
- **Mapping.** `wix-version.mjs` gives `v1.0.0-beta.1` → `1.0.0.1`,
  `v1.0.0-beta.12` → `1.0.0.12`, and `v1.0.0` → `1.0.0.65535`. Out-of-range tags
  fail early, as tested with `v1.0.0-beta.70000`.
- **Generated file.** `src-tauri/tauri.windows.conf.json` is gitignored. It is
  written only in the Windows release job, so the working tree stays clean.
- **macOS.** `app,dmg` keeps the `.app` for the ADR-009 gate and the updater
  signature. The gate's `.app` path now matches the bundler output.
- **Tests.** `vitest run scripts/release/lib/wix-version.test.mjs` passes 11/11.
  `pnpm test` passes 580/580. `check-workflow-policy` and `check-conventions` pass.
  `pnpm lint` has 3 pre-existing warnings in generated `bindings.ts`, which this
  change does not touch.

## Non-blocking notes

- `v1.0.0-beta.1` and `v1.0.0-rc.1` both map to `1.0.0.1`. A `-rc` and a `-beta`
  of the same version would share an MSI version. Low risk, since only one
  pre-release of a version ships at a time. Consider a note in RELEASING.md if
  rc tags get used.
- The Linux/Windows asset-glob audit relies on dev-1's reading of run logs. I did
  not re-verify it against the run artifacts.
