#!/usr/bin/env bash
# Smoke-test installers/linux/install.sh against locally built packages served from
# a local HTTP mirror (no GitHub needed). Installs the .deb, checks it, uninstalls.
#   scripts/test-install-local.sh target/release/bundle
# Needs root or passwordless sudo (CI runners have it).
set -euo pipefail
cd "$(dirname "$0")/.."
BUNDLES="${1:-target/release/bundle}"
MIRROR="$(mktemp -d)"; PORT="${PORT:-8765}"
trap 'kill "${HTTP_PID:-0}" 2>/dev/null || true; rm -rf "$MIRROR"' EXIT

VINCI_UPDATE_SIGNING_KEY="" scripts/package-release.sh 0.0.0-test "$BUNDLES" "$MIRROR" >/dev/null 2>&1
ls "$MIRROR"/VINCI-linux-*.deb >/dev/null || { echo "no .deb built in $BUNDLES" >&2; exit 1; }
python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$MIRROR" >/dev/null 2>&1 &
HTTP_PID=$!
sleep 1

export VINCI_BASE_URL="http://127.0.0.1:$PORT" VINCI_ALLOW_HTTP=1
bash installers/linux/install.sh --method deb --yes --no-launch --no-deps
dpkg -s vinci >/dev/null && [ -x /usr/bin/vinci ] && echo "install: OK"

# Tampered download must be rejected.
cp "$MIRROR/VINCI-linux-x86_64.deb" "$MIRROR/orig.deb"
printf 'x' >> "$MIRROR/VINCI-linux-x86_64.deb"
if bash installers/linux/install.sh --method deb --yes --no-launch --no-deps >/dev/null 2>&1; then
  echo "tampered package was NOT rejected" >&2; exit 1
fi
mv -f "$MIRROR/orig.deb" "$MIRROR/VINCI-linux-x86_64.deb"
echo "tamper check: OK (checksum mismatch rejected)"

bash installers/linux/install.sh --uninstall --yes
! dpkg -s vinci >/dev/null 2>&1 && echo "uninstall: OK"
