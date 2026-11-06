# VINCI installers

| File | Platform | Usage |
|---|---|---|
| `linux/install.sh` | Ubuntu, Debian, Mint, Pop!_OS, Fedora, openSUSE, Arch & derivatives | `curl -fsSL <INSTALL_URL>/install.sh \| bash` |
| `windows/install.ps1` | Windows 10 (1809+) / 11, x64 (ARM64 via emulation) | `irm <INSTALL_URL>/install.ps1 \| iex` |
| `windows/uninstall.ps1` | Windows | `irm <INSTALL_URL>/uninstall.ps1 \| iex` |

`<INSTALL_URL>` is `https://github.com/USERNAME/vinci/releases/latest/download` until a
custom domain is configured (see `docs/RELEASING.md`).

## Linux — `install.sh`

What it does, in order:

1. Detects the distribution (`/etc/os-release`), CPU architecture (x86_64 / aarch64) and session (X11 / Wayland).
2. Chooses the package: **.deb** (Debian family), **.rpm** (Fedora / openSUSE), **AppImage** (Arch & others, or `--user`).
3. Downloads `SHA256SUMS` (and verifies its Ed25519 signature when a release key is configured), then the package, and checks its **SHA-256**. Nothing runs before verification.
4. Installs runtime dependencies (FFmpeg, GStreamer good/libav/PipeWire, xdg-desktop-portal, FUSE 2 for AppImage) — the package manager resolves them for deb/rpm.
5. Installs VINCI (`apt` / `dnf` / `zypper`, or per-user AppImage in `~/.local/share/vinci` with a menu entry and icon — **no root needed**).
6. Checks permissions: access to `/dev/dri/renderD*` for VA-API (offers to add you to the `render`/`video` group).
7. Reports missing optional components, verifies the installation and offers to launch VINCI.

```bash
# options (with curl | bash, pass them after `bash -s --`)
curl -fsSL <INSTALL_URL>/install.sh | bash -s -- --beta
curl -fsSL <INSTALL_URL>/install.sh | bash -s -- --user          # AppImage, no sudo
curl -fsSL <INSTALL_URL>/install.sh | bash -s -- --version v0.2.0
curl -fsSL <INSTALL_URL>/install.sh | bash -s -- --uninstall     # keeps clips & settings
curl -fsSL <INSTALL_URL>/install.sh | bash -s -- --uninstall --purge
```

## Windows — `install.ps1`

1. Checks Windows version (build ≥ 17763; WGC needs 18362+), 64-bit, architecture.
2. Downloads `VINCI-windows-x64-setup.exe` (or the MSI) and `SHA256SUMS`, verifies **SHA-256**; checks the **Authenticode** signature when the release is code-signed.
3. Runs the per-user NSIS installer (`%LOCALAPPDATA%\VINCI`, **no admin rights**), silently with `VINCI_SILENT=1`.
4. Verifies the installation (uninstall registry entry + `vinci.exe` + bundled `ffmpeg.exe`).
5. Creates Start-menu and desktop shortcuts, optional autostart (`HKCU\…\Run`), offers to launch.

```powershell
$env:VINCI_SILENT = "1"; $env:VINCI_AUTOSTART = "1"; irm <INSTALL_URL>/install.ps1 | iex
$env:VINCI_CHANNEL = "beta"; irm <INSTALL_URL>/install.ps1 | iex
```

## Classic installers

Every release also ships the regular packages: `VINCI-windows-x64-setup.exe` (NSIS),
`VINCI-windows-x64.msi`, `VINCI-linux-<arch>.deb`, `.rpm` and `.AppImage`.

## Your data is never touched

Installers and updates only replace the application. Settings (`~/.config/vinci`,
`%APPDATA%\VINCI`), the library index (`~/.local/share/vinci`, `%LOCALAPPDATA%\VINCI`) and
your clips (`Videos/VINCI`) are kept. `--purge` deletes settings and the index, **never clips**.
