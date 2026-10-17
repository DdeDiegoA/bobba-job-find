# Privacy, backup and reset

Boba keeps your job search on your computer.

## What stays where

- **Database.** One encrypted local database (SQLCipher): your Profile, Offers, applications,
  documents and the audit log.
- **Keys and secrets.** Your OS keychain holds the database key and your tokens. If there's no
  keychain, Boba asks for a **passphrase** instead (see [Locked state](troubleshooting.md#boba-is-locked)).
- **No cloud, no account, no telemetry.**
- **Network.** Boba only contacts the Sources you enabled, your mailbox provider, and (if you
  turn it on) GitHub for update checks. **Settings → Privacy → Network activity** lists every
  contact, so you can check.
- **AI.** Your agent CLI talks to its own provider under its own terms. Boba gives it the
  Offers and Profile facts needed for a task and nothing else, and doesn't send anything on its
  own.

## Settings → Privacy

- **Audit Log**: a tamper-evident record of what Boba did.
- **Network activity**: every outbound contact.
- **Sessions**: session boards you've opted into, with **Sign out / clear session**.
- **Export**, **Backup and restore**, **Reset everything**, described below.

## Export

**Export your data** writes your Profile, applications, seen jobs, documents and audit log to a
folder you choose, in open formats another tool can read. **The export is not encrypted**:
anyone who can open that folder can read it. Store it somewhere you control and delete it
when you're done. Secrets are never included.

## Backup and restore

A backup is **one file encrypted with a passphrase you choose**. It opens on any system, so it is
your recovery if this computer is lost.

- **Create a backup**: choose a destination and a passphrase. Boba can't recover a lost
  passphrase.
- **Restore**: choose the file and enter its passphrase. Boba checks the file first, then
  replaces everything on this computer, **keeping a snapshot of what it replaced**.

## Reset everything

Permanently deletes the database, every snapshot, the automation browser profile and its
session cookies, and the keychain entries. **It can't be undone**, so make a backup first if you
might want this data. You confirm by typing a word the screen shows you. If your system won't
let Boba delete the database file while it runs, Boba erases its content and key and tells you
what remains on disk. Uninstall afterwards from your system's app list.

## Retention

Boba clears old seen jobs and snapshots on a schedule so your database doesn't grow forever.
The audit log chain is never touched by this.

Next: [Troubleshooting](troubleshooting.md).
