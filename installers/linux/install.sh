#!/usr/bin/env bash
# =============================================================================
#  VINCI — Capture Every Moment
#  Linux installer:   curl -fsSL <INSTALL_URL>/install.sh | bash
#
#  Options (after `bash -s --` when piped):
#    --version vX.Y.Z   install a specific release (default: latest stable)
#    --beta             latest pre-release (beta channel)
#    --method M         deb | rpm | appimage   (default: auto-detected)
#    --user             per-user AppImage install, never uses sudo
#    --no-deps          do not install runtime dependencies
#    --yes              never prompt (assume defaults)
#    --no-launch        do not offer to start VINCI
#    --uninstall        remove VINCI (keeps your clips and settings)
#    --purge            with --uninstall: also delete settings & library index
#                       (your clips are NEVER deleted)
#  Environment: VINCI_REPO=owner/repo, VINCI_BASE_URL=<mirror of release assets>
#
#  Security: every download is verified against SHA256SUMS; when a release
#  signing key is configured below, SHA256SUMS itself is verified (Ed25519).
#  Nothing is executed before verification. The whole script is wrapped in
#  main() so a truncated download cannot run partially.
# =============================================================================
set -euo pipefail

VINCI_REPO="${VINCI_REPO:-USERNAME/vinci}"
# Ed25519 public key (PEM) used to verify SHA256SUMS.sig. Set by
# scripts/configure-project.sh — while it reads UNCONFIGURED, only SHA-256 is checked.
VINCI_RELEASE_PUBKEY_PEM="UNCONFIGURED"

# ----------------------------------------------------------------- utilities
if [ -t 1 ]; then B=$'\e[1m'; D=$'\e[2m'; R=$'\e[0m'; G=$'\e[32m'; Y=$'\e[33m'; E=$'\e[31m'; else B=""; D=""; R=""; G=""; Y=""; E=""; fi
say()  { printf '%s\n' "  $*"; }
step() { printf '\n%s\n' "${B}▸ $*${R}"; }
ok()   { printf '%s\n' "  ${G}✔${R} $*"; }
warn() { printf '%s\n' "  ${Y}!${R} $*" >&2; }
die()  { printf '%s\n' "  ${E}✘ $*${R}" >&2; exit 1; }
has()  { command -v "$1" >/dev/null 2>&1; }

# Interactive prompts read from the terminal, not from the piped script.
ask() { # ask "Question" default(Y|N) -> 0 for yes
  local q="$1" def="${2:-Y}" reply=""
  if [ "$ASSUME_YES" = 1 ] || ! { [ -r /dev/tty ] && [ -w /dev/tty ]; } 2>/dev/null; then
    [ "$def" = Y ]; return
  fi
  printf '  %s %s ' "$q" "$([ "$def" = Y ] && echo '[Y/n]' || echo '[y/N]')" >/dev/tty
  read -r reply </dev/tty || reply=""
  reply="${reply:-$def}"
  case "$reply" in [Yy]*) return 0 ;; *) return 1 ;; esac
}

SUDO=""
need_root() {
  if [ "$(id -u)" -eq 0 ]; then SUDO=""; return 0; fi
  if [ "$USER_ONLY" = 1 ]; then return 1; fi
  if has sudo; then SUDO="sudo"; return 0; fi
  if has doas; then SUDO="doas"; return 0; fi
  return 1
}

download() { # download URL DEST
  if has curl; then curl -fsSL --proto '=https' --tlsv1.2 --retry 3 --connect-timeout 15 ${CURL_EXTRA[@]+"${CURL_EXTRA[@]}"} -o "$2" "$1"
  elif has wget; then wget -q --https-only -O "$2" "$1"
  else die "curl or wget is required"; fi
}

sha256_of() {
  if has sha256sum; then sha256sum "$1" | awk '{print $1}'
  elif has shasum; then shasum -a 256 "$1" | awk '{print $1}'
  else die "sha256sum is required to verify downloads"; fi
}

