#!/usr/bin/env bash
# One-time setup of the Tauri updater minisign keypair (AD-22, S10.5 AC2).
#
#   scripts/release/generate-updater-keys.sh --out ~/boba-updater
#
# Writes boba-updater.key (private, password-protected), boba-updater.key.pub
# and boba-updater.password. Prints the public key to paste into
# src-tauri/tauri.conf.json `plugins.updater.pubkey`. The private key must never
# enter the repository: store it as TAURI_SIGNING_PRIVATE_KEY and its password as
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
set -euo pipefail

OUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="${2:?--out needs a directory}"; shift 2 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ -z "$OUT" ]; then
  echo "usage: $0 --out <directory>" >&2
  exit 2
fi

umask 077
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

KEY="$OUT/boba-updater.key"
PWFILE="$OUT/boba-updater.password"
if [ -e "$KEY" ]; then
  echo "refusing to overwrite $KEY; a new key would break every installed app's updates" >&2
  exit 1
fi

PASSWORD="${BOBA_UPDATER_KEY_PASSWORD:-$(openssl rand -base64 24)}"
printf '%s' "$PASSWORD" > "$PWFILE"
chmod 600 "$PWFILE"

# The CLI writes <path> and <path>.pub. `--password` is the only non-interactive
# form the signer offers; this is a one-time local run by a maintainer, never CI.
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$PASSWORD" \
  pnpm tauri signer generate -w "$KEY" --password "$PASSWORD" >/dev/null

chmod 600 "$KEY"
PUBKEY="$(cat "$KEY.pub")"

echo "public key (paste into src-tauri/tauri.conf.json plugins.updater.pubkey):"
echo
echo "$PUBKEY"
echo
echo "Next steps (one time):"
echo "  1. Store the private key contents as TAURI_SIGNING_PRIVATE_KEY:"
printf '       %s\n' "$KEY"
echo "  2. Store $PWFILE as TAURI_SIGNING_PRIVATE_KEY_PASSWORD."
echo "  3. Keep both in an offline maintainer backup. Losing them means every"
echo "     installed app can no longer verify updates until a manual reinstall."
