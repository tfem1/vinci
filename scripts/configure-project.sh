#!/usr/bin/env bash
# Replace the repository / domain placeholders once your GitHub repository exists.
#
#   scripts/configure-project.sh <owner/repo> [install-base-url] [release-public-key-b64]
#
# Examples:
#   scripts/configure-project.sh jdoe/vinci
#   scripts/configure-project.sh jdoe/vinci https://vinci.app  "MCowBQYDK2VwAyEA…"   # after DNS is set up
#
# - owner/repo           replaces USERNAME/vinci everywhere (README, installers, Tauri config, Cargo).
# - install-base-url     where install.sh / install.ps1 are served (default: GitHub
#                        "latest release" download URL, which works with no extra setup).
# - release public key   base64 Ed25519 key printed by `cargo run -p vinci-update --bin
#                        vinci-release-sign -- keygen`; embedded in the updater and the
#                        Linux installer (SHA256SUMS signature check).
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="${1:?usage: $0 <owner/repo> [install-base-url] [release-public-key-b64]}"
[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "invalid repository: $REPO" >&2; exit 1; }
INSTALL_URL="${2:-https://github.com/$REPO/releases/latest/download}"
INSTALL_URL="${INSTALL_URL%/}"
PUBKEY="${3:-}"

files=$(grep -rlE 'USERNAME/vinci|<INSTALL_URL>' --exclude-dir={node_modules,target,dist,.git} . || true)
for f in $files; do
  sed -i.bak -e "s#USERNAME/vinci#$REPO#g" -e "s#<INSTALL_URL>#$INSTALL_URL#g" "$f" && rm -f "$f.bak"
  echo "updated $f"
done

if [ -n "$PUBKEY" ]; then
  printf '%s\n' "$PUBKEY" > crates/vinci-update/update-public-key.txt
  PEM="$(cargo run -q -p vinci-update --bin vinci-release-sign -- pubkey-pem --pubkey "$PUBKEY")"
  python3 - "$PEM" <<'PY'
import sys, re
pem = sys.argv[1].strip()
p = "installers/linux/install.sh"
s = open(p).read()
s = re.sub(r'VINCI_RELEASE_PUBKEY_PEM="[^"]*"', 'VINCI_RELEASE_PUBKEY_PEM="' + pem + '"', s, flags=re.S)
open(p, "w").write(s)
PY
  echo "release public key embedded (updater + install.sh)"
fi
echo "done — review with: git diff"
