# Troubleshooting

Most hiccups here have a short fix.

## macOS says it can't check the app (Gatekeeper)

Boba is signed with the project's own certificate but not notarised by Apple. Once:

1. Open the app and dismiss the warning.
2. **System Settings → Privacy & Security → Open Anyway**, then **Open**.

Or, for a download from this project's releases only:
`xattr -d com.apple.quarantine "/Applications/Boba Job Find.app"`.

Full steps: [docs/install/macos.md](../install/macos.md). If macOS asks for your Keychain
password *after an update*, don't approve it blindly; check the release's signature as described
there.

## Windows SmartScreen blocks the installer

Press **More info → Run anyway**. This is normal for a new open-source installer with no
download reputation yet. You can compare the SHA-256 on the release page with
`Get-FileHash`. More in [docs/install/windows.md](../install/windows.md).

## Linux: no tray icon, or the AppImage won't start

GNOME needs the *AppIndicator and KStatusNotifierItem Support* extension for the tray icon;
the window, notifications and scheduled runs work without it. The AppImage needs FUSE 2
(`libfuse2`) or `--appimage-extract-and-run`. See [docs/install/linux.md](../install/linux.md).

## Boba is Locked

Boba can't read its key from your keychain, so it can't open your data. The most common causes:

- **No keychain available** (common on minimal Linux setups). Boba asks for your passphrase
  instead: enter it on the **Unlock** screen. It stays in memory for this login session only,
  and locks again after a long idle period (8 hours by default).
- **The keychain is locked or the prompt was dismissed.** Unlock it, then press **Retry reading
  the key**.

While Boba is locked, scheduled runs are skipped, and you get a notification so a quiet day isn't
a mystery. Unlock once and the next run goes ahead.

If you forgot the passphrase, restore from your **recovery backup** (it has its own passphrase).

## Boba says "Limited mode" or AI features are off

No agent CLI is connected, or the one you picked can't pass the safety check for untrusted
content. Search, scoring and the Tracker still work. Open **Settings → AI engine**, pick
another CLI, or run the test call to see what's wrong. Aider can't be used for tasks that read
untrusted text.

## A Source is Failing

Open **Connections** and read the last error on its card. Many failures are temporary. An ATS
board or aggregator may need a valid board name or API key. Remove and re-add the Source if it
keeps failing.

## A session board paused itself

The board showed a CAPTCHA, a sign-in wall, an "unusual activity" notice or a rate limit, so
Boba stopped. Open the board in your browser and sort it out, then **I've resolved it, resume**
in **Settings → Privacy → Sessions**. If the board reports unusual activity, give it a few days.

## Email sign-in doesn't finish

Boba only accepts the sign-in reply on your own machine, in the browser window it opened. If that
window was closed or blocked, try again, or connect with your own client ID, or use IMAP.

## An automatic send didn't go out

Check the **Outbox**: each held item has a note that says why (Kill Switch on, policy held it, no
mailbox connected, and so on). Nothing is ever sent silently.

## Still stuck?

Search or open an issue on the project's GitHub. Please don't paste your Profile or documents.
Security problems go through [SECURITY.md](../../SECURITY.md).
