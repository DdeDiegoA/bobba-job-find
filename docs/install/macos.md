# macOS

## Install

1. Download `Boba Job Find_<version>_universal.dmg` from the
   [latest release](https://github.com/DdeDiegoA/bobba-job-find/releases/latest).
2. Open the DMG and drag **Boba Job Find** to **Applications**.
3. The app is **signed with the project's persistent self-signed identity, but
   not notarised** (there is no Apple Developer ID at MVP). Gatekeeper will stop
   the first launch. Use one of the two documented paths below.

### Gatekeeper bypass (first launch only)

**System Settings path (recommended, macOS 13 and later):**

1. Open the app once and dismiss the warning ("Apple cannot check it for
   malicious software").
2. Open **System Settings → Privacy & Security**.
3. Scroll to **Security** and press **Open Anyway** next to Boba Job Find.
4. Authenticate, then press **Open** in the confirmation dialog.

**Terminal path (same result):**

```sh
xattr -d com.apple.quarantine "/Applications/Boba Job Find.app"
```

then open the app normally. Only do this for a download you got from this
project's GitHub releases; the command removes the quarantine flag from
whatever app you point it at.

Every later launch opens normally.

## Why this is safe to approve

- The build is signed by one persistent, project-owned certificate, not ad-hoc.
  The same certificate signs every release, so your Keychain entry (and the
  database key inside it) survives every update. If you ever see a Keychain
  prompt after an update, something is wrong: verify the release's
  `codesign` output against the pin in `scripts/release/macos-identity.pin`.
- The release pipeline fails if `codesign -dv` reports an ad-hoc signature or a
  different certificate, so an ad-hoc build cannot be published by accident.
- `tauri dev` and local builds use the separate `app.bobajobfind.dev` Keychain
  service and a separate data directory, so a development build can never read
  or overwrite your real data.

## Uninstall

Drag the app to the Trash. Your database and Keychain entry are not removed;
use **Settings → Privacy** to export a backup first, and **Reset everything**
in the app if you want the data gone.

## Future: Apple Developer ID

When the project starts signing with an Apple Developer ID, the first launch of
that build will go through the **Locked** state once: the new signature has a
new designated requirement, so the Keychain key is not readable until you
approve **Always Allow** in the foreground. That is expected and happens once
per user; the release notes for that version say so.
