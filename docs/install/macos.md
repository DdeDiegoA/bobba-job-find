# macOS

## Install

1. Download `Boba Job Find_<version>_universal.dmg` from the
   [latest release](https://github.com/DdeDiegoA/bobba-job-find/releases/latest).
2. Open the DMG and drag **Boba Job Find** to **Applications**. The image mounts
   straight away: there is no licence agreement to click through.
3. The app is **signed with the project's persistent self-signed identity, but
   not notarised** (there is no Apple Developer ID yet), and macOS quarantines
   everything downloaded from the browser. Gatekeeper stops the first launch and
   may say the app is *damaged*: the download is fine, it is the untrusted
   certificate. Clear the quarantine flag once, on the copy in Applications:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Boba Job Find.app"
   ```

   Only do this for a download you got from this project's GitHub releases; the
   command removes the quarantine flag from whatever app you point it at.
4. Open **Boba Job Find** normally. Every later launch, and every update, opens
   without a prompt.

### Why not right-click → Open?

macOS 15 and later no longer let **Control-click → Open** bypass Gatekeeper for
software that is not notarised. **System Settings → Privacy & Security → Open
Anyway** only appears when the warning offers it, and a "damaged" warning does
not offer it. Removing the quarantine flag, as in step 3, is the path that always
works.

### Check that it is our build (optional)

Anyone can create a self-signed certificate, so check the one the app carries
before you approve it:

```sh
codesign -d -r- "/Applications/Boba Job Find.app"
```

The last line must be

```
designated => identifier "app.bobajobfind" and certificate root = H"8f272deee2dd5a0967cfab48424796709c92879f"
```

and the certificate leaf must be the one pinned in
[`scripts/release/macos-identity.pin`](../../scripts/release/macos-identity.pin).
`codesign --verify --deep --strict "/Applications/Boba Job Find.app"` must answer
`valid on disk`. `spctl -a -vv -t exec` still answers `rejected` no matter what
you do: Gatekeeper wants an Apple notarisation ticket, which the project cannot
get yet. That rejection is expected and does not change how the app runs.

## Why this is safe to approve

- The build is signed by one persistent, project-owned certificate, not ad-hoc.
  The same certificate signs every release, so your Keychain entry (and the
  database key inside it) survives every update. If you ever see a Keychain
  prompt after an update, something is wrong: verify the release's
  `codesign` output against the pin in `scripts/release/macos-identity.pin`.
- The release pipeline fails if `codesign -dv` reports an ad-hoc signature or a
  different certificate, so an ad-hoc build cannot be published by accident.
- The MIT licence and the ai-job-search upstream notice ship inside the app, at
  `Contents/Resources/LICENSE`, so they travel with every copy.
- `tauri dev` and local builds use the separate `app.bobajobfind.dev` Keychain
  service and a separate data directory, so a development build can never read
  or overwrite your real data.

## Uninstall

Drag the app to the Trash. Your database and Keychain entry are not removed;
use **Settings → Privacy** to export a backup first, and **Reset everything**
in the app if you want the data gone.

## Future: Apple Developer ID

Notarisation needs an Apple Developer ID, which the project does not have yet.
When it starts signing with one, the first launch of that build will go through
the **Locked** state once: the new signature has a new designated requirement, so
the Keychain key is not readable until you approve **Always Allow** in the
foreground. That is expected and happens once per user; the release notes for
that version say so.
