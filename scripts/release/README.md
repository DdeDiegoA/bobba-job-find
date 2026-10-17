# Release scripts (S10.5)

The logic behind `.github/workflows/release.yml`. Each guard is also runnable by
hand, and its parsing lives in `lib/` with unit tests (`pnpm test`).

| Script | What it does | Run by |
|---|---|---|
| `check-version.mjs <tag>` | Fails unless the tag, `package.json`, `tauri.conf.json` and `Cargo.toml` agree (AD-22). | release guard |
| `verify-updater-pubkey.mjs` | Fails unless `plugins.updater.pubkey` is a real minisign public key (never a placeholder or a secret key). | release guard |
| `verify-updater-endpoint.mjs` | Fails unless the updater endpoint names the repository running it (`GITHUB_REPOSITORY`, or `--repo` locally). | release guard + CI |
| `check-macos-signature.mjs <App.app>` | Fails on an ad-hoc signature, the wrong identifier, or a designated requirement whose certificate differs from `macos-identity.pin` (ADR-009). | release build + publish |
| `verify-release.mjs <tag> <dir>` | Fails unless every installer, the `.sig` files and a `latest.json` resolving every platform are attached, the version matches the tag, and the release is not a draft. | publish |
| `build-sidecars.sh <triple>` | Builds `boba-mcp` for the release target and places it in `src-tauri/binaries/` (universal on macOS). | release build |
| `import-macos-identity.sh` | CI: imports `MACOS_CERTIFICATE` into a temporary keychain for codesign. | release build |
| `generate-updater-keys.sh --out <dir>` | One-time: creates the updater minisign keypair and prints the public key. | maintainer |
| `generate-macos-identity.sh --out <dir>` | One-time: creates the persistent self-signed signing identity and pins its certificate leaf. | maintainer |

`macos-identity.pin` is committed: it is the one certificate every macOS build
must carry. See `docs/install/RELEASING.md` for key custody and the full
runbook.

Pure parsing (tag/version rules, `codesign` output, `latest.json`, minisign
keys) lives in `lib/*.mjs` and is unit-tested in `lib/*.test.mjs`.
