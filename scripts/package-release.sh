#!/usr/bin/env bash
# Collect Tauri bundles under stable names, write SHA256SUMS, sign it and build the
# signed update manifest (vinci-update.json).
#
#   scripts/package-release.sh <version> <input-dir> <output-dir> [stable|beta]
#
# <input-dir> is searched recursively for the bundles produced by `tauri build`
# (deb, rpm, AppImage, NSIS setup.exe, MSI). Signing uses VINCI_UPDATE_SIGNING_KEY
# when set (CI secret); without it the release is published unsigned (SHA-256 only).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:?version}"; IN="${2:?input dir}"; OUT="${3:?output dir}"; CHANNEL="${4:-stable}"
REPO="${VINCI_REPO:-${GITHUB_REPOSITORY:-USERNAME/vinci}}"
TAG="v${VERSION#v}"
mkdir -p "$OUT"

declare -A KEYS=()
add() { # add <source> <stable name> <manifest key>
  cp -f "$1" "$OUT/$2"
  KEYS["$3"]="$2"
  echo "  $2  ←  $(basename "$1")"
}
while IFS= read -r -d '' f; do
  b="$(basename "$f")"
  case "$b" in
    *_amd64.deb)            add "$f" VINCI-linux-x86_64.deb       linux-x86_64-deb ;;
    *_arm64.deb)            add "$f" VINCI-linux-aarch64.deb      linux-aarch64-deb ;;
    *.x86_64.rpm)           add "$f" VINCI-linux-x86_64.rpm       linux-x86_64-rpm ;;
    *.aarch64.rpm)          add "$f" VINCI-linux-aarch64.rpm      linux-aarch64-rpm ;;
    *_amd64.AppImage)       add "$f" VINCI-linux-x86_64.AppImage  linux-x86_64-appimage ;;
    *_aarch64.AppImage)     add "$f" VINCI-linux-aarch64.AppImage linux-aarch64-appimage ;;
    *_x64-setup.exe)        add "$f" VINCI-windows-x64-setup.exe  windows-x86_64-nsis ;;
    *_x64_en-US.msi)        add "$f" VINCI-windows-x64.msi        windows-x86_64-msi ;;
  esac
done < <(find "$IN" -type f \( -name '*.deb' -o -name '*.rpm' -o -name '*.AppImage' -o -name '*-setup.exe' -o -name '*.msi' \) -print0)

[ "${#KEYS[@]}" -gt 0 ] || { echo "no bundles found in $IN" >&2; exit 1; }

# Installer scripts are published with every release (stable install URLs).
cp installers/linux/install.sh installers/windows/install.ps1 installers/windows/uninstall.ps1 "$OUT/"

( cd "$OUT" && sha256sum -- * | grep -v -E ' (SHA256SUMS|vinci-update\.json)' > SHA256SUMS )
echo "SHA256SUMS:"; sed 's/^/  /' "$OUT/SHA256SUMS"

SIGN=(cargo run -q --release -p vinci-update --bin vinci-release-sign --)
if [ -n "${VINCI_UPDATE_SIGNING_KEY:-}" ]; then
  "${SIGN[@]}" sign-file --in "$OUT/SHA256SUMS" --out "$OUT/SHA256SUMS.sig"
  args=(manifest --version "${VERSION#v}" --channel "$CHANNEL" --base-url "https://github.com/$REPO/releases/download/$TAG" --date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --out "$OUT/vinci-update.json")
  [ -n "${VINCI_RELEASE_NOTES:-}" ] && args+=(--notes "$VINCI_RELEASE_NOTES")
  for k in "${!KEYS[@]}"; do args+=(--asset "$k=$OUT/${KEYS[$k]}"); done
  "${SIGN[@]}" "${args[@]}"
else
  echo "WARNING: VINCI_UPDATE_SIGNING_KEY not set — no SHA256SUMS.sig / vinci-update.json (in-app updates disabled for this release)." >&2
fi
echo "release assets in $OUT"
