# Manual QA — macOS upgrade keeps Keychain access (ADR-009 §1, §5)

Run on **every macOS release candidate**. There is no macOS e2e in CI, so this is
the check that stands in for it: an auto-update that cannot read `db-key` would
either prompt in the background (a run can't answer) or, worse, look like a fresh
install and mint a new key over the user's data.

**Why it can fail:** macOS Keychain access is bound to the binary's *designated
requirement*. Every build must be signed with the **one persistent project
self-signed certificate** (`identifier "app.bobajobfind" and certificate leaf = H"…"`),
never ad-hoc: an ad-hoc requirement is a cdhash that changes on every build, so
build N+1 would be a stranger to the keychain (ADR-009).

## Before you start

- One macOS machine, one user account, and both artifacts built from the same
  signing identity: **build N** (the previous release) and **build N+1** (this
  release candidate). Different binary hashes are expected and required.
- A test user's data is at risk: use a machine or account you can wipe.
- Keep the two `.app` bundles somewhere outside `/Applications`:

```sh
mkdir -p ~/Desktop/boba-qa && cd ~/Desktop/boba-qa
# N  = the shipped previous release
# N+1 = this release candidate
```

## 1. Check the signing identity of both builds (once per candidate)

```sh
for app in Boba-N.app Boba-N+1.app; do
  codesign -dv --verbose=4 "$app" 2>&1 | grep -E "Identifier|Signature|TeamIdentifier"
done
```

Expected, **both builds**:

- `Identifier=app.bobajobfind`
- a signature that is **not** `adhoc` (CI fails the release on `Signature=adhoc`)
- the same designated requirement for both:

```sh
codesign -d -r- Boba-N.app     # "designated => identifier \"app.bobajobfind\" and certificate leaf = H\"…\""
codesign -d -r- Boba-N+1.app   # exactly the same line
```

If the two requirements differ, stop: the release is broken, not the machine.

## 2. Build N: create data and a key (expect one prompt)

1. Install **build N** (drag to `/Applications`), launch it. Gatekeeper warns on
   first install: right-click → **Open** → **Open** (the documented bypass).
2. Complete onboarding through the first screen that writes data (a profile fact
   is enough), so `boba.db` exists and `db-key` is in the Keychain.
3. The first Keychain access shows **"Boba Job Find wants to use your confidential
   information stored in ... in your keychain"**: enter the login password and
   click **Always Allow**. This is the one-time approval tied to the persistent
   identity.
4. Note the data you stored (a profile headline is enough to see it again), then
   **quit** Boba.

## 3. Install build N+1 over build N

1. Quit Boba, then replace `/Applications/Boba Job Find.app` with **build N+1**
   (drag-and-drop; the same path, a different binary).
2. Launch it.

## 4. Expected result

- **No Keychain prompt** and **no unlock screen**: the store opens with no user
  interaction, and the data stored in step 2 is there.
- Nothing was re-keyed and no second `boba.db` appeared:

```sh
ls -l ~/Library/Application\ Support/app.bobajobfind/           # one boba.db (plus boba.db.kdf only on a passphrase machine)
ls -l ~/Library/Application\ Support/app.bobajobfind/snapshots/  # pre-migration snapshots, if a migration ran
```

- The Keychain still holds exactly one item for the app:

```sh
security find-generic-password -s app.bobajobfind -a db-key   # one entry; the same created_at as in step 2
```

- In `/Applications` (or anywhere in the app), **Settings → Privacy** lists the
  store paths and any snapshot, and the run history shows no `skipped_locked` row
  for this launch.

## 5. If it fails

| Symptom | Cause | What to do |
|---|---|---|
| A Keychain prompt you must answer, then the data is there | The designated requirement changed (a re-signed or differently signed build) | The release is broken: rebuild both artifacts with the persistent certificate |
| Unlock screen instead of your data | The key is unreadable for this binary; ad-hoc signature, or a different signing identity | Same as above. The data is **not** lost: never continue with "Start fresh" — the store stays locked until the key is readable (ADR-009 §2) |
| Empty app, no prompt | A fresh install path/data dir was used (e.g. a different bundle identifier) | Verify `Identifier=app.bobajobfind` on both builds |

`tauri dev` builds are excluded on purpose: they use the separate keychain service
`app.bobajobfind.dev` and a separate data directory, so local runs never touch a
real store (ADR-009 §1).

## 6. Record the result

Append to the release notes: candidate version, the two `codesign -dv` outputs, the
one-line result (**store opens unattended: yes/no**), and who ran it. A candidate
without this line is not released (ADR-009 §5).
