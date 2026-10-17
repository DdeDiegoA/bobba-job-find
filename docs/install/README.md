# Install Boba Job Find

Boba Job Find is a local-first desktop app. Download the installer for your OS
from the [latest release](https://github.com/diegoarenas/boba-job-find/releases/latest),
or build it yourself from source.

| OS | Installer | Signing |
|---|---|---|
| macOS 13+ (Apple Silicon and Intel) | Universal `.dmg` | Self-signed with the project's persistent identity; [Gatekeeper bypass](macos.md) on first launch |
| Windows 10/11 x64 | NSIS `-setup.exe`, or `.msi` | Unsigned; [SmartScreen bypass](windows.md) until the SignPath Foundation application is accepted |
| Linux x64 | `.AppImage`, `.deb`, `.rpm` | Unsigned (as is normal for Linux desktop apps) |

Nothing is uploaded anywhere: your data stays in an encrypted local database,
and secrets live in your OS keychain. The app never runs a cloud service, has
no account and no telemetry.

- [macOS](macos.md)
- [Windows](windows.md)
- [Linux](linux.md)
- [Updates and signatures](updates.md)
- [Releasing (maintainers)](RELEASING.md)

## Requirements

- **Agent CLI:** Boba uses an agent CLI you already have installed (Claude Code,
  Codex, OpenCode, Gemini CLI…). It does not bundle a model.
- **macOS/Linux:** a working keychain (Keychain, GNOME Keyring / KWallet). Without
  one, Boba falls back to the passphrase unlock and says so.
- **Linux:** WebKitGTK 4.1 runtime libraries (the `.deb`/.rpm list them; the
  AppImage still needs the system WebKitGTK and FUSE).
