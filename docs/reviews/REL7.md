# REL7 review (reviewer-1)

Commits reviewed: `992b4bb` (version bump to 1.0.0-beta.2 and CHANGELOG) and
`ea5aa11` (platform-aware FIX1 tests, requested by the orquestador after CI run
39104117414 failed on Linux). Base: `593feda`.

**Verdict: CHANGES REQUESTED** on the CHANGELOG (`992b4bb`). The version, lock and
WiX work is correct. The `ea5aa11` test fix is correct on Linux, with one wrong
comment and one unverified root cause.

## Scope

Bump the workspace, `package.json`, `tauri.conf.json` and `Cargo.lock` to
`1.0.0-beta.2`, add the beta.2 CHANGELOG section, and fix two FIX1 tests that
fail on Linux CI.

## Checks: version and lock (992b4bb)

- **Every guarded field agrees.** `package.json:4`, `src-tauri/tauri.conf.json:4`
  and `Cargo.toml:22` (`[workspace.package]`) are all `1.0.0-beta.2`.
  `node scripts/release/check-version.mjs v1.0.0-beta.2` prints `version ok`
  and exits 0. Against `v1.0.0-beta.1` it fails on all three fields, as it should.
- **Lock regenerated, not hand-edited.** `Cargo.lock` changes are only the 13
  workspace crate `version` lines (`boba-*`, `boba-job-find`). No third-party
  entry moved. `cargo metadata --locked` exits 0.
- **WiX mapping distinct per beta.** `wix-version.mjs v1.0.0-beta.2` prints
  `1.0.0.2`; `v1.0.0-beta.1` prints `1.0.0.1`. The MSI version is numeric and
  unique per beta, so the Windows upgrade path keeps working. The release
  workflow generates it from the tag and does not commit it.
- **No stray beta.1 in shipped config.** A grep of `package.json`,
  `tauri.conf.json`, `Cargo.toml` and `.github/` finds only the version strings
  above. The remaining `1.0.0-beta.1` hits are test fixtures
  (`scripts/release/lib/*.test.mjs`) and doc history, which is expected.
- **Nit (not blocking).** `scripts/release/README.md:11` still says the WiX
  mapping cannot carry `1.0.0-beta.1`. The mapping itself works for any beta,
  so the wording should be generic.

## Findings: CHANGELOG (992b4bb)

- **Wrong REL attribution (change request).** The CHANGELOG says "Releases run
  only after the CI gate passes (REL2)". The gate was added in REL3:
  `7096a45` and `0d4f4ca` ("gate installers on CI"). Also "`latest.json` points
  at public release asset URLs (REL3)" is REL5: `51f5d4b`. Fix both tags.
- **Wrong install command (change request).** The CHANGELOG says the quarantine
  path is `xattr -d com.apple.quarantine`. The install docs use
  `xattr -dr com.apple.quarantine "/Applications/Boba Job Find.app"`
  (`docs/install/macos.md:16`, `docs/user-guide/troubleshooting.md:13`). Without
  `-r` the flag is not removed from the app's contents. Use the documented form.
