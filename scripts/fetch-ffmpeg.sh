#!/usr/bin/env bash
# Download a static FFmpeg build to bundle as a Tauri sidecar (AppImage / Windows).
# Verified against the checksums published with the build. Requires network access
# to github.com (works in GitHub Actions).
#
#   scripts/fetch-ffmpeg.sh linux64|linuxarm64|win64
#
# Env: FFMPEG_SERIES (default 8.1 — needed for Windows.Graphics.Capture `gfxcapture`).
set -euo pipefail
cd "$(dirname "$0")/.."

PLATFORM="${1:?usage: $0 linux64|linuxarm64|win64}"
SERIES="${FFMPEG_SERIES:-8.1}"
BASE="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest"
case "$PLATFORM" in
  linux64)    NAME="ffmpeg-n${SERIES}-latest-linux64-gpl-${SERIES}.tar.xz";    TRIPLE="x86_64-unknown-linux-gnu"; EXT="" ;;
  linuxarm64) NAME="ffmpeg-n${SERIES}-latest-linuxarm64-gpl-${SERIES}.tar.xz"; TRIPLE="aarch64-unknown-linux-gnu"; EXT="" ;;
  win64)      NAME="ffmpeg-n${SERIES}-latest-win64-gpl-${SERIES}.zip";         TRIPLE="x86_64-pc-windows-msvc";   EXT=".exe" ;;
  *) echo "unknown platform $PLATFORM" >&2; exit 1 ;;
esac

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
echo "downloading $NAME"
curl -fsSL --retry 3 -o "$WORK/$NAME" "$BASE/$NAME"
curl -fsSL --retry 3 -o "$WORK/checksums.sha256" "$BASE/checksums.sha256"
EXPECTED="$(grep " $NAME\$" "$WORK/checksums.sha256" | awk '{print $1}')"
[ -n "$EXPECTED" ] || { echo "no checksum for $NAME" >&2; exit 1; }
ACTUAL="$(sha256sum "$WORK/$NAME" | awk '{print $1}')"
[ "$EXPECTED" = "$ACTUAL" ] || { echo "CHECKSUM MISMATCH for $NAME" >&2; exit 1; }
echo "sha256 OK ($ACTUAL)"

mkdir -p "$WORK/x" src-tauri/binaries
case "$NAME" in
  *.zip) unzip -q "$WORK/$NAME" -d "$WORK/x" ;;
  *) tar -xJf "$WORK/$NAME" -C "$WORK/x" ;;
esac
for tool in ffmpeg ffprobe; do
  src="$(find "$WORK/x" -type f -name "$tool$EXT" | head -1)"
  [ -n "$src" ] || { echo "$tool not found in archive" >&2; exit 1; }
  install -m 0755 "$src" "src-tauri/binaries/$tool-$TRIPLE$EXT"
done
LICENSE_SRC="$(find "$WORK/x" -maxdepth 2 -iname 'LICENSE*' | head -1 || true)"
[ -n "$LICENSE_SRC" ] && cp "$LICENSE_SRC" src-tauri/binaries/FFMPEG-LICENSE.txt
echo "sidecars ready in src-tauri/binaries/ for $TRIPLE"
