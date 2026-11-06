#!/usr/bin/env bash
# Test installers/linux/install.sh against a locally built archive served from a local
# HTTP mirror — no GitHub, no root, a throw-away $HOME.
# Covers: install, idempotent re-install, tampered archive, missing asset, missing release.
#
#   scripts/test-install-local.sh [archives-dir=release-dist] [linux-x86_64|linux-arm64]
set -euo pipefail
cd "$(dirname "$0")/.."

DIST="${1:-release-dist}"
case "$(uname -m)" in aarch64|arm64) default_target=linux-arm64 ;; *) default_target=linux-x86_64 ;; esac
TARGET="${2:-$default_target}"
ASSET="vinci-$TARGET.tar.gz"
[ -f "$DIST/$ASSET" ] || { echo "missing $DIST/$ASSET — run: scripts/package-archive.sh $TARGET $DIST" >&2; exit 1; }

WORK="$(mktemp -d)"; PORT="${PORT:-8765}"
mkdir -p "$WORK/mirror" "$WORK/empty" "$WORK/home"
cleanup() { kill "${HTTP_PID:-0}" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT
cp "$DIST/$ASSET" installers/linux/install.sh "$WORK/mirror/"
( cd "$WORK/mirror" && sha256sum "$ASSET" install.sh > SHA256SUMS )
python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$WORK" >/dev/null 2>&1 &
HTTP_PID=$!
sleep 1

run() { # run <mirror-subdir> — installs into the throw-away HOME
  env HOME="$WORK/home" XDG_DATA_HOME="" VINCI_ALLOW_HTTP=1 VINCI_BASE_URL="http://127.0.0.1:$PORT/$1" \
    bash installers/linux/install.sh 2>&1
}
pass() { printf 'PASS  %s\n' "$*"; }
die()  { printf 'FAIL  %s\n' "$*" >&2; exit 1; }

out="$(run mirror)" || die "install failed:
$out"
for b in vinci vinci-cli; do [ -x "$WORK/home/.local/bin/$b" ] || die "$b not installed"; done
"$WORK/home/.local/bin/vinci-cli" --version >/dev/null || die "installed vinci-cli does not run"
[ -f "$WORK/home/.local/share/applications/vinci.desktop" ] || die "desktop entry missing"
pass "install into ~/.local/bin ($("$WORK/home/.local/bin/vinci-cli" --version))"

run mirror >/dev/null || die "re-install failed"
pass "re-install is idempotent"

cp "$WORK/mirror/$ASSET" "$WORK/orig"
printf 'x' >> "$WORK/mirror/$ASSET"
if out="$(run mirror)"; then die "tampered archive was installed"; fi
grep -q "checksum mismatch" <<<"$out" || die "tamper: unexpected message: $out"
pass "tampered archive rejected (checksum mismatch)"

rm "$WORK/mirror/$ASSET"
if out="$(run mirror)"; then die "install succeeded without the asset"; fi
grep -q "does not contain $ASSET" <<<"$out" || die "missing asset: unexpected message: $out"
pass "missing asset reported clearly"
mv "$WORK/orig" "$WORK/mirror/$ASSET"

if out="$(run empty)"; then die "install succeeded without a release"; fi
grep -q "not found" <<<"$out" || die "missing release: unexpected message: $out"
pass "missing release reported clearly"
echo "install.sh: all checks passed"
