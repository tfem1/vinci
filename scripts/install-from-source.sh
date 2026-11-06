#!/usr/bin/env bash
# Build VINCI from this checkout and install it into ~/.local/bin (no release needed).
#
#   git clone https://github.com/tfem1/vinci && cd vinci && ./scripts/install-from-source.sh
#
# Needs: Rust (https://rustup.rs), Node.js 20+ with npm, and the WebKitGTK/GTK
# development packages (the script tells you the exact command if they are missing).
# VINCI_BIN_DIR=~/bin to install somewhere else.
set -euo pipefail
cd "$(dirname "$0")/.."

ok()   { printf '  \033[32m✔\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*" >&2; }
fail() { printf '\n  \033[31merror:\033[0m %s\n\n' "$*" >&2; exit 1; }
has()  { command -v "$1" >/dev/null 2>&1; }

[ "$(uname -s)" = Linux ] || fail "this script builds the Linux version. On Windows run scripts\\install-from-source.ps1; macOS is not supported."

fam="other"
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  fam="$(. /etc/os-release; case " ${ID:-} ${ID_LIKE:-} " in
    *" debian "*|*" ubuntu "*) echo debian ;; *" fedora "*|*" rhel "*) echo fedora ;;
    *" arch "*) echo arch ;; *" suse "*|*" opensuse"*) echo suse ;; *) echo other ;; esac)"
fi
case "$fam" in
  debian) build_deps="sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev ffmpeg gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav gstreamer1.0-pipewire" ;;
  fedora) build_deps="sudo dnf install gcc pkgconf-pkg-config webkit2gtk4.1-devel libayatana-appindicator-gtk3-devel librsvg2-devel libxdo-devel openssl-devel ffmpeg-free gstreamer1-plugins-good pipewire-gstreamer" ;;
  arch)   build_deps="sudo pacman -S --needed base-devel webkit2gtk-4.1 libayatana-appindicator librsvg xdotool openssl ffmpeg gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav gst-plugin-pipewire" ;;
  suse)   build_deps="sudo zypper install -t pattern devel_basis && sudo zypper install webkit2gtk3-soup2-devel libayatana-appindicator3-devel librsvg-devel libxdo-devel libopenssl-devel ffmpeg" ;;
  *)      build_deps="(install a C toolchain, pkg-config, WebKitGTK 4.1, libayatana-appindicator3, librsvg, libxdo and OpenSSL development packages)" ;;
esac

# --- prerequisites ----------------------------------------------------------
has cargo || fail "Rust is not installed. Install it from https://rustup.rs, open a new terminal and run this script again."
has npm && has node || fail "Node.js 20+ and npm are required: https://nodejs.org (or your distribution's nodejs/npm packages)."
node -e 'process.exit(+process.versions.node.split(".")[0] >= 20 ? 0 : 1)' || fail "Node.js $(node --version) is too old; VINCI needs Node.js 20 or newer."
has pkg-config || fail "pkg-config is missing. Install the build dependencies:
    $build_deps"
missing=""
for lib in webkit2gtk-4.1 libsoup-3.0 javascriptcoregtk-4.1; do
  pkg-config --exists "$lib" || missing="$missing $lib"
done
pkg-config --exists ayatana-appindicator3-0.1 || pkg-config --exists appindicator3-0.1 || missing="$missing ayatana-appindicator3"
[ -z "$missing" ] || fail "missing development libraries:$missing
  Install the build dependencies:
    $build_deps"
ok "Prerequisites: $(cargo --version | cut -d' ' -f1-2), node $(node --version)"

# --- build -------------------------------------------------------------------
# `tauri build` (not plain `cargo build`) is required: it embeds the web UI in the binary.
npm ci --no-audit --no-fund
npx tauri build --no-bundle
cargo build --release --locked -p vinci-cli
ok "Built target/release/vinci and target/release/vinci-cli"

# --- install -------------------------------------------------------------------
bin_dir="${VINCI_BIN_DIR:-$HOME/.local/bin}"
mkdir -p "$bin_dir"
for b in vinci vinci-cli; do
  install -m 0755 "target/release/$b" "$bin_dir/.$b.new"
  mv -f "$bin_dir/.$b.new" "$bin_dir/$b"
done
share="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$share/applications" "$share/icons/hicolor/256x256/apps"
install -m 0644 src-tauri/icons/128x128@2x.png "$share/icons/hicolor/256x256/apps/vinci.png"
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
# KDE Plasma: lets KWin hand VINCI the screen directly (no screen-sharing prompt).
X-KDE-Wayland-Interfaces=zkde_screencast_unstable_v1
DESKTOP
# Same permission for the command-line engine (hidden from the menu).
cat >"$share/applications/vinci-cli.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=VINCI (command line)
Exec=$bin_dir/vinci-cli
Terminal=true
NoDisplay=true
X-KDE-Wayland-Interfaces=zkde_screencast_unstable_v1
DESKTOP
# Refresh KDE's application database so KWin sees the entries right away.
for k in kbuildsycoca6 kbuildsycoca5; do
  if command -v "$k" >/dev/null 2>&1; then "$k" >/dev/null 2>&1 || true; break; fi
