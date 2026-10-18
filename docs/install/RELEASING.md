# Releasing (maintainers)

The pipeline is `.github/workflows/release.yml` (S10.5, AD-22). A `v*` tag
builds every installer, signs the updater artifacts, verifies the macOS
signature, and publishes the release with `latest.json`. Nothing runs on a push
to `main`, and the CI workflow never publishes.

Since REL3 the release run starts by calling the whole CI workflow
(`ci.yml` is reusable via `on: workflow_call`) on the tagged commit, and the
installer matrix only starts if that gate passes. CI and Release therefore never
disagree, and a red `main` can never be released.

## One-time setup

### 1. Updater minisign keypair

```sh
scripts/release/generate-updater-keys.sh --out ~/boba-updater
```

Keep `boba-updater.key` and `boba-updater.password` in an offline maintainer
backup, and store them as repository secrets:

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | the file contents of `boba-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password in `boba-updater.password` |

The public key is already committed in `tauri.conf.json`
(`plugins.updater.pubkey`). If you ever regenerate the keypair you must commit
the new public key **and** ship one manual reinstall: installed apps cannot
verify artifacts signed by a key they do not know.

### 2. macOS signing identity

```sh
scripts/release/generate-macos-identity.sh --out ~/boba-signing
```

This writes the self-signed code-signing certificate, its key, a
password-protected `.p12`, and pins the certificate leaf in
`scripts/release/macos-identity.pin` (commit that file). Store:

| Secret | Value |
|---|---|
| `MACOS_CERTIFICATE` | `openssl base64 -A -in ~/boba-signing/boba-signing.p12` |
| `MACOS_CERTIFICATE_PASSWORD` | the password in `boba-signing.password` |

Keep the key and password offline. The identity is **persistent by design**
(ADR-009): its designated requirement is what lets the macOS Keychain key
survive updates. Losing it forces every macOS user through one Keychain
re-approval; replacing it changes the pin, and the release guard must be updated
in the same commit.

### 3. Optional: bundled email clients

The BYO client path works without these; with one set, the app offers the
project's client too. They are compiled in (`option_env!` in `boba-mail`), so
they only reach users through a release build. Register:

| Client | Secret(s) | Registration |
|---|---|---|
| Microsoft Graph | `BOBA_GRAPH_CLIENT_ID` | Entra app registration, public client, loopback redirect, offline_access |
| Gmail | `BOBA_GMAIL_CLIENT_ID`, `BOBA_GMAIL_CLIENT_SECRET` | Google Cloud OAuth **installed app** (the "secret" is public by design for installed apps) |

The release workflow passes all three to the build; an unset (or empty) secret
means the bundled path is unavailable and the UI offers BYO instead. Client ids
are public by definition; the Gmail client secret is only "secret" in name, but
keep it in repository secrets anyway rather than in the tree.

### 4. Protect the signing secrets (required once the repository is public)

Anyone who can push a `v*` tag triggers `release.yml`, which holds the signing
identity and the updater key. Restrict tag creation to maintainers:

- **Settings → Rules → Rulesets → New ruleset → Target: Tags**, pattern `v*`,
  "Restrict creations" and "Restrict updates" to the maintainers team; or
- put `MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`,
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in a
  GitHub **environment** named `release` with required reviewers, and add
  `environment: release` to the `guard`/`build` jobs.

The updater endpoint must name the publishing repository: the release guard
(`scripts/release/verify-updater-endpoint.mjs`) compares
`tauri.conf.json → plugins.updater.endpoints[0]` with `GITHUB_REPOSITORY` on
every CI run and every release, so a repository move fails loudly instead of
serving a 404 to installed apps.

### 5. Windows signing (future)

The project applies to [SignPath Foundation](https://signpath.org/) for free
Authenticode signing. Until it is accepted, Windows installers ship unsigned and
the install docs carry the SmartScreen bypass. When it is accepted, add the
SignPath steps to the Windows matrix entry and remove that section from
`docs/install/windows.md`.

## Cutting a release

1. Bump the version in **all three** places (the guard fails otherwise):
   `package.json`, `src-tauri/tauri.conf.json`, `Cargo.toml`
   (`[workspace.package]`).
2. Push to `main` and wait for CI to go green **on that commit**:

   ```sh
   git push origin main
   ```

   The release re-runs the same gate (below), so a red `main` cannot publish —
   but tagging a broken commit only wastes two long pipelines. Wait for green.

3. Tag the commit CI just passed, and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. Watch the workflow. It runs four jobs:
   - **ci** — the entire `ci.yml` matrix (hygiene, web, rust, the three-OS
     build) on the tagged commit (`on: workflow_call`).
   - **guard** — tag/version agreement, updater pubkey, the pin, the secrets.
   - **build** — the matrix, only after `ci` and `guard` pass; the release
     stays a **draft**.
   - **publish** — downloads the draft, verifies every installer and
     `latest.json`, re-checks the signature of the `.app` inside the DMG, then
     publishes (`--latest` for a stable tag, plain publish for a `-beta` tag).

   At most one release run per tag: a second run queues behind the first.

5. Confirm on the release page that `latest.json` is attached and the release is
   not a draft.

A beta (`v0.3.0-beta.1`) is built and published the same way but never becomes
`/releases/latest`, so the updater keeps serving the stable channel.

### The MSI version

The app version stays semver everywhere — the updater, the update guard and the
release guard all read `1.0.0-beta.1` — but WiX (the MSI toolset) accepts only a
numeric `major.minor.patch.build`, with major/minor ≤ 255 and patch/build ≤
65535. The release workflow therefore derives `bundle.windows.wix.version` from
the tag with `scripts/release/wix-version.mjs`, which writes it to
`src-tauri/tauri.windows.conf.json` (generated on the Windows runner, gitignored,
merged by Tauri on Windows only):

| Tag | MSI version |
|---|---|
| `v1.0.0-beta.1` | `1.0.0.1` |
| `v1.0.0-beta.12` | `1.0.0.12` |
| `v1.0.0-rc.2` | `1.0.0.2` |
| `v1.0.0` | `1.0.0.65535` |

The build field is the last numeric part of the pre-release (`-beta` with no
number counts as 1), and a stable release takes the maximum so it always
upgrades its own betas. Nothing to edit per release: the guard fails early if a
tag cannot be represented. To build a beta MSI locally, run the same command
before `pnpm tauri build`:

```sh
node scripts/release/wix-version.mjs v1.0.0-beta.1 --config src-tauri/tauri.windows.conf.json
```

To rebuild an existing tag without moving it: Actions → Release → Run workflow,
enter the tag.

## Local builds (for developers)

- `pnpm tauri dev` needs no signing material and never touches a real store:
  debug builds use the `app.bobajobfind.dev` keychain service and their own
  `<app-data>/dev/` database and snapshots directory (ADR-009). A dev database
  created before S10.5 at `<app-data>/boba.db` is no longer opened by debug
  builds; move it into `dev/` if you want to keep it.
- `pnpm tauri build` **with bundling** needs the updater key because
  `createUpdaterArtifacts` is on: set
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, pass
  `--no-bundle`, or turn updater artifacts off for that build (what CI does for
  its signature-check bundle, since it never holds the key):

  ```sh
  pnpm tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
  ```

  `tauri dev` is unaffected.
- A local bundle is signed with whatever identity your keychain has; only CI
  enforces the pinned self-signed identity, and `check-macos-signature.mjs`
  tells you when a local build does not match it.

## Release notes

Include, every time:

- the version and the platforms;
- macOS: "self-signed, Gatekeeper bypass required" (link `docs/install/macos.md`);
- Windows: "unsigned until SignPath is accepted" (link `docs/install/windows.md`);
- any Keychain-related change. **The first release signed by an Apple Developer
  ID must say** that on first launch the app shows the Locked state and asks for
  one "Always Allow" approval; that is the one-time migration described in
  ADR-009.

## If something fails

- **Guard fails on versions:** fix the three files; the tag must move (delete
  and re-push the tag before the release is public).
- **The `ci` job fails:** nothing was built and nothing was published. Fix
  `main`, delete and re-push the tag before the release is public.
- **Signature gate fails:** the runner did not sign with the pinned identity.
  Check that `MACOS_CERTIFICATE` matches `macos-identity.pin`; never bypass the
  gate — an ad-hoc build breaks Keychain access for every existing user.
- **A bad release is already public:** publish a fixed patch version. Do not
  delete the latest release: `latest.json` is resolved by URL, and installed
  apps may already be mid-update.

## Licence

Boba Job Find is MIT (`LICENSE`), including the upstream
[MadsLorentzen/ai-job-search](https://github.com/MadsLorentzen/ai-job-search)
notice (NFR-20).
