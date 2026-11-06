# Releasing VINCI

## One-time setup

1. **Create the GitHub repository** and push this code.
2. **Replace the placeholders** (`USERNAME/vinci`, `<INSTALL_URL>`) — already done for
   `tfem1/vinci`; for a fork or a renamed repository run:
   ```bash
   scripts/configure-project.sh <owner>/<repo>
   ```
3. **Create the update-signing key** (Ed25519):
   ```bash
   cargo run -p vinci-update --bin vinci-release-sign -- keygen
   ```
   * Add the printed secret as the repository secret **`VINCI_UPDATE_SIGNING_KEY`**
     (Settings → Secrets and variables → Actions). Never commit it.
   * Embed the public key in the updater and the Linux installer:
     ```bash
     scripts/configure-project.sh <owner>/<repo> "" "<public key base64>"
     ```
   Without a key, releases still work (SHA-256 verification only) but in-app updates are
   disabled — VINCI refuses unsigned updates by design.
4. *(Optional)* **Custom install domain** (e.g. `vinci.app`): enable GitHub Pages with
   "GitHub Actions" as source; the `Install site` workflow publishes `install.sh` and
   `install.ps1` at the site root. Add the domain in Settings → Pages, then
   `scripts/configure-project.sh <owner>/<repo> https://<your-domain>` so the scripts
   advertise the short URL.
5. *(Optional)* **Windows code signing**: without it, SmartScreen warns on first launch
   (integrity is still guaranteed by SHA-256). Configure Tauri's
   `bundle.windows.signCommand` / certificate as described in the Tauri v2 documentation
   and store the certificate in repository secrets.

## Cutting a release

* Stable: Actions → **Bump version** → `0.2.0` (or locally `scripts/bump-version.sh 0.2.0`,
  commit, `git tag v0.2.0 && git push --tags`).
* Beta: same with `0.3.0-beta.1` — published as a GitHub *pre-release*; users on the beta
  channel (Settings → Updates, or `install.sh --beta`, `$env:VINCI_CHANNEL="beta"`) get it.

The **Release** workflow then:

1. builds Linux x86_64 + aarch64 (deb, rpm, AppImage with bundled FFmpeg 8.1) and Windows
   x64 (NSIS per-user setup + MSI, with bundled FFmpeg 8.1);
2. smoke-tests `install.sh` against the freshly built `.deb` (install, tamper rejection, uninstall);
3. renames bundles to stable names (`VINCI-linux-x86_64.deb`, `VINCI-windows-x64-setup.exe`, …);
4. writes `SHA256SUMS`, signs it (`SHA256SUMS.sig`, raw Ed25519) and produces the signed
   update manifest `vinci-update.json`;
5. publishes the GitHub release with the installers and `install.sh` / `install.ps1`.

Because asset names are stable, these URLs always point to the latest stable release:

```
https://github.com/<owner>/<repo>/releases/latest/download/install.sh
https://github.com/<owner>/<repo>/releases/latest/download/install.ps1
https://github.com/<owner>/<repo>/releases/latest/download/VINCI-windows-x64-setup.exe
```

## Verifying a release by hand

```bash
sha256sum -c SHA256SUMS --ignore-missing
openssl pkeyutl -verify -pubin -inkey release.pem -rawin -in SHA256SUMS -sigfile SHA256SUMS.sig
cargo run -p vinci-update --bin vinci-release-sign -- verify --pubkey <b64> --manifest vinci-update.json
```
