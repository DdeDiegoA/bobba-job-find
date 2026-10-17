#!/usr/bin/env bash
# Build the boba-mcp sidecar for a release target and place it where Tauri's
# `bundle.externalBin` expects it (ADR-007: it is signed with the app).
#
#   scripts/release/build-sidecars.sh <target-triple>
#   scripts/release/build-sidecars.sh universal-apple-darwin
#
# A universal macOS app needs a universal sidecar: Tauri looks for
# `binaries/boba-mcp-universal-apple-darwin`.
set -euo pipefail

TARGET="${1:?usage: $0 <target-triple|universal-apple-darwin>}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | node -e \
  'let s="";process.stdin.on("data",(d)=>(s+=d)).on("end",()=>process.stdout.write(JSON.parse(s).target_directory))')"
TARGET_DIR="${TARGET_DIR:-$ROOT/target}"
BIN_DIR="$ROOT/src-tauri/binaries"
mkdir -p "$BIN_DIR"

export CARGO_INCREMENTAL=0

build() {
  local triple="$1"
  # Progress goes to stderr: the caller captures stdout as the binary path.
  echo "building boba-mcp for ${triple}..." >&2
  cargo build --locked --release -p boba-mcp --target "$triple" >&2
  local exe=""
  case "$triple" in *windows*) exe=".exe" ;; esac
  local built="$TARGET_DIR/$triple/release/boba-mcp$exe"
  if [ ! -f "$built" ]; then
    echo "cargo did not produce $built" >&2
    exit 1
  fi
  printf '%s' "$built"
}

if [ "$TARGET" = "universal-apple-darwin" ]; then
  arm="$(build aarch64-apple-darwin)"
  intel="$(build x86_64-apple-darwin)"
  out="$BIN_DIR/boba-mcp-universal-apple-darwin"
  lipo -create "$arm" "$intel" -output "$out"
else
  out="$BIN_DIR/boba-mcp-$TARGET"
  case "$TARGET" in *windows*) out="$out.exe" ;; esac
  cp "$(build "$TARGET")" "$out"
fi

chmod +x "$out"
echo "sidecar ready: $out"