# ------------------------------------------------------------------ main flow
main() {
  VERSION="latest"; CHANNEL="stable"; METHOD="auto"; USER_ONLY=0; NO_DEPS=0
  ASSUME_YES=0; NO_LAUNCH=0; UNINSTALL=0; PURGE=0; CURL_EXTRA=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --version) VERSION="${2:?--version needs a value}"; shift ;;
      --beta) CHANNEL="beta" ;;
      --method) METHOD="${2:?--method needs deb|rpm|appimage}"; shift ;;
      --user) USER_ONLY=1; METHOD="appimage" ;;
      --no-deps) NO_DEPS=1 ;;
      --yes|-y) ASSUME_YES=1 ;;
      --no-launch) NO_LAUNCH=1 ;;
      --uninstall) UNINSTALL=1 ;;
      --purge) PURGE=1 ;;
      -h|--help) sed -n '2,24p' "${BASH_SOURCE[0]:-/dev/null}" 2>/dev/null || true; exit 0 ;;
      *) die "unknown option: $1" ;;
    esac
    shift
  done

  printf '\n%s\n%s\n' "${B}  V I N C I${R}" "${D}  Capture Every Moment${R}"

  [ "$(uname -s)" = "Linux" ] || die "This installer is for Linux. On Windows use: irm <INSTALL_URL>/install.ps1 | iex"
  detect_platform
  if [ "$UNINSTALL" = 1 ]; then uninstall; exit 0; fi

  choose_method
  resolve_release
  TMP_DIR="$(mktemp -d)"; trap 'rm -rf "${TMP_DIR:-}"' EXIT
  fetch_and_verify "$TMP_DIR"
  [ "$NO_DEPS" = 1 ] || install_runtime_deps
  install_package "$TMP_DIR/$ASSET"
  configure_permissions
  check_runtime
  verify_install
  offer_launch
}

detect_platform() {
  step "Detecting your system"
  case "$(uname -m)" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) die "Unsupported CPU architecture: $(uname -m) (VINCI supports x86_64 and aarch64)" ;;
  esac
  DISTRO_ID="unknown"; DISTRO_LIKE=""; DISTRO_NAME="Linux"
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    DISTRO_ID="${ID:-unknown}"; DISTRO_LIKE="${ID_LIKE:-}"; DISTRO_NAME="${PRETTY_NAME:-$DISTRO_ID}"
  fi
  SESSION="${XDG_SESSION_TYPE:-}"
  [ -n "${WAYLAND_DISPLAY:-}" ] && SESSION="wayland"
  ok "$DISTRO_NAME · $ARCH${SESSION:+ · $SESSION session}"
}

family() {
  case " $DISTRO_ID $DISTRO_LIKE " in
    *" debian "*|*" ubuntu "*|*" linuxmint "*|*" pop "*|*" elementary "*|*" zorin "*|*" kali "*|*" neon "*) echo debian ;;
    *" fedora "*|*" rhel "*|*" centos "*|*" rocky "*|*" almalinux "*|*" nobara "*|*" ultramarine "*) echo fedora ;;
    *" opensuse"*|*" suse "*|*" sles "*) echo suse ;;
    *" arch "*|*" manjaro "*|*" endeavouros "*|*" cachyos "*|*" garuda "*) echo arch ;;
    *) echo other ;;
  esac
}

choose_method() {
  FAMILY="$(family)"
  if [ "$METHOD" = auto ]; then
    case "$FAMILY" in
      debian) METHOD=deb ;;
      fedora|suse) METHOD=rpm ;;
      *) METHOD=appimage ;;
    esac
  fi
  if [ "$METHOD" != appimage ] && ! need_root; then
    warn "No sudo/doas available: falling back to a per-user AppImage install."
    METHOD=appimage; USER_ONLY=1
  fi
  case "$METHOD/$ARCH" in
    deb/x86_64|appimage/x86_64|rpm/x86_64) ;;
    deb/aarch64|appimage/aarch64|rpm/aarch64) ;;
    *) die "No $METHOD package for $ARCH" ;;
  esac
  case "$METHOD" in
    deb) ASSET="VINCI-linux-$ARCH.deb" ;;
    rpm) ASSET="VINCI-linux-$ARCH.rpm" ;;
    appimage) ASSET="VINCI-linux-$ARCH.AppImage" ;;
    *) die "unknown method $METHOD (use deb, rpm or appimage)" ;;
  esac
  ok "Install method: $METHOD ($ASSET)"
}

