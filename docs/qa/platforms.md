# Platform QA — tray, notifications, autostart and keychain (S10.6, NFR-18)

NFR-18: *"Tray, notifications, launch-at-login and keychain behave the same on all
three OSes, or document the platform limits."* This is the verification matrix for
that sentence: what Boba does on macOS, Windows, GNOME and KDE, how to check it,
and the limits users are told about.

Installation and signing paths live in `docs/install/` (macOS, Windows, Linux);
this file is about the four OS-integrated features.

## What the app does, and where the code is

| Feature | Implementation | Source of truth |
|---|---|---|
| Tray | One Tauri `Menu` on every OS, items **Open**, **Run now**, **Pause schedule**, **Kill Switch**, **Quit**. Strings embed at compile time in the UI locale. | `src-tauri/src/tray/` (`menu.rs`, `run_controls.rs`, `kill_switch.rs`), icon created in `lib.rs` |
| Notifications | `tauri-plugin-notification` for the run digest, the locked-run notice, Outbox holds and nudges. A click reaches the app as **window focus** and opens Today (max 120 s after the banner). | `src-tauri/src/notifications/`, `src-tauri/src/scheduler/notify.rs` |
| Autostart | `tauri-plugin-autostart`; the user's choice is one `setting` row (`launch_at_login`, default **off**) reconciled with the OS at every startup. A login launch carries `--autostart` and starts in the tray with no window. | `src-tauri/src/autostart/mod.rs`, plugin registered in `lib.rs` |
| Keychain | `keyring-core` with the platform store. Service **`app.bobajobfind`**, entry `db-key` (+ audit anchor and stored secrets). Dev builds use **`app.bobajobfind.dev`** and a `dev/` data subtree, so a local run never touches real data. | `src-tauri/crates/boba-store/src/keys/mod.rs`, ADR-004, ADR-009 |

## Verification matrix

