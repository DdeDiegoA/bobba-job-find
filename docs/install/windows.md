# Windows

## Install

1. Download `Boba Job Find_<version>_x64-setup.exe` (NSIS) from the
   [latest release](https://github.com/DdeDiegoA/bobba-job-find/releases/latest).
   The `.msi` is published for managed installs; both work the same.
2. Windows 10/11 x64 is supported.

## SmartScreen bypass (until SignPath is accepted)

Windows builds are **unsigned** for now: free Authenticode signing needs the
project's [SignPath Foundation](https://signpath.org/) application to be
accepted (tracked in `docs/install/RELEASING.md`). Until then SmartScreen shows:

> Windows protected your PC — Microsoft Defender SmartScreen prevented an
> unrecognized app from starting.

Press **More info**, check the publisher reads what you expect, then press
**Run anyway**. This is the normal path for a new open-source installer that has
no download reputation yet; the warning fades as more people install the same
release.

Verify the download if you want to be certain it is the published artifact:
compare the SHA-256 shown on the release page with

```powershell
Get-FileHash .\Boba-Job-Find_<version>_x64-setup.exe -Algorithm SHA256
```

## Data and secrets

The database key is stored in Windows Credential Manager, which does not bind
items to the binary signature, so updates never lose your data. The database
itself lives under `%APPDATA%\app.bobajobfind\` (encrypted, SQLCipher).

## Known issue: Reset everything while the database file is locked

**Reset everything** overwrites the database contents, deletes snapshots and
browser profiles, and removes the DB key, the audit anchor and stored secrets
from Credential Manager. Windows can refuse to delete a file that is still open
(for example when another process has it mapped), and Boba deliberately does
**not** delete the DB key when the file could not be unlinked: a keyless file
would leave an unreadable database behind on the next launch. In that case the
reset reports what it could not remove (the `remaining` list) instead of
pretending it finished.

If you need a fully clean start: quit Boba, then delete
`%APPDATA%\app.bobajobfind\`, and remove the `db-key`, audit-anchor and secret
entries for `app.bobajobfind` from Credential Manager. Nothing is left behind
once those three are gone.

## Uninstall

Settings → Apps → Boba Job Find → Uninstall. The data directory and the
Credential Manager entry are left in place so a reinstall finds your data;
remove them with **Reset everything** in the app before uninstalling.
