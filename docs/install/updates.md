# Updates and signatures

## Opt-in, off by default

Boba never contacts an update server unless you turn update checks on
(**Settings → Privacy → Network activity**). The switch is off on a fresh
install, and while it is off the app makes no update request at all — the
"Update checks" row in Network activity only appears once it is on.

When update checks are on:

- The app asks GitHub Releases (the only update host) on startup and every
  24 hours, and the contact shows up in Network activity like every other
  destination.
- Boba only ever downloads an artifact from the release it just read, and only
  if it is newer than the running version.

## Signature verification

Every update artifact is signed with the project's **minisign** key. The public
key is compiled into the app (`plugins.updater.pubkey` in
`src-tauri/tauri.conf.json`); the private key exists only as a GitHub secret.

- An artifact whose signature does not verify is **refused**. There is no
  "install anyway".
- The signature also carries the release version. An update server response
  that pairs a newer version number with an older, validly signed artifact is
  rejected (`requireSignedVersion`), so a downgrade replay cannot be used to
  make you install an old build.
- This is separate from OS code signing. macOS self-signing protects your
  Keychain across updates; minisign is what proves the update is ours.

## Installing an update

When an update is available, a **native confirmation dialog** asks before
anything is downloaded or installed ("Update" / "Later"). Nothing installs
silently and nothing restarts the app without your click. On Windows the
installer then runs in passive mode (a progress bar, no wizard).

## If you suspect an update

The only update endpoint is
`https://github.com/DdeDiegoA/bobba-job-find/releases/latest/download/latest.json`.
If you see a prompt that did not follow a change you made to the setting, cancel
it, check Network activity, and compare the release against this repository.
You can always update by hand: download the installer from the releases page and
install it over the running version; your data is untouched.
