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

The install one-liners use `https://github.com/tfem1/vinci/releases/latest/download/…`,
which only resolves once a **stable** release exists — publish a tag without suffix first.

* Stable: Actions → **Bump version** → `0.2.0` (or locally `scripts/bump-version.sh 0.2.0`,
  commit, `git tag v0.2.0 && git push origin v0.2.0`). For the very first release the
  version in the repository is already `0.1.0`: `git tag v0.1.0 && git push origin v0.1.0`.
* Beta: same with `0.3.0-beta.1` — published as a GitHub *pre-release* (not served by
  `/releases/latest/`); users opt in with Settings → Updates → Beta.

The **Release** workflow (`.github/workflows/release.yml`) then, for linux-x86_64,
linux-arm64 and windows-x86_64:

1. builds `vinci` (`npx tauri build --no-bundle`) and `vinci-cli`;
2. packages the portable archives `vinci-linux-x86_64.tar.gz`, `vinci-linux-arm64.tar.gz`,
   `vinci-windows-x86_64.zip` (Windows includes FFmpeg 8.1);
3. smoke-tests `install.sh` against the Linux archive from a local mirror (install,
   re-install, tampered archive, missing asset, missing release);
4. builds the classic installers (deb, rpm, AppImage, NSIS, MSI) — optional, a failure
   there does not block the release;
5. publishes the release with the archives, `install.sh`, `install.ps1`, `uninstall.ps1`,
   `SHA256SUMS` and the classic installers; with the signing secret also
   `SHA256SUMS.sig` and the signed update manifest `vinci-update.json`.

macOS is not built: VINCI has no macOS capture backend.

## Verifying a release by hand

```bash
sha256sum -c SHA256SUMS --ignore-missing
openssl pkeyutl -verify -pubin -inkey release.pem -rawin -in SHA256SUMS -sigfile SHA256SUMS.sig
cargo run -p vinci-update --bin vinci-release-sign -- verify --pubkey <b64> --manifest vinci-update.json
```
