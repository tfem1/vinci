#!/usr/bin/env bash
# Package the built binaries into the portable archive installed by install.sh.
#
#   scripts/package-archive.sh <linux-x86_64|linux-arm64> [out-dir=release-dist] [binaries-dir]
#
# Expects `npx tauri build --no-bundle` and `cargo build --release -p vinci-cli`
# to have run. Produces <out-dir>/vinci-<target>.tar.gz containing
# vinci-<target>/{vinci, vinci-cli, vinci.png, LICENSE, THIRD_PARTY_NOTICES.md}.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="${1:?usage: $0 <linux-x86_64|linux-arm64> [out-dir] [binaries-dir]}"
OUT="${2:-release-dist}"
BIN="${3:-target/release}"
case "$TARGET" in linux-x86_64|linux-arm64) ;; *) echo "unknown target $TARGET" >&2; exit 1 ;; esac

NAME="vinci-$TARGET"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/$NAME" "$OUT"
for b in vinci vinci-cli; do
  [ -x "$BIN/$b" ] || { echo "missing $BIN/$b — build first" >&2; exit 1; }
  install -m 0755 "$BIN/$b" "$WORK/$NAME/$b"
done
install -m 0644 src-tauri/icons/128x128@2x.png "$WORK/$NAME/vinci.png"
install -m 0644 LICENSE THIRD_PARTY_NOTICES.md "$WORK/$NAME/"
tar -C "$WORK" -czf "$OUT/$NAME.tar.gz" "$NAME"
echo "$OUT/$NAME.tar.gz"