resolve_release() {
  step "Finding the release"
  if [ -n "${VINCI_BASE_URL:-}" ]; then
    BASE="${VINCI_BASE_URL%/}"
    [ "${VINCI_ALLOW_HTTP:-0}" = 1 ] && CURL_EXTRA=(--proto '=http,https')
    ok "Using mirror $BASE"; return
  fi
  case "$VINCI_REPO" in USERNAME/*) die "This installer is not configured yet (VINCI_REPO=$VINCI_REPO). Maintainers: run scripts/configure-project.sh" ;; esac
  if [ "$VERSION" != latest ]; then
    BASE="https://github.com/$VINCI_REPO/releases/download/$VERSION"
  elif [ "$CHANNEL" = beta ]; then
    local api tag
    api="$(curl -fsSL "https://api.github.com/repos/$VINCI_REPO/releases?per_page=15")" || die "GitHub API unreachable"
    tag="$(printf '%s' "$api" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
    [ -n "$tag" ] || die "No release found"
    BASE="https://github.com/$VINCI_REPO/releases/download/$tag"; VERSION="$tag"
  else
    BASE="https://github.com/$VINCI_REPO/releases/latest/download"
  fi
  ok "$VINCI_REPO · ${VERSION}${CHANNEL:+ ($CHANNEL)}"
}

fetch_and_verify() {
  local dir="$1"
  step "Downloading"
  download "$BASE/SHA256SUMS" "$dir/SHA256SUMS" || die "Could not download SHA256SUMS from $BASE"
  if [ "$VINCI_RELEASE_PUBKEY_PEM" != "UNCONFIGURED" ]; then
    download "$BASE/SHA256SUMS.sig" "$dir/SHA256SUMS.sig" || die "Could not download the checksum signature"
    has openssl || die "openssl is required to verify the release signature"
    printf '%s\n' "$VINCI_RELEASE_PUBKEY_PEM" >"$dir/release.pem"
    openssl pkeyutl -verify -pubin -inkey "$dir/release.pem" -rawin -in "$dir/SHA256SUMS" -sigfile "$dir/SHA256SUMS.sig" >/dev/null 2>&1 \
      || die "SIGNATURE VERIFICATION FAILED for SHA256SUMS — aborting, nothing was installed."
    ok "Release signature verified (Ed25519)"
  else
    warn "Release signing key not configured in this installer: verifying SHA-256 only."
  fi
  download "$BASE/$ASSET" "$dir/$ASSET" || die "Could not download $ASSET"
  local expected actual
  expected="$(awk -v f="$ASSET" '$2==f || $2=="*"f {print $1}' "$dir/SHA256SUMS" | head -1)"
  [ -n "$expected" ] || die "$ASSET is not listed in SHA256SUMS"
  actual="$(sha256_of "$dir/$ASSET")"
  [ "$expected" = "$actual" ] || die "CHECKSUM MISMATCH for $ASSET (expected $expected, got $actual) — aborting."
  ok "$ASSET verified (SHA-256 $actual)"
}

install_runtime_deps() {
  step "Runtime dependencies"
  local pkgs=()
  case "$FAMILY" in
    debian) pkgs=(ffmpeg gstreamer1.0-plugins-good gstreamer1.0-libav gstreamer1.0-pipewire xdg-desktop-portal)
            [ "$METHOD" = appimage ] && pkgs+=(libfuse2t64) ;;
    fedora) pkgs=(gstreamer1-plugins-good pipewire-gstreamer xdg-desktop-portal)
            has ffmpeg || pkgs+=(ffmpeg-free)
            [ "$METHOD" = appimage ] && pkgs+=(fuse-libs) ;;
    suse)   pkgs=(ffmpeg gstreamer-plugins-good pipewire-gstreamer xdg-desktop-portal) ;;
    arch)   pkgs=(ffmpeg gst-plugins-good gst-plugin-pipewire gst-libav xdg-desktop-portal webkit2gtk-4.1 fuse2) ;;
    *)      warn "Unknown distribution: please make sure ffmpeg, GStreamer (good, libav, pipewire) and FUSE 2 are installed."; return ;;
  esac
  if [ "$METHOD" = deb ] || [ "$METHOD" = rpm ]; then
    ok "Handled by the package manager together with VINCI"
    [ "$FAMILY" = fedora ] && has rpm && ! rpm -q rpmfusion-free-release >/dev/null 2>&1 && \
      say "${D}Tip: for NVIDIA/Intel/AMD H.264/HEVC hardware encoding on Fedora, enable RPM Fusion and install 'ffmpeg' (see README).${R}"
    return
  fi
  if ! need_root; then
    warn "Cannot install dependencies without sudo. If VINCI fails to start, install: ${pkgs[*]}"
    return
  fi
  if ask "Install/update runtime dependencies (${pkgs[*]})?" Y; then
    case "$FAMILY" in
      debian) $SUDO apt-get update -qq && $SUDO apt-get install -y "${pkgs[@]}" || \
                { warn "retrying without libfuse2t64"; $SUDO apt-get install -y "${pkgs[@]/libfuse2t64/libfuse2}"; } ;;
      fedora) $SUDO dnf install -y "${pkgs[@]}" ;;
      suse)   $SUDO zypper --non-interactive install "${pkgs[@]}" ;;
      arch)   $SUDO pacman -S --needed --noconfirm "${pkgs[@]}" ;;
    esac
    ok "Dependencies installed"
  fi
}

install_package() {
  local file="$1"
  step "Installing VINCI"
  case "$METHOD" in
    deb)
      $SUDO apt-get install -y "$file" >/dev/null || { $SUDO dpkg -i "$file" || true; $SUDO apt-get -f install -y; }
      BIN="/usr/bin/vinci" ;;
    rpm)
      if has dnf; then $SUDO dnf install -y "$file"
      elif has zypper; then $SUDO zypper --non-interactive install --allow-unsigned-rpm "$file"
      else $SUDO rpm -Uvh "$file"; fi
      BIN="/usr/bin/vinci" ;;
    appimage)
      local dir="$HOME/.local/share/vinci" bindir="$HOME/.local/bin"
      mkdir -p "$dir" "$bindir" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/256x256/apps"
      install -m 0755 "$file" "$dir/VINCI.AppImage.new" && mv -f "$dir/VINCI.AppImage.new" "$dir/VINCI.AppImage"
      ln -sf "$dir/VINCI.AppImage" "$bindir/vinci"
      # Icon from inside the AppImage (no FUSE needed for extraction).
      ( cd "$(mktemp -d)" && "$dir/VINCI.AppImage" --appimage-extract '*.png' >/dev/null 2>&1 \
          && f="$(find squashfs-root -name '*.png' -size +2k | sort -r | head -1)" && [ -n "$f" ] \
          && cp "$f" "$HOME/.local/share/icons/hicolor/256x256/apps/vinci.png" ) || true
      cat >"$HOME/.local/share/applications/vinci.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=VINCI
GenericName=Game Clip Recorder
Comment=Capture Every Moment — instant replay for games
Exec=$dir/VINCI.AppImage %U
TryExec=$dir/VINCI.AppImage
Icon=vinci
Terminal=false
Categories=AudioVideo;Video;Recorder;Game;
Keywords=clip;replay;record;game;capture;
StartupWMClass=vinci
DESKTOP
      has update-desktop-database && update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true
      case ":$PATH:" in *":$bindir:"*) ;; *) warn "$bindir is not in your PATH (the menu entry works regardless)." ;; esac
      BIN="$dir/VINCI.AppImage" ;;
  esac
  ok "Installed ($METHOD)"
}

configure_permissions() {
  step "Permissions"
  # VA-API hardware encoding needs access to the DRM render node.
  local node=""
  for n in /dev/dri/renderD*; do [ -e "$n" ] && { node="$n"; break; }; done
  if [ -n "$node" ] && { [ ! -r "$node" ] || [ ! -w "$node" ]; }; then
    local grp; grp="$(stat -c %G "$node")"
    warn "Your user cannot access $node (group '$grp'): GPU encoding via VA-API will be unavailable."
    if need_root && ask "Add $(id -un) to the '$grp' group? (takes effect after re-login)" Y; then
      $SUDO usermod -aG "$grp" "$(id -un)" && ok "Added to '$grp' — log out and back in to apply"
    else
      say "Run later: sudo usermod -aG $grp $(id -un)"
    fi
  else
    ok "GPU device access OK${node:+ ($node)}"
  fi
  if [ "$SESSION" = wayland ]; then
    say "${D}Wayland: on first start your desktop asks which screen VINCI may capture (asked once).${R}"
  fi
}

check_runtime() {
  step "Checking the environment"
  local missing=0
  if has ffmpeg; then ok "ffmpeg $(ffmpeg -hide_banner -version 2>/dev/null | head -1 | awk '{print $3}')"
  elif [ "$METHOD" = appimage ]; then ok "ffmpeg (bundled in the AppImage)"
  else warn "ffmpeg not found — VINCI needs it to capture"; missing=1; fi
  if [ "$SESSION" = wayland ]; then
    if has gst-inspect-1.0 && gst-inspect-1.0 pipewiresrc >/dev/null 2>&1; then ok "GStreamer PipeWire source (Wayland capture)"
    else warn "GStreamer PipeWire plugin missing (gstreamer1.0-pipewire / gst-plugin-pipewire): Wayland capture will not work"; missing=1; fi
  fi
  if [ "$METHOD" = appimage ] && ! { ls /usr/lib*/libfuse.so.2* /usr/lib/*/libfuse.so.2* >/dev/null 2>&1; }; then
    warn "libfuse2 not found: AppImages need it (Ubuntu: libfuse2t64, Fedora: fuse-libs, Arch: fuse2)."
  fi
  if has nvidia-smi; then ok "NVIDIA GPU detected — NVENC will be validated on first start"; fi
  [ "$missing" = 0 ] || warn "Some optional components are missing; VINCI will explain what is unavailable in Settings → Diagnostics."
}

verify_install() {
  step "Verifying"
  [ -x "$BIN" ] || die "Installation failed: $BIN not found"
  ok "VINCI is installed: $BIN"
  say "Your clips go to ~/Videos/VINCI. Default hotkey: Alt+F10 (save last 30 s)."
  say "Uninstall any time: curl -fsSL <INSTALL_URL>/install.sh | bash -s -- --uninstall"
}

offer_launch() {
  [ "$NO_LAUNCH" = 1 ] && return
  [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] || return
  if ask "Launch VINCI now?" Y; then
    nohup "$BIN" >/dev/null 2>&1 &
    ok "VINCI is starting — look for it in your system tray."
  fi
}

uninstall() {
  step "Uninstalling VINCI"
  local removed=0
  if has dpkg && dpkg -s vinci >/dev/null 2>&1; then need_root || die "sudo required"; $SUDO apt-get remove -y vinci; removed=1; fi
  if has rpm && rpm -q vinci >/dev/null 2>&1; then
    need_root || die "sudo required"
    if has dnf; then $SUDO dnf remove -y vinci; elif has zypper; then $SUDO zypper --non-interactive remove vinci; else $SUDO rpm -e vinci; fi
    removed=1
  fi
  if [ -e "$HOME/.local/share/vinci/VINCI.AppImage" ]; then
    rm -f "$HOME/.local/share/vinci/VINCI.AppImage" "$HOME/.local/bin/vinci" "$HOME/.local/share/applications/vinci.desktop" "$HOME/.local/share/icons/hicolor/256x256/apps/vinci.png"
    removed=1
  fi
  rm -f "$HOME/.config/autostart/VINCI.desktop" "$HOME/.config/autostart/vinci.desktop" 2>/dev/null || true
  [ "$removed" = 1 ] && ok "VINCI removed" || warn "VINCI does not seem to be installed"
  if [ "$PURGE" = 1 ]; then
    rm -rf "$HOME/.config/vinci" "$HOME/.local/share/vinci" "$HOME/.cache/vinci"
    ok "Settings, library index and cache deleted"
  fi
  say "Your clips (~/Videos/VINCI) were not touched."
}

main "$@"
