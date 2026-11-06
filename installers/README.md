# VINCI installers

| Path | Command | Installs to | Needs a release |
|---|---|---|---|
| A. Linux (x86_64, arm64) | `curl -fsSL https://github.com/tfem1/vinci/releases/latest/download/install.sh \| bash` | `~/.local/bin` | yes |
| B. Windows 10/11 (x64, ARM64 via emulation) | `irm https://github.com/tfem1/vinci/releases/latest/download/install.ps1 \| iex` | `%LOCALAPPDATA%\vinci` + user PATH | yes |
| C. From source | `git clone https://github.com/tfem1/vinci && cd vinci && ./scripts/install-from-source.sh` (Windows: `scripts\install-from-source.ps1`) | same as A / B | **no** |

`/releases/latest/download/` resolves only once a stable release (tag `vX.Y.Z`, no
suffix) has been published by the Release workflow. Before that, A and B stop with
"no published release found" and point to C.

## `linux/install.sh`

1. Detects the OS (Linux only — macOS gets an explicit "not supported" message) and CPU (`x86_64` / `arm64`).
2. Downloads `SHA256SUMS` — a 404 here means "no release published yet".
3. If a release signing key is configured in the script, verifies `SHA256SUMS.sig` (Ed25519, OpenSSL).
4. Downloads `vinci-linux-<arch>.tar.gz` (a 404 means "no build for this machine in the release") and checks its SHA-256.
5. Installs `vinci` and `vinci-cli` into `~/.local/bin` (or `$VINCI_BIN_DIR`) with an atomic replace, plus an applications-menu entry and icon.
6. Reports missing runtime libraries (`ldd`) and FFmpeg with the exact `apt` / `dnf` / `pacman` / `zypper` command — it never runs `sudo` itself.
7. Warns if `~/.local/bin` is not in `PATH`.

Options: `VINCI_VERSION`, `VINCI_BIN_DIR`, `VINCI_BASE_URL` (mirror).
Test locally against a mirror, without GitHub or root: `scripts/package-archive.sh linux-x86_64 && scripts/test-install-local.sh`.

## `windows/install.ps1`

1. Refuses to run in Constrained Language mode (AppLocker/WDAC) with manual-install instructions.
2. Detects `AMD64` / `ARM64` (ARM64 → native build if a release ever ships one, else the x64 build under Windows 11 emulation; Windows 10 on ARM is refused with an explanation).
3. Downloads `SHA256SUMS` and `vinci-windows-x86_64.zip` through the system proxy with the user's Windows credentials; HTTP 407 and "cannot connect" produce explicit proxy instructions; 404 → "no release" / "no asset" messages.
4. Verifies the SHA-256, stops a running VINCI from that folder, extracts to `%LOCALAPPDATA%\vinci` (only program files are replaced; `data\` and `cache\` are left alone).
5. Adds the folder to the **user** PATH once (registry value kept as `REG_EXPAND_SZ`, other entries untouched), notifies Explorer, updates the current session.
6. Start-menu shortcut; warns if the WebView2 runtime is missing.

Execution policy: `irm … | iex` is not subject to it; a saved copy can be run with
`powershell -ExecutionPolicy Bypass -File .\install.ps1`.

## `windows/uninstall.ps1`

Removes the program files, the PATH entry and shortcuts (and an NSIS/MSI install if present).
`$env:VINCI_PURGE = "1"` also deletes settings and the library index. Clips are never deleted.
