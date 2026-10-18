# REL6 review (reviewer-1)

Commits reviewed: `3a189db` (config, workflow, docs) and `1dc4564` (troubleshooting
and QA doc fix, rebased onto `main` at `ab434cb`). Approved commit: `1dc4564`.

**Verdict: APPROVED** (after the change request on `3a189db` was fixed in `1dc4564`).

## Scope

Remove the DMG software licence agreement (`bundle.licenseFile`) so the DMG
mounts without a prompt, keep the MIT LICENSE in the app resources, make the
`hdiutil attach` in `release.yml` robust without a tty, and document the macOS
quarantine install path.

## Checks

- **Config key.** `src-tauri/tauri.conf.json:42` now reads
  `"resources": { "../LICENSE": "LICENSE" }`. The `licenseFile` key is gone, and
  the JSON parses. The map form is the Tauri v2 `bundle.resources` mapping
  (source to target). The local Tauri CLI schema is not installed, so I did not
  check the schema file itself; the commit's debug-bundle check is the evidence
  for the map form and was not re-run here.
- **Licence still ships.** `LICENSE` is at the repo root, 2301 bytes, and the
  mapping copies it to `Contents/Resources/LICENSE`. The commit says the bundled
  file is byte-identical to the root; not re-verified (no bundle built).
- **Release workflow.** `release.yml:291` is
  `hdiutil attach "$dmg" -nobrowse -noverify -noautoopen -readonly -mountpoint "$mnt" >/dev/null`,
  with the same trap detach and `check-macos-signature.mjs` call. `-noverify`
  skips only the image checksum, and the signature gate still runs on the mounted
  `.app`. Adequate.
- **Signature docs match the checker.** The doc's expected designated requirement
  (`identifier "app.bobajobfind" and certificate root = H"8f27…"`) matches
  `scripts/release/lib/macos-signature.mjs` for a self-signed identity (anchor
  `root`) and the pin `scripts/release/macos-identity.pin`. The doc links to the
  pin path correctly (`../../scripts/release/macos-identity.pin` from
  `docs/install/`).
- **README and install docs.** The `xattr -dr` command, the "right-click → Open
  no longer bypasses" claim, and the `spctl` "rejected" note are consistent with
  each other. The internal links resolve (`docs/install/macos.md`, `README.md`
  links to `docs/install/README.md`).
- **Troubleshooting fixed.** `1dc4564` rewrites `docs/user-guide/troubleshooting.md`
  to copy-to-Applications, then `xattr -dr`, then open. "Open Anyway" is kept only
  for a warning that offers it. `docs/qa/macos-upgrade-test.md` step 1 no longer
  says right-click → Open. A grep of `README.md` and `docs/` finds no remaining
  `xattr -d` or stale Open Anyway instruction.
- **Gates on `1dc4564` (full CI gate, run here).**
  - `pnpm lint`, `pnpm typecheck`: exit 0. `pnpm test`: 602/602.
  - `check-i18n`, `check-conventions`, `check-workflow-policy`: ok.
  - `gen-bindings --check`: ok. `cargo fmt --all --check`: ok.
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`: exit 0.
  - `cargo test --workspace --locked`: 2147 passed, 0 failed.
  - `cargo deny check`: advisories, bans, licenses, sources ok (warnings only).
  - `pnpm vitest run scripts/release`: 7 files, 81/81 (from `3a189db`; no release
    script changed since).
- **Not run:** a real macOS release DMG mount in CI. The local debug-bundle attach
  (`-noverify -noautoopen -readonly`, stdin `/dev/null`) is dev's evidence: rc=0,
  mounted, `Contents/Resources/LICENSE` byte-identical to the repo root. Not
  re-run here.

## Findings

None blocking. The change request on `3a189db` (stale troubleshooting guide) is
resolved in `1dc4564`.

## Notes (non-blocking)

- `docs/install/RELEASING.md:199` and `docs/security/residuals.md:108` still say
  "Gatekeeper bypass". That is still broadly true (the quarantine step is a
  bypass), so it is not wrong, but the wording could follow the new path later.
- `docs/install/macos.md` links to the repo as `DdeDiegoA/bobba-job-find`, a
  pre-existing spelling that is not part of this change. Left as is.
