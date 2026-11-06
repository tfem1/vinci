#!/usr/bin/env bash
# =============================================================================
#  VINCI — Capture Every Moment — installer for Linux (x86_64, arm64)
#
#    curl -fsSL https://github.com/tfem1/vinci/releases/latest/download/install.sh | bash
#
#  Installs `vinci` (desktop app) and `vinci-cli` into ~/.local/bin — no sudo.
#  Optional environment variables:
#    VINCI_VERSION=v0.2.0   install that release instead of the latest one
#    VINCI_BIN_DIR=~/bin    install somewhere else (default: ~/.local/bin)
#    VINCI_BASE_URL=URL     download the release assets from a mirror
#
#  Every download is checked against the release's SHA256SUMS before anything
#  is installed. The script body is wrapped in main() so a truncated download
#  cannot run half a script.
# =============================================================================
set -euo pipefail

REPO="tfem1/vinci"
# Ed25519 public key (PEM) for SHA256SUMS.sig — set by scripts/configure-project.sh.
# While UNCONFIGURED, only the SHA-256 checksums are verified.
VINCI_RELEASE_PUBKEY_PEM="UNCONFIGURED"
FROM_SOURCE="git clone https://github.com/$REPO && cd vinci && ./scripts/install-from-source.sh"

say()  { printf '  %s\n' "$*"; }
ok()   { printf '  \033[32m✔\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*" >&2; }
fail() { printf '\n  \033[31merror:\033[0m %s\n\n' "$*" >&2; exit 1; }
has()  { command -v "$1" >/dev/null 2>&1; }

# fetch URL DEST — returns 0 on HTTP 200 and 4 on HTTP 404; exits with a clear
# message on network/proxy errors and on any other HTTP status.
fetch() {
  local url="$1" dest="$2" code rc=0 proto="=https"
  [ "${VINCI_ALLOW_HTTP:-0}" = 1 ] && proto="=http,https"   # local mirrors (tests only)
  code="$(curl -sSL --proto "$proto" --tlsv1.2 --retry 2 --connect-timeout 20 -w '%{http_code}' -o "$dest" "$url")" || rc=$?
  if [ "$rc" -ne 0 ]; then
    fail "could not download $url (curl error $rc).
  Check your internet connection or proxy settings (curl honours HTTPS_PROXY)."
  fi
  case "$code" in
    200) return 0 ;;
    404) return 4 ;;
    *) fail "downloading $url failed with HTTP $code." ;;
  esac
}

sha256_of() {
  if has sha256sum; then sha256sum "$1" | awk '{print $1}'
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}

main() {
  printf '\n  \033[1mV I N C I\033[0m  ·  Capture Every Moment\n\n'

  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) fail "macOS is not supported: VINCI's capture engine currently supports Linux and Windows only." ;;
    *) fail "unsupported operating system: $(uname -s). VINCI supports Linux and Windows." ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) fail "unsupported CPU architecture: $(uname -m) (release builds exist for x86_64 and arm64).
  You can try building from source: $FROM_SOURCE" ;;
  esac
  has curl || fail "curl is required."
  has tar || fail "tar is required."
  has sha256sum || has shasum || fail "sha256sum (coreutils) is required to verify the download."

  local asset="vinci-$os-$arch.tar.gz" version="${VINCI_VERSION:-latest}" base
  if [ -n "${VINCI_BASE_URL:-}" ]; then
    base="${VINCI_BASE_URL%/}"
  elif [ "$version" = latest ]; then
    base="https://github.com/$REPO/releases/latest/download"
  else
    base="https://github.com/$REPO/releases/download/$version"
  fi
  local bin_dir="${VINCI_BIN_DIR:-$HOME/.local/bin}"
  ok "System: $os $arch — installing $version to $bin_dir"

  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "${TMP_DIR:-}"' EXIT

  # 1. Checksums (this also tells us whether a release exists at all).
  if ! fetch "$base/SHA256SUMS" "$TMP_DIR/SHA256SUMS"; then
    if [ "$version" = latest ] && [ -z "${VINCI_BASE_URL:-}" ]; then
      fail "no published release found for $REPO (…/releases/latest returned 404).
  Releases appear after a version tag is built by CI. Until then, build from source:
    $FROM_SOURCE"
    fi
    fail "release '$version' not found at $base (SHA256SUMS returned 404)."
  fi
  if [ "$VINCI_RELEASE_PUBKEY_PEM" != "UNCONFIGURED" ]; then
    fetch "$base/SHA256SUMS.sig" "$TMP_DIR/SHA256SUMS.sig" || fail "the release has no SHA256SUMS.sig — refusing to install unsigned files."
    has openssl || fail "openssl is required to verify the release signature."
    printf '%s\n' "$VINCI_RELEASE_PUBKEY_PEM" >"$TMP_DIR/release.pem"
    openssl pkeyutl -verify -pubin -inkey "$TMP_DIR/release.pem" -rawin -in "$TMP_DIR/SHA256SUMS" -sigfile "$TMP_DIR/SHA256SUMS.sig" >/dev/null 2>&1 \
      || fail "SIGNATURE VERIFICATION FAILED for SHA256SUMS — nothing was installed."
    ok "Release signature verified (Ed25519)"
  fi

  # 2. The archive for this machine.
  if ! fetch "$base/$asset" "$TMP_DIR/$asset"; then
    fail "this release does not contain $asset (release builds: linux-x86_64, linux-arm64, windows-x86_64).
  See https://github.com/$REPO/releases or build from source:
    $FROM_SOURCE"
  fi
  local expected actual
  expected="$(awk -v f="$asset" '$2==f || $2=="*"f {print $1; exit}' "$TMP_DIR/SHA256SUMS")"
  [ -n "$expected" ] || fail "$asset is not listed in SHA256SUMS — refusing to install an unverified file."
  actual="$(sha256_of "$TMP_DIR/$asset")"
  [ "$expected" = "$actual" ] || fail "checksum mismatch for $asset (expected $expected, got $actual) — nothing was installed."
  ok "Downloaded and verified $asset"

  # 3. Install (atomic replace: a running copy keeps working until restarted).
  mkdir -p "$TMP_DIR/x"
  tar -xzf "$TMP_DIR/$asset" -C "$TMP_DIR/x"
  local src="$TMP_DIR/x/vinci-$os-$arch"
  { [ -f "$src/vinci" ] && [ -f "$src/vinci-cli" ]; } || fail "unexpected archive layout in $asset."
  mkdir -p "$bin_dir"
  for b in vinci vinci-cli; do
    install -m 0755 "$src/$b" "$bin_dir/.$b.new"
    mv -f "$bin_dir/.$b.new" "$bin_dir/$b"
  done
  ok "Installed $bin_dir/vinci and $bin_dir/vinci-cli"

  # 4. Applications-menu entry (the app is a GUI program).
  local share="${XDG_DATA_HOME:-$HOME/.local/share}"
  mkdir -p "$share/applications" "$share/icons/hicolor/256x256/apps"
  if [ -f "$src/vinci.png" ]; then install -m 0644 "$src/vinci.png" "$share/icons/hicolor/256x256/apps/vinci.png"; fi
  cat >"$share/applications/vinci.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=VINCI
