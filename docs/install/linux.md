# Linux

Three formats are published for x64: AppImage, `.deb` and `.rpm`. They all need
the WebKitGTK 4.1 runtime, which the distro packages install for you.

## AppImage

```sh
chmod +x "Boba Job Find_<version>_amd64.AppImage"
./"Boba Job Find_<version>_amd64.AppImage"
```

The AppImage still uses the system WebKitGTK and needs FUSE 2 (`libfuse2` on
Ubuntu 22.04+/Debian 12). On newer distros without FUSE the AppImage can be
extracted with `--appimage-extract-and-run`.

## Debian / Ubuntu

```sh
sudo apt install ./boba-job-find_<version>_amd64.deb
```

## Fedora / RHEL

```sh
sudo dnf install ./boba-job-find-<version>-1.x86_64.rpm
```

## Secrets and the keychain

Boba stores the database key in the desktop's Secret Service (GNOME Keyring,
KWallet via `kwalletd`, or `keepassxc`). If no keyring is available, the app
starts in the **Locked** state and asks for the passphrase instead; it never
writes a key next to the database.

## Tray icon

The tray uses AppIndicator. On GNOME, install the **AppIndicator and
KStatusNotifierItem Support** extension (or the distro's equivalent) or the icon
will not appear; the window, notifications and scheduled runs keep working
without it.

## Uninstall

Use your package manager for `.deb`/`.rpm`; delete the AppImage file for the
portable build. Data lives in `~/.local/share/app.bobajobfind/` and the key in
the keyring; **Reset everything** in the app removes both.
