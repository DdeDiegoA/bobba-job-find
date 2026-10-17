#!/usr/bin/env bash
# CI helper (S10.5 AC3, ADR-009): import the project self-signed identity into a
# temporary keychain for this runner. Expects:
#
#   MACOS_CERTIFICATE           base64 of boba-signing.p12
#   MACOS_CERTIFICATE_PASSWORD  its export password
#
# The workflow sets APPLE_SIGNING_IDENTITY to "Boba Job Find Self-Signed"; Tauri
# passes it to codesign. The keychain lives in $RUNNER_TEMP and is deleted when
# the job ends.
set -euo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "the macOS identity import only runs on macOS" >&2
  exit 1
fi
: "${MACOS_CERTIFICATE:?MACOS_CERTIFICATE is not set}"
: "${MACOS_CERTIFICATE_PASSWORD:?MACOS_CERTIFICATE_PASSWORD is not set}"

KEYCHAIN="${RUNNER_TEMP:-/tmp}/boba-signing.keychain-db"
KEYCHAIN_PASSWORD="$(openssl rand -hex 24)"
P12="${RUNNER_TEMP:-/tmp}/boba-signing.p12"

# The keychain must outlive this script (the build and the signature check run
# in later steps), so only the exported p12 is removed here. The workflow deletes
# the keychain in a final step.
cleanup() {
  rm -f "$P12"
}
trap cleanup EXIT

printf '%s' "$MACOS_CERTIFICATE" | openssl base64 -d -A -out "$P12"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security import "$P12" -k "$KEYCHAIN" -P "$MACOS_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security
# Let codesign use the key without an interactive prompt on the runner.
security set-key-partition-list -S apple-tool:,apple:,codesign: \
  -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
echo "imported the project signing identity into $KEYCHAIN"