done
ok "Installed $bin_dir/vinci and $bin_dir/vinci-cli (+ applications menu entry)"

# Wayland screen capture runs a GStreamer pipeline (crates/vinci-capture/src/deps.rs):
# report the missing elements with the exact install command for this distribution.
gst_pkg() { # gst_pkg <family> <element>
  case "$1:$2" in
    arch:gst-launch-1.0) echo gstreamer ;;
    arch:pipewiresrc) echo gst-plugin-pipewire ;;
    arch:videoconvert|arch:videoscale|arch:videorate) echo gst-plugins-base ;;
    arch:matroskamux) echo gst-plugins-good ;;
    arch:x264enc) echo gst-plugins-ugly ;;
    arch:h264parse) echo gst-plugins-bad ;;
    arch:avenc_aac) echo gst-libav ;;
    debian:gst-launch-1.0) echo gstreamer1.0-tools ;;
    debian:pipewiresrc) echo gstreamer1.0-pipewire ;;
    debian:videoconvert|debian:videoscale|debian:videorate) echo gstreamer1.0-plugins-base ;;
    debian:matroskamux) echo gstreamer1.0-plugins-good ;;
    debian:x264enc) echo gstreamer1.0-plugins-ugly ;;
    debian:h264parse) echo gstreamer1.0-plugins-bad ;;
    debian:avenc_aac) echo gstreamer1.0-libav ;;
    fedora:gst-launch-1.0) echo gstreamer1 ;;
    fedora:pipewiresrc) echo pipewire-gstreamer ;;
    fedora:videoconvert|fedora:videoscale|fedora:videorate) echo gstreamer1-plugins-base ;;
    fedora:matroskamux) echo gstreamer1-plugins-good ;;
    fedora:x264enc) echo gstreamer1-plugins-ugly ;;
    fedora:h264parse) echo gstreamer1-plugins-bad-free ;;
    fedora:avenc_aac) echo gstreamer1-plugin-libav ;;
    suse:gst-launch-1.0) echo gstreamer-utils ;;
    suse:pipewiresrc) echo gstreamer-plugin-pipewire ;;
    suse:videoconvert|suse:videoscale|suse:videorate) echo gstreamer-plugins-base ;;
    suse:matroskamux) echo gstreamer-plugins-good ;;
    suse:x264enc) echo gstreamer-plugins-ugly ;;
    suse:h264parse) echo gstreamer-plugins-bad ;;
    suse:avenc_aac) echo gstreamer-plugins-libav ;;
  esac
}
check_wayland_capture() { # check_wayland_capture <family>
  [ "${XDG_SESSION_TYPE:-}" = wayland ] || [ -n "${WAYLAND_DISPLAY:-}" ] || return 0
  local missing="" el pkgs="" p
  if has gst-inspect-1.0; then
    for el in pipewiresrc videoconvert videoscale videorate matroskamux x264enc h264parse avenc_aac; do
      gst-inspect-1.0 "$el" >/dev/null 2>&1 || missing="$missing $el"
    done
  else
    missing=" gst-launch-1.0 pipewiresrc videoconvert matroskamux x264enc h264parse avenc_aac"
  fi
  [ -n "$missing" ] || return 0
  for el in $missing; do
    p="$(gst_pkg "$1" "$el")"
    case " $pkgs " in *" $p "*) ;; *) [ -n "$p" ] && pkgs="$pkgs $p" ;; esac
  done
  warn "Wayland screen capture needs GStreamer plugins that are missing:$missing"
  case "$1" in
    arch)   warn "install them:  sudo pacman -S --needed$pkgs" ;;
    debian) warn "install them:  sudo apt install$pkgs" ;;
    fedora) warn "install them:  sudo dnf install$pkgs" ;;
    suse)   warn "install them:  sudo zypper install$pkgs" ;;
    *)      warn "install the GStreamer base, good, bad, ugly, libav and PipeWire plugins with your package manager." ;;
  esac
}
has ffmpeg || warn "FFmpeg is not installed — VINCI needs it to capture:  $build_deps"
check_wayland_capture "$fam"
path_hint() { # path_hint <bin_dir> — the command for the user's shell
  case "$(basename "${SHELL:-bash}")" in
    fish) printf 'fish_add_path %s' "$1" ;;
    zsh)  printf "echo 'export PATH=\"%s:\$PATH\"' >> ~/.zshrc" "$1" ;;
    *)    printf "echo 'export PATH=\"%s:\$PATH\"' >> ~/.bashrc" "$1" ;;
  esac
}
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) warn "$bin_dir is not in your PATH, so 'vinci' and 'vinci-cli' are not found by name yet. Run once, then open a new terminal:  $(path_hint "$bin_dir")" ;;
esac
printf '\n  Done. Run: vinci   (or vinci-cli detect)\n\n'