- **Overstated FIX1 claim (change request).** "Test connection ... falls back to
  Limited mode instead of failing the setup wizard." The engine step offers
  Limited mode as the way on after a failure (`engine.test.tsx:212`, "offers
  Limited mode as the way on"). It is not an automatic fallback. Reword to
  "offers Limited mode".
- **Accurate, verified.** The DMG licence fix (`28a3d8b`), the MSI numeric
  version (`0f481ad`, REL2), the Windows bash shell (`cda98cb`, REL4), and the
  FIX1 login-shell PATH plus `~/.opencode/bin` (`31d04c1`, `388878d`).

## Findings: `ea5aa11` (Linux test fix)

- **`known_locations` test (correct).** The `~/Library/pnpm` assertion is now
  under `cfg(target_os = "macos")`, matching the production `cfg` at
  `detect/mod.rs:343`. Reproduced in a Linux container (`rust:1.92-bookworm`)
  at `992b4bb`: fails as CI did (`assertion failed: dirs.contains(&home.join("Library/pnpm"))`).
  At `ea5aa11`: 5 of 5 runs pass.
- **Scope note (not blocking, but say so in the CHANGELOG or split it).**
  `ea5aa11` also adds two production search paths: `~/.local/share/pnpm` on
  Linux and `%LOCALAPPDATA%\pnpm` on Windows. These are the pnpm `PNPM_HOME`
  defaults and are tested, but they are behaviour changes, not a test fix. The
  CHANGELOG FIX1 bullet should mention them, or they should move to their own
  commit.
- **Login-shell tests: fixture is hermetic, stated cause is wrong (change
  request on the comment).** The fake shell now answers from a hardcoded string
  and matches `" $* "` for `-ilc`. It does not read the host's `$SHELL`, `$HOME`
  or rc files, so it is hermetic, and the `-ilc` check still proves the
  interactive flag is passed. The comment is wrong, though: a `#!/bin/sh` script
  gets `-ilc` as `$1` under every shell, so "the positional parameters depend on
  the interpreter" is not a real cause. Remove that claim.
- **Login-shell root cause not established.** In the clean Linux container the
  original `login_shell_path_*` tests passed. The CI failure shows `left:
  "/usr/bin:/bin"`, which means the `-ilc` run returned nothing usable and the
  `-lc` run answered. A plausible cause is a spawn race on Linux: a fake
  executable written while another test forks can hit `ETXTBSY`, which makes the
  first spawn fail and sends the probe to `-lc`. The `case` change would not fix
  that. Not proven here. Either show the `-ilc` failure from the CI log or fix
  the fixture against ETXTBSY (for example retry the spawn on `ETXTBSY`). Rerun
  CI on `ea5aa11` and watch the login-shell tests for flakes.

## Gates run

On `992b4bb` (full CI gate, CARGO_INCREMENTAL=0):

- `pnpm lint`, `pnpm typecheck`: exit 0 (3 warnings in generated
  `src/lib/ipc/bindings.ts`, which this diff does not touch). `pnpm test`: 79
  files, 602 tests passed.
- `check-i18n`, `check-conventions`, `check-workflow-policy`, `gen-bindings
  --check`: ok.
- `pnpm vitest run scripts/release`: 7 files, 81 tests passed.
- `cargo fmt --all --check`: exit 0. `cargo metadata --locked`: exit 0.
- `cargo clippy --workspace --all-targets --locked -D warnings`: exit 0.
- `cargo test --workspace --locked`: all suites ok, 0 failed.
- `cargo deny check`: advisories, bans, licenses, sources ok.

On `ea5aa11` (macOS):

- `cargo fmt --all --check`: exit 0. `cargo clippy -p boba-agent --all-targets
  --locked -D warnings`: finished without errors.
- `cargo test -p boba-agent --locked`: 131 + 8 passed, 0 failed.

Linux container (`rust:1.92-bookworm`, `cargo test -p boba-agent --lib detect
--locked`): `ea5aa11` 5 of 5 passed. `992b4bb` reproduced the CI failure.

**Not run:** the Windows `#[cfg(windows)]` branches (no Windows toolchain
here); a real Linux CI rerun on the REL7 commit (only the orquestador can
trigger it); `pnpm test` and the workspace gate on `ea5aa11` (the diff touches
only `detect/mod.rs`, and the boba-agent gate above covers it).

## Required before approval

1. Fix the CHANGELOG REL attributions (REL3 for the CI gate, REL5 for the
   `latest.json` URLs).
2. Use the documented `xattr -dr ... "/Applications/Boba Job Find.app"` form.
3. Reword "falls back to Limited mode" to "offers Limited mode".
4. Remove the interpreter-dependent claim from the login-shell comments.
5. Either show the CI `-ilc` evidence or harden the fixture against the spawn
   race. Then rerun CI on the REL7 commit.
6. Mention the pnpm search paths in the FIX1 CHANGELOG bullet, or split them out.
