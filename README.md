<div align="center">

<img src="docs/assets/icon-1024.png" width="96" alt="VINCI logo" />

# VINCI

**Capture Every Moment.**

A lightweight instant-replay recorder for gamers — Windows & Linux.
GPU-encoded replay buffer, one-key clips, no re-encoding, no account, no cloud.

[Install](#install) · [Features](#features) · [Development](#development) · [Troubleshooting](#troubleshooting) · [Français](README.fr.md)

</div>

---

## Install

Three supported ways — all per-user, no administrator rights:

| | Command | Installs to |
|---|---|---|
| **A. Linux** (x86_64, arm64) | `curl -fsSL https://github.com/tfem1/vinci/releases/latest/download/install.sh \| bash` | `~/.local/bin` |
| **B. Windows 10/11** (x64; ARM64 via emulation) | `irm https://github.com/tfem1/vinci/releases/latest/download/install.ps1 \| iex` | `%LOCALAPPDATA%\vinci` + user `PATH` |
| **C. From source** (Linux, Windows) | `git clone https://github.com/tfem1/vinci && cd vinci && ./scripts/install-from-source.sh` | same as A / B |

> **A and B install the latest published release.** Until the first release exists these
> URLs return 404 — the scripts then say so and point you to **C**, which needs no release.
> Pre-releases (`v…-beta.N` tags) are not served by `/releases/latest/`.
> macOS is not supported (VINCI has no macOS capture backend).

### A. Linux

```bash
curl -fsSL https://github.com/tfem1/vinci/releases/latest/download/install.sh | bash
```

Detects the CPU (x86_64 / arm64), downloads `vinci-linux-<arch>.tar.gz` and `SHA256SUMS`,
verifies the checksum, installs `vinci` (app) and `vinci-cli` into `~/.local/bin` with an
applications-menu entry, then reports any missing runtime library (WebKitGTK, FFmpeg…)
with the exact install command for your distribution. If the release or the archive for
your machine is missing it stops with a clear message. Nothing needs `sudo`.

To read the script before running it:
`curl -fsSLO https://github.com/tfem1/vinci/releases/latest/download/install.sh && less install.sh && bash install.sh`.
Options: `VINCI_VERSION=v0.2.0` (specific release), `VINCI_BIN_DIR=~/bin` (other folder) —
e.g. `curl -fsSL …/install.sh | VINCI_VERSION=v0.2.0 bash`.
Uninstall: `rm ~/.local/bin/vinci ~/.local/bin/vinci-cli ~/.local/share/applications/vinci.desktop`.

### B. Windows

```powershell
irm https://github.com/tfem1/vinci/releases/latest/download/install.ps1 | iex
```

Detects x64 / ARM64 (ARM64 uses the x64 build under Windows 11 emulation), downloads
`vinci-windows-x86_64.zip` and `SHA256SUMS`, verifies the checksum, extracts `vinci.exe`,
`vinci-cli.exe` and the bundled FFmpeg to `%LOCALAPPDATA%\vinci`, adds that folder to your
**user** `PATH` (only once — re-running never duplicates it), creates a Start-menu shortcut
and checks for the WebView2 runtime.

* **Proxy:** the system proxy is used with your Windows credentials. If the proxy still
  refuses (HTTP 407) or github.com is unreachable, the script prints what to configure.
* **Execution policy:** the `irm … | iex` form is not subject to it. If you saved the file
  and PowerShell says *"running scripts is disabled on this system"*, run it for that one
  process only: `powershell -ExecutionPolicy Bypass -File .\install.ps1`. A locked-down
  PowerShell (Constrained Language mode via AppLocker/WDAC) is detected and explained.
* To read it first: `irm https://github.com/tfem1/vinci/releases/latest/download/install.ps1 -OutFile install.ps1; notepad install.ps1`.
* Uninstall: `irm https://github.com/tfem1/vinci/releases/latest/download/uninstall.ps1 | iex` (clips are kept).

### C. From source (works today, no release needed)

Needs Rust (https://rustup.rs), Node.js 20+ and the platform build tools — each script
checks them first and prints the exact install command for anything missing.

```bash
# Linux — builds and installs into ~/.local/bin
git clone https://github.com/tfem1/vinci && cd vinci && ./scripts/install-from-source.sh
```

```powershell
# Windows — builds and installs into %LOCALAPPDATA%\vinci (+ user PATH)
git clone https://github.com/tfem1/vinci; cd vinci; powershell -ExecutionPolicy Bypass -File .\scripts\install-from-source.ps1
```

The build command alone (output: `target/release/vinci` and `target/release/vinci-cli`):

```bash
npm ci && npx tauri build --no-bundle && cargo build --release -p vinci-cli
```

(On Windows run `scripts\fetch-ffmpeg.ps1` first — the app bundles FFmpeg there.) Use
`npx tauri build`, not a plain `cargo build`: only the Tauri CLI embeds the web UI; a plain
cargo build produces a development binary that expects the Vite dev server.

### Classic installers

When they build, releases also carry `VINCI-windows-x64-setup.exe`, `.msi`, `.deb`,
`.rpm` and `.AppImage` on the [Releases page](https://github.com/tfem1/vinci/releases).

Default hotkey: **Alt+F10** saves the last 30 seconds. Clips go to `Videos/VINCI`.

## Screenshots

| Dashboard | Library |
|---|---|
| ![Dashboard](docs/assets/screenshots/dashboard.png) | ![Library](docs/assets/screenshots/library.png) |
| **Capture** | **Player** |
| ![Capture](docs/assets/screenshots/capture.png) | ![Player](docs/assets/screenshots/player.png) |

*Real screenshots of VINCI running on Linux (virtual X11 desktop, CPU encoding — the
"x264 (CPU)" notice is VINCI honestly reporting that this test machine has no GPU encoder).*

## Why VINCI

| | |
|---|---|
| **LIGHTWEIGHT** | The engine is a ~12 MB native process using < 1 % CPU; capture and encoding run on the GPU's video engine. The interface process is closed when VINCI sits in the tray. 8 MB Linux packages. |
| **QUALITY** | HEVC / AV1 hardware encoding with quality-targeted rate control (CQ / ICQ / CRF) — sharp motion at a fraction of typical H.264 bitrates. Three profiles: Quality, Balanced, Compact. |
| **INSTANT** | Save the last 30 s, 1, 2, 3 or 5 minutes (up to 10) with one key. Clips are cut from the already-encoded buffer — no re-encoding, saved in about a second. |
| **SIMPLE** | One command to install, no account, no cloud, no telemetry. |

## Features

**Instant replay**
- Rolling buffer from 15 s to 10 min, in RAM or in a fixed-size disk ring (≈ 0 RAM), chosen automatically.
- Save 30 s / 1 m / 2 m / 3 m / 5 m from the dashboard, the tray, a hotkey per length, or `vinci --save-clip 60`.
- The clip includes the moment you pressed the key; cut on keyframes, never re-encoded.
- Crash-safe: atomic writes, the encoder is supervised and restarted, already buffered footage survives a restart, corrupted temporary files are cleaned at startup.

**Capture** — native APIs, no injection (anti-cheat safe)
- Windows: **Windows.Graphics.Capture** (monitor or game window) and **DXGI Desktop Duplication** (exclusive fullscreen); frames stay on the GPU (D3D11 → NVENC/AMF zero-copy, QSV via D3D11 interop).
- Linux: **X11** (XShm) and **Wayland** through **xdg-desktop-portal + PipeWire** (screen chosen once, remembered).
- Game audio + microphone (WASAPI loopback on Windows, PulseAudio/PipeWire on Linux), per-source volume.

**Encoding & compression**
- H.264, HEVC, AV1 on **NVIDIA NVENC, AMD AMF, Intel Quick Sync, VA-API**; every encoder is validated with a real test encode on your PC; automatic fallback (and CPU x264 as last resort).
- 720p / 1080p / 1440p / 4K (never upscales), 30 / 60 / 120 / 144 fps.
- Compress any clip in place: "Discord 10 MB", "50 MB", "Compact AV1/HEVC", 1080p downscale — still one file.
- Lossless trim.

**Library**
- One physical file per clip in your folder (optionally one sub-folder per game); SQLite only stores metadata.
- Thumbnails, built-in player with seeking, search, game filter, favourites, sort (date, size, length, name), rename (renames the file), move, delete, show in folder. Files added or removed outside VINCI are picked up.

**And also**
- Game detection (≈ 85 games + Steam/Epic/Riot/GOG libraries, Proton-aware): clips are named and filed per game; optional "only buffer while a game runs".
- Discreet in-game overlay ("Clip saved · 30 s"), configurable corner and opacity.
- Live CPU / RAM / GPU / encoder usage.
- Resource saver mode, capture priority, RAM budget.
- Signed automatic updates (stable / beta) that never touch your clips or settings.
- Headless CLI (`vinci-cli`) running the same engine.

## How it works

```
 Native capture (WGC / DDA / X11 / PipeWire)
        │  frames stay on the GPU
        ▼
 Hardware encoder (NVENC / AMF / QSV / VA-API) ──► fragmented MP4 stream
        │
        ▼
 VINCI replay buffer — GOP-aligned ring (RAM or disk), last N seconds
        │  hotkey
        ▼
 Copy the selected fragments → remux (-c copy, faststart) → atomic rename → library
```

Rust engine (independent crates) + Tauri 2 shell + React/TypeScript UI. FFmpeg/GStreamer run
as supervised helper processes. Full design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Compatibility

| | Status |
|---|---|
| Windows 11, Windows 10 1903+ (x64) | Supported — WGC + DDA, NVENC / AMF / QSV. 1809 works with DDA only. ARM64 (Windows 11) runs the x64 build under emulation. |
| Linux x86_64 / arm64 with glibc ≥ 2.35: Ubuntu 22.04+, Debian 12+, Mint 21+, Pop!_OS, Fedora 36+, openSUSE Tumbleweed, Arch | Supported (X11 and Wayland). Needs WebKitGTK 4.1 and FFmpeg from your distribution — `install.sh` tells you if they are missing. On Fedora use RPM Fusion's `ffmpeg` for H.264/HEVC hardware encoding. |
| macOS | Not supported — VINCI has no macOS capture backend. |
| GPUs | NVIDIA (GTX 900+; AV1 needs RTX 40), AMD (VCN; AV1 needs RX 7000), Intel (Quick Sync / VA-API; AV1 needs Arc / Core Ultra). Without a GPU encoder VINCI falls back to CPU x264. |

What was verified, and how: [docs/TESTING.md](docs/TESTING.md).

## Configuration

Everything is in the app (**Capture** and **Settings**). Files, if you need them:

| | Linux | Windows |
|---|---|---|
| Settings | `~/.config/vinci/settings.json` | `%APPDATA%\VINCI\config\settings.json` |
| Library index, logs | `~/.local/share/vinci/` | `%LOCALAPPDATA%\VINCI\data\` |
| Thumbnails, replay ring | `~/.cache/vinci/` | `%LOCALAPPDATA%\VINCI\cache\` |
| Clips | `~/Videos/VINCI` | `Videos\VINCI` |

Command line (also usable from desktop shortcuts or compositor key bindings):

```bash
vinci --save-clip          # save the default length
vinci --save-clip 120      # save the last 2 minutes
vinci --toggle-replay      # start/stop the buffer
vinci --minimized          # start in the tray
```

**Wayland:** VINCI registers its hotkeys through the XDG GlobalShortcuts portal (KDE Plasma,
GNOME 48+, Hyprland…). On desktops without it, bind `vinci --save-clip` to a key in your
desktop's keyboard settings.

## Development

To install from source, use [path C](#c-from-source-works-today-no-release-needed). To work on VINCI:

```bash
git clone https://github.com/tfem1/vinci.git
cd vinci
```

### Linux (Ubuntu/Debian)

```bash
sudo apt install -y build-essential curl libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev libssl-dev patchelf ffmpeg gstreamer1.0-plugins-good \
  gstreamer1.0-libav gstreamer1.0-pipewire
```

Fedora: `sudo dnf install webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel libxdo-devel openssl-devel ffmpeg-free gstreamer1-plugins-good pipewire-gstreamer`
· Arch: `sudo pacman -S --needed webkit2gtk-4.1 libappindicator-gtk3 librsvg xdotool openssl ffmpeg gst-plugins-good gst-libav gst-plugin-pipewire base-devel`

Install Rust (https://rustup.rs) and Node.js 20+, then:

```bash
npm ci
npm run tauri dev                  # development (hot reload)
npx tauri build --no-bundle        # production binary: target/release/vinci
npm run tauri build                # + .deb, .rpm, .AppImage in target/release/bundle/
cargo test --workspace             # 70+ tests, incl. real FFmpeg encodes
scripts/e2e-linux.sh               # end-to-end on a virtual X11 desktop (needs Xvfb, pulseaudio, xdotool)
cargo run -p vinci-cli -- detect   # headless: hardware & encoder report
```

### Windows

Install [Rust](https://rustup.rs) (MSVC toolchain), [Node.js 20+](https://nodejs.org),
Visual Studio Build Tools ("Desktop development with C++") and WebView2 (preinstalled on Windows 11). Then in PowerShell:

```powershell
npm ci
powershell -ExecutionPolicy Bypass -File scripts\fetch-ffmpeg.ps1   # bundled FFmpeg 8.1 (WGC support)
npm run tauri dev
npm run tauri build      # NSIS setup.exe + MSI in target\release\bundle\
cargo test --workspace
```

## Troubleshooting

| Problem | Fix |
|---|---|
| Installer: "no published release found" (404) | No release has been published yet. Install from source ([path C](#c-from-source-works-today-no-release-needed)). |
| Installer: "this release does not contain vinci-…" | No build exists for your OS/CPU in that release (builds: Linux x86_64/arm64, Windows x64). Use path C. |
| `vinci: command not found` after installing | Linux: add `~/.local/bin` to your `PATH` (the installer prints the line). Windows: open a **new** terminal (PATH changes apply to new terminals). |
| Linux installer: "missing system libraries" | Run the `sudo apt/dnf/pacman …` command it printed (WebKitGTK 4.1, appindicator, FFmpeg, GStreamer). |
| Windows: "proxy requires authentication (HTTP 407)" / cannot reach github.com | Run the two `[System.Net.WebRequest]::DefaultWebProxy…` lines the installer prints (with your proxy address), then retry — or download the zip from the Releases page. |
| Windows: "running scripts is disabled on this system" | Use the `irm … \| iex` one-liner, or `powershell -ExecutionPolicy Bypass -File .\install.ps1`. |
| "Using x264 (CPU) — encoder not available" | Your GPU encoder failed its test encode. Update the GPU driver; on Linux check `/dev/dri/renderD*` access (`sudo usermod -aG render,video $USER`, then re-login) and install the VA-API driver (`intel-media-va-driver`, `mesa-va-drivers`). **Settings → Diagnostics** shows each encoder's error. |
| Wayland: "Wayland capture needs GStreamer with the PipeWire plugin" | Install `gstreamer1.0-pipewire` (Debian/Ubuntu), `pipewire-gstreamer` (Fedora) or `gst-plugin-pipewire` (Arch), plus `xdg-desktop-portal` for your desktop. |
| Hotkey does nothing | Another app may own it (shown in **Settings → Hotkeys**) — pick another. On Wayland without the GlobalShortcuts portal, bind `vinci --save-clip` in your desktop settings. |
| Fullscreen game not captured on Windows | Switch **Capture method** to *Desktop Duplication* (captures exclusive fullscreen), or play borderless. |
| No sound in clips | Check **Capture → Audio** (output device). Linux: `pactl list short sources` must show a `.monitor` source. Adjust **Settings → Advanced → Audio offset** if out of sync. |
| Clip doesn't play in VINCI | Use "Open in player". HEVC/AV1 playback inside the app depends on the system web view (Windows needs the HEVC/AV1 Video Extensions). |
| AppImage won't start | Install FUSE 2 (`libfuse2t64` on Ubuntu 24.04, `fuse-libs` on Fedora, `fuse2` on Arch). |
| Anything else | **Settings → Advanced → Open logs folder** (`vinci.log`) and **Diagnostics → Copy report**. |

## Privacy & security

No account, no telemetry, no cloud. The only network request is the optional daily
update check, and updates are installed only if their Ed25519 signature and SHA-256 match.
The replay buffer lives in RAM or in a temporary file deleted on exit. VINCI never injects
code into games.

## Project layout

```
crates/            Rust engine — one crate per module
  vinci-hw/        hardware & capability detection
  vinci-capture/   capture backends (WGC, DDA, X11, PipeWire), WASAPI
  vinci-encoding/  encoders, profiles, probing, compression
  vinci-replay/    fMP4 ring buffer & zero-re-encode export
  vinci-clips/     SQLite library
  vinci-storage/   paths, atomic persistence
  vinci-games/     game detection
  vinci-hotkeys/   global hotkeys
  vinci-update/    signed updates + release signing tool
  vinci-engine/    orchestration (used by the app and the CLI)
  vinci-cli/       headless CLI
src-tauri/         desktop shell (Tauri 2): windows, tray, overlay, media server
src/               React + TypeScript UI (components, pages, hooks, styles)
installers/        install.sh, install.ps1, uninstall.ps1
scripts/           release, packaging, test and setup scripts
docs/              architecture, testing, releasing
.github/workflows/ CI, release, version bump, install site
```

## Contributing & releases

Pull requests welcome — run `cargo fmt`, `cargo clippy --workspace -- -D warnings`,
`cargo test --workspace` and `npm test`. Maintainers: see [docs/RELEASING.md](docs/RELEASING.md)
(signing keys, stable/beta channels, one-click version bump).

## License

[MIT](LICENSE). The Windows installer and the AppImage bundle FFmpeg (GPL v3 build) as a
separate program — see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