GenericName=Game Clip Recorder
Comment=Capture Every Moment — instant replay for games
Exec=$bin_dir/vinci
Icon=vinci
Terminal=false
Categories=AudioVideo;Video;Recorder;
StartupWMClass=vinci
DESKTOP
  ok "Added VINCI to the applications menu"

  # 5. Runtime checks — reported, never installed behind your back.
  local missing_libs="" fam="other" deps_cmd
  if has ldd; then
    missing_libs="$(ldd "$bin_dir/vinci" 2>/dev/null | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ')"
  fi
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    fam="$(. /etc/os-release; case " ${ID:-} ${ID_LIKE:-} " in
      *" debian "*|*" ubuntu "*) echo debian ;; *" fedora "*|*" rhel "*) echo fedora ;;
      *" arch "*) echo arch ;; *" suse "*|*" opensuse"*) echo suse ;; *) echo other ;; esac)"
  fi
  case "$fam" in
    debian) deps_cmd="sudo apt install libwebkit2gtk-4.1-0 libayatana-appindicator3-1 ffmpeg gstreamer1.0-plugins-good gstreamer1.0-libav gstreamer1.0-pipewire" ;;
    fedora) deps_cmd="sudo dnf install webkit2gtk4.1 libayatana-appindicator-gtk3 ffmpeg-free gstreamer1-plugins-good pipewire-gstreamer" ;;
    arch)   deps_cmd="sudo pacman -S --needed webkit2gtk-4.1 libayatana-appindicator ffmpeg gst-plugins-good gst-libav gst-plugin-pipewire" ;;
    suse)   deps_cmd="sudo zypper install libwebkit2gtk-4_1-0 libayatana-appindicator3-1 ffmpeg gstreamer-plugins-good pipewire-gstreamer" ;;
    *)      deps_cmd="(install WebKitGTK 4.1, libayatana-appindicator3, FFmpeg and GStreamer good/libav/pipewire with your package manager)" ;;
  esac
  if [ -n "$missing_libs" ]; then
    warn "missing system libraries: $missing_libs"
    warn "install the runtime dependencies:  $deps_cmd"
  fi
  if ! has ffmpeg; then
    warn "FFmpeg is not installed — VINCI needs it to capture. Install it:  $deps_cmd"
  fi
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) warn "$bin_dir is not in your PATH. Add this line to ~/.bashrc (or your shell's profile):
      export PATH=\"$bin_dir:\$PATH\"" ;;
  esac

  printf '\n'
  say "Done. Start VINCI from your applications menu or run: vinci"
  say "Default hotkey: Alt+F10 saves the last 30 seconds. Clips go to ~/Videos/VINCI."
  say "Uninstall: rm $bin_dir/vinci $bin_dir/vinci-cli $share/applications/vinci.desktop"
  printf '\n'
}

main "$@"