Legend: 🔎 needs a physical run on that OS · ⚠ documented limit. Nothing in this
pass was ✅ — see [Status of this pass](#status-of-this-pass).

### macOS 13+ (Apple silicon or Intel)

| Check | Steps | Expected | Status |
|---|---|---|---|
| Tray appears | Launch, close the window (red button) | App hides to the menu bar; clicking the icon opens the menu with all five items | 🔎 |
| Tray menu acts | Pick **Run now**; pick **Pause schedule**; engage **Kill Switch** | A run starts; scheduled runs stop; automatic sends stop within 2 s | 🔎 |
| Notification | Finish a run with matches | "N Strong, M Good today" banner in Notification Centre; clicking it opens Today | 🔎 |
| Notification permission | First digest on a fresh install | macOS asks for permission once; deny → no banner, run still completes | ⚠ the app never asks explicitly; the OS prompt is the only gate. A denied permission fails silently (the error is tolerated so the run continues). Verify this on every release. |
| Autostart off by default | Fresh install, log out/in | Boba does not start | 🔎 |
| Autostart on | Settings → turn on launch at login, log out/in | Boba starts in the tray, no window, no dock bounce | 🔎 |
| Autostart reconciled | Enable, move the `.app`, restart | The registration follows the app location | 🔎 |
| Keychain item | Keychain Access → search `app.bobajobfind` | One item, `db-key`, "Always Allow" granted once | 🔎 (procedure: `docs/qa/macos-upgrade-test.md`) |
| Keychain survives update | Build N → build N+1 over it | No new key, data opens | 🔎 full procedure in `docs/qa/macos-upgrade-test.md` |

macOS limits (documented in `docs/install/macos.md`): one persistent self-signed
certificate and a Gatekeeper bypass on first install; the Keychain item is bound
to that certificate's designated requirement, so an ad-hoc signature would lose
the key on update — which is why CI fails any `Signature=adhoc` build.

### Windows 10/11 x64

| Check | Steps | Expected | Status |
|---|---|---|---|
| Tray appears | Launch, close the window | Icon in the notification area (possibly under the overflow chevron) | 🔎 |
| Tray menu acts | Same five items | Same as macOS | 🔎 |
| Notification | Finish a run with matches | Toast from Boba; clicking it focuses Boba and opens Today | 🔎 |
| Focus Assist | Turn on **Alarms only** | No toast; the run still completes | ⚠ toast suppression is an OS setting; no app-side bypass |
| Autostart off/on | Fresh install; Settings toggle; sign out/in | Off by default; after enabling, Boba starts in the tray (`Run` registry value) | 🔎 |
| Keychain item | Credential Manager → Windows Credentials → search `app.bobajobfind` | A generic credential per secret | 🔎 |
| Keychain survives update | Install over the previous version | Data opens, no new key | 🔎 |

Windows limits (documented in `docs/install/windows.md`): builds are unsigned
until SignPath Foundation accepts the project, so SmartScreen needs the documented
"More info → Run anyway" bypass. Credential Manager does not bind items to the
binary signature, so updates never lose the key. **Reset everything** can refuse to
delete a database file another process holds open; the DB key is then deliberately
kept, and the UI lists what could not be removed.

### GNOME (GNOME 45+, with the AppIndicator extension) and KDE Plasma 5/6

| Check | Steps | Expected | Status |
|---|---|---|---|
| Tray appears — KDE | Launch | Tray icon in the system tray | 🔎 |
| Tray appears — GNOME | Launch with the **AppIndicator and KStatusNotifierItem Support** extension | Tray icon in the top bar | 🔎 |
| Tray — GNOME without the extension | Launch on a stock GNOME | **No tray icon**; the window and scheduled runs keep working; launching Boba again brings the window back | ⚠ documented in `docs/install/linux.md`; the app cannot work around a missing AppIndicator host |
| Notification | Finish a run | Daemon notification (GNOME/KDE); clicking focuses Boba | 🔎 |
| No notification daemon | Bare window manager | `show()` returns an error; the run completes, nothing crashes | ⚠ no banner at all; the digest is still on Today |
| Autostart | Settings toggle; log out/in | `~/.config/autostart/*.desktop` appears/disappears; login launch starts in the tray | 🔎 |
| Keychain — GNOME | `secret-tool search --all service app.bobajobfind` | Items in GNOME Keyring | 🔎 |
| Keychain — KDE | Export/backup works after unlock; `kwalletd` running | Items in KWallet | 🔎 |
| No keyring | Minimal WM without Secret Service | App starts **Locked** and asks for the passphrase; a scheduled run is recorded `skipped_locked` and the user is notified | ⚠ by design (ADR-004/ADR-013); the run is never silently lost |

## Carried-over platform checks (from earlier slices) — **not yet verified**

These were accepted as residuals by earlier slices and are still waiting for a
physical run. They are listed here so the release checklist is one document.

### W-1 · Sign-out refuses while a live Chrome holds the Session Board profile (S11.1b residual 1)

The session-check reads Chrome's `SingletonLock`. On macOS and Linux it is a
symlink whose target is `<host>-<pid>`; on Windows Chromium writes a **file**
holding the same string, and that form is unverified
(`src-tauri/crates/boba-browser/src/session/mod.rs`, `lock_holder`).

On **Windows**:

1. Enable a Session Board Source, sign in, and **leave that Chrome window open**.
2. Settings → Privacy → Sessions → **Sign out / clear session**.
3. Expected: the sign-out is **refused** with a message; the profile and cookies
   survive. If it succeeds, the Windows lock format is wrong and the profile was
   deleted under a live browser — file it and stop.
4. Close Chrome, sign out again: expected success, profile gone.
5. Kill Chrome from Task Manager while the profile is open (leaving a stale
   lock), then sign out: expected success — a stale lock must not block forever.

Repeat steps 1–4 on macOS to confirm the symlink form still works after any
Chromium update.

### W-2 · An unreadable key enters Locked and never mints a new key (S1.2b AC3, real boot)

On each OS, with real data present:

1. Install, complete onboarding so `boba.db` exists and the key is in the OS store.
   Note the `boba.db` modification time.
2. Quit. Delete **`db-key`** (macOS Keychain Access · Windows Credential Manager ·
   Linux `secret-tool clear service app.bobajobfind`). Start Boba.
3. Expected: the **Locked** screen, offering *retry keychain*, *enter passphrase*
   and *restore from recovery backup*. It never creates a new database over the
   existing one and never writes a new key.

```sh
# macOS / Linux — after the run
ls -l ~/Library/Application\ Support/app.bobajobfind/   # or ~/.local/share/app.bobajobfind/
# boba.db (plus boba.db.kdf only on a passphrase machine); mtime unchanged
```

```powershell
# Windows
Get-Item "$env:APPDATA\app.bobajobfind\boba.db" | Select-Object LastWriteTime
```

4. Enter the correct passphrase (a passphrase machine) or restore the keychain
   item: the store opens with the original data.

### W-3 · A locked run is recorded as skipped and notified once (S1.2b AC4)

With the app in the Locked state (W-2 step 2), trigger a scheduled run at its due
time (or set the clock forward).

Expected: one `job_run` row with outcome **`skipped_locked`** — never `success` —
and exactly one notification per slot — title *"Boba is locked"*, body
*"Open it to run today's search."* After unlocking, the next due slot runs normally.

### W-4 · `boba.db.kdf` is not readable by other accounts on Windows (S1.2b passphrase fallback)

`src-tauri/crates/boba-store/src/unlock/mod.rs` creates the KDF record with
`mode(0o600)`, which is a Unix mode: on Windows the file inherits the directory's
ACL. The record holds only salt and parameters (never key material), but it
identifies a passphrase-protected store and must stay user-scoped.

1. On Windows, put the app into the passphrase fallback (no workable keychain
   entry, choose *enter passphrase*) so `boba.db.kdf` is written.
2. ```powershell
   icacls "$env:APPDATA\app.bobajobfind\boba.db.kdf"
   ```
3. Expected: only the current user (full control), `SYSTEM` and `Administrators`;
   **no** `Everyone` or `Users` entry. If a broad principal appears, the app-data
   directory ACL needs tightening before v1.

### W-5 · Reset with an unlinkable database file (S10.2 D-1)

Windows cannot unlink a file another process holds open. S10.2 erases the
contents in place and keeps the DB **key** while the file remains, so the app
still opens an empty store.

On **Windows**:

1. Hold `boba.db` open from another process (e.g. a second `sqlite3` session or a
   file-locking tool).
2. Settings → Privacy → **Reset everything** and type the confirmation.
3. Expected: the reset **succeeds partially** and lists the file it could not
   remove in `remaining`; the DB key is **not** deleted; relaunching opens an
   empty store. Nothing is reported as a silent success.
4. Release the handle, reset again (or follow `docs/install/windows.md` to remove
   the directory and the credential entries by hand): every artifact is gone.

## Status of this pass

**Not yet verified.** This QA pass ran on macOS in a non-interactive environment:
the tables above were reviewed against the code and the automated suites
(`cargo test`, `pnpm test`), but **no row marked 🔎 was executed on a physical
desktop**. Windows and GNOME/KDE were not available at all. Every 🔎 row and all
five carried-over checks (W-1…W-5) are the release checklist: run them as written
on a VM or workstation and record pass/fail per row in the release notes.

## Limits to carry into the release notes

1. **GNOME without AppIndicator** has no tray — relaunching the app restores the
   window; everything except the tray icon keeps working.
2. **Notification permission** is granted and revoked by the OS. Boba does not
   surface a "notifications are off" state; a denied permission drops banners
   silently while the run and the digest on Today continue.
3. **Login launch starts hidden** (`--autostart`): this is intended, not a
   failure — the window opens from the tray or a second launch.
4. **Linux without a keyring** falls back to the passphrase; until it is entered,
   scheduled runs are skipped and that is notified, never failed quietly.
5. **Unsigned Windows builds** need the SmartScreen bypass until SignPath accepts
   the project; macOS builds need the one-time Gatekeeper bypass.
