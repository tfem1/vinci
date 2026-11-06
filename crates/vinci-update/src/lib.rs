//! # vinci-update — update manager
//!
//! Security model:
//! * Each release publishes `vinci-update.json` = `{ "manifest": "<json>", "signature": "<b64>" }`.
//!   The signature is Ed25519 over the exact manifest bytes, made in CI with a key that
//!   never leaves GitHub Secrets. The public key is compiled into VINCI.
//! * The manifest lists every installer with its SHA-256 and size; downloads are
//!   verified before anything is executed.
//! * Without a configured public key VINCI refuses to install updates (check only).
//! * User data (settings, library, clips) lives outside the install directory and is
//!   never touched by an update.

pub mod kind;

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub use kind::{detect_install_kind, InstallKind};

const B64: base64::engine::general_purpose::GeneralPurpose = base64::engine::general_purpose::STANDARD;
pub const MANIFEST_ASSET: &str = "vinci-update.json";

/// Release public key: `VINCI_UPDATE_PUBKEY` at build time, else `update-public-key.txt`.
pub fn embedded_public_key() -> Option<String> {
    let k = option_env!("VINCI_UPDATE_PUBKEY").unwrap_or(include_str!("../update-public-key.txt")).trim();
    (!k.is_empty() && k != "UNCONFIGURED").then(|| k.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("updates are not configured for this build (no release public key)")]
    NotConfigured,
    #[error("network error: {0}")]
    Network(String),
    #[error("invalid update manifest: {0}")]
    Manifest(String),
    #[error("update signature verification FAILED — the update was rejected")]
    BadSignature,
    #[error("downloaded file does not match its SHA-256 (expected {expected}, got {actual})")]
    HashMismatch { expected: String, actual: String },
    #[error("refusing insecure URL {0}")]
    InsecureUrl(String),
    #[error("no package for this platform ({0}) in the release")]
    NoAsset(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub channel: Channel,
    #[serde(default)]
    pub pub_date: String,
    #[serde(default)]
    pub notes: String,
    /// Keyed by `<os>-<arch>-<kind>`: `windows-x86_64-nsis`, `linux-x86_64-appimage`, …
    pub assets: BTreeMap<String, Asset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedManifest {
    pub manifest: String,
    pub signature: String,
}

pub fn keygen() -> (String, String) {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).expect("OS randomness");
    let sk = SigningKey::from_bytes(&seed);
    (B64.encode(seed), B64.encode(sk.verifying_key().to_bytes()))
}

fn signing_key(secret_b64: &str) -> Result<SigningKey, UpdateError> {
    let seed: [u8; 32] = B64
        .decode(secret_b64.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| UpdateError::Manifest("bad signing key".into()))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn verifying_key(public_b64: &str) -> Result<VerifyingKey, UpdateError> {
    let bytes: [u8; 32] =
        B64.decode(public_b64.trim()).ok().and_then(|v| v.try_into().ok()).ok_or_else(|| UpdateError::Manifest("bad public key".into()))?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| UpdateError::Manifest("bad public key".into()))
}

pub fn sign_bytes(data: &[u8], secret_b64: &str) -> Result<[u8; 64], UpdateError> {
    Ok(signing_key(secret_b64)?.sign(data).to_bytes())
}

pub fn verify_bytes(data: &[u8], sig: &[u8], public_b64: &str) -> Result<(), UpdateError> {
    let sig: [u8; 64] = sig.try_into().map_err(|_| UpdateError::BadSignature)?;
    verifying_key(public_b64)?.verify(data, &Signature::from_bytes(&sig)).map_err(|_| UpdateError::BadSignature)
}

/// PEM (SubjectPublicKeyInfo) form of a public key, for `openssl pkeyutl -verify`.
pub fn public_key_pem(public_b64: &str) -> Result<String, UpdateError> {
    let raw = verifying_key(public_b64)?.to_bytes();
    let mut der = vec![0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
    der.extend_from_slice(&raw);
    Ok(format!("-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n", B64.encode(der)))
}

pub fn sign_manifest(m: &Manifest, secret_b64: &str) -> Result<SignedManifest, UpdateError> {
    let json = serde_json::to_string(m).map_err(|e| UpdateError::Manifest(e.to_string()))?;
    let sig = sign_bytes(json.as_bytes(), secret_b64)?;
    Ok(SignedManifest { manifest: json, signature: B64.encode(sig) })
}

pub fn verify_manifest(s: &SignedManifest, public_b64: &str) -> Result<Manifest, UpdateError> {
    let sig = B64.decode(s.signature.trim()).map_err(|_| UpdateError::BadSignature)?;
    verify_bytes(s.manifest.as_bytes(), &sig, public_b64)?;
    serde_json::from_str(&s.manifest).map_err(|e| UpdateError::Manifest(e.to_string()))
}

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Platform key for this build and install kind, e.g. `linux-x86_64-deb`.
pub fn platform_key(kind: &InstallKind) -> String {
    format!("{}-{}-{}", std::env::consts::OS, std::env::consts::ARCH, kind.asset_suffix())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AvailableUpdate {
    pub version: String,
    pub current: String,
    pub channel: Channel,
    pub notes: String,
    pub pub_date: String,
    pub asset_key: String,
    pub asset: Option<Asset>,
}

/// Is `candidate` newer than `current` for this channel?
pub fn is_newer(candidate: &str, current: &str, channel: Channel) -> bool {
    let (Ok(c), Ok(cur)) =
        (semver::Version::parse(candidate.trim_start_matches('v')), semver::Version::parse(current.trim_start_matches('v')))
    else {
        return false;
    };
    if channel == Channel::Stable && !c.pre.is_empty() {
        return false;
    }
    c > cur
}

pub struct Updater {
    pub repo: String,
    pub current_version: String,
    pub channel: Channel,
    pub public_key: Option<String>,
    pub kind: InstallKind,
    /// Overrides discovery (self-hosting / tests): direct URL of `vinci-update.json`.
    pub manifest_url: Option<String>,
    pub allow_http: bool,
    user_agent: String,
}

impl Updater {
    pub fn new(repo: &str, current_version: &str, channel: Channel) -> Self {
        Self {
            repo: repo.to_string(),
            current_version: current_version.to_string(),
            channel,
            public_key: std::env::var("VINCI_UPDATE_PUBKEY_OVERRIDE").ok().or_else(embedded_public_key),
            kind: detect_install_kind(),
            manifest_url: std::env::var("VINCI_UPDATE_MANIFEST_URL").ok(),
            allow_http: std::env::var_os("VINCI_UPDATE_ALLOW_HTTP").is_some(),
            user_agent: format!("VINCI/{current_version} (+https://github.com/{repo})"),
        }
    }

    fn agent(&self, url: &str) -> Result<ureq::Agent, UpdateError> {
        if !url.starts_with("https://") && !(self.allow_http && url.starts_with("http://")) {
            return Err(UpdateError::InsecureUrl(url.to_string()));
        }
        let local = url.contains("://127.0.0.1") || url.contains("://localhost");
        Ok(ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout_read(std::time::Duration::from_secs(60))
            .try_proxy_from_env(!local)
            .user_agent(&self.user_agent)
            .build())
    }

    fn get_json(&self, url: &str) -> Result<serde_json::Value, UpdateError> {
        let resp = self
            .agent(url)?
            .get(url)
            .set("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| UpdateError::Network(e.to_string()))?;
        resp.into_json().map_err(|e| UpdateError::Network(e.to_string()))
    }

    fn manifest_location(&self) -> Result<String, UpdateError> {
        if let Some(u) = &self.manifest_url {
            return Ok(u.clone());
        }
        let api = format!("https://api.github.com/repos/{}", self.repo);
        let release = match self.channel {
            Channel::Stable => self.get_json(&format!("{api}/releases/latest"))?,
            Channel::Beta => {
                let list = self.get_json(&format!("{api}/releases?per_page=15"))?;
                list.as_array()
                    .and_then(|a| a.iter().find(|r| !r["draft"].as_bool().unwrap_or(false)).cloned())
                    .ok_or_else(|| UpdateError::Manifest("no release published".into()))?
            }
        };
        release["assets"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"] == MANIFEST_ASSET))
            .and_then(|x| x["browser_download_url"].as_str())
            .map(String::from)
            .ok_or_else(|| UpdateError::Manifest(format!("latest release has no {MANIFEST_ASSET}")))
    }

    /// Check for a newer, correctly signed release.
    pub fn check(&self) -> Result<Option<AvailableUpdate>, UpdateError> {
        let key = self.public_key.clone().ok_or(UpdateError::NotConfigured)?;
        let url = self.manifest_location()?;
        let resp = self.agent(&url)?.get(&url).call().map_err(|e| UpdateError::Network(e.to_string()))?;
        let signed: SignedManifest = resp.into_json().map_err(|e| UpdateError::Manifest(e.to_string()))?;
        let m = verify_manifest(&signed, &key)?;
        if !is_newer(&m.version, &self.current_version, self.channel) {
            return Ok(None);
        }
        let asset_key = platform_key(&self.kind);
        Ok(Some(AvailableUpdate {
            version: m.version.clone(),
            current: self.current_version.clone(),
            channel: m.channel,
            notes: m.notes.clone(),
            pub_date: m.pub_date.clone(),
            asset: m.assets.get(&asset_key).cloned(),
            asset_key,
        }))
    }

    /// Download and verify the installer for `update` into `dir`.
    pub fn download(&self, update: &AvailableUpdate, dir: &Path, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf, UpdateError> {
        let asset = update.asset.as_ref().ok_or_else(|| UpdateError::NoAsset(update.asset_key.clone()))?;
        std::fs::create_dir_all(dir)?;
        let name = asset.url.rsplit('/').next().filter(|n| !n.is_empty() && !n.contains("..")).unwrap_or("vinci-update.bin");
        let dest = dir.join(name);
        let part = dir.join(format!("{name}.part"));
        let resp = self.agent(&asset.url)?.get(&asset.url).call().map_err(|e| UpdateError::Network(e.to_string()))?;
        let mut reader = resp.into_reader();
        let mut f = std::fs::File::create(&part)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut total = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(|e| UpdateError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            f.write_all(&buf[..n])?;
            total += n as u64;
            progress(total, asset.size);
            if total > asset.size.saturating_mul(2).max(1 << 20) {
                let _ = std::fs::remove_file(&part);
                return Err(UpdateError::Manifest("download larger than announced".into()));
            }
        }
        f.sync_all()?;
        drop(f);
        let actual = hex(&h.finalize());
        if !actual.eq_ignore_ascii_case(asset.sha256.trim()) {
            let _ = std::fs::remove_file(&part);
            return Err(UpdateError::HashMismatch { expected: asset.sha256.clone(), actual });
        }
        std::fs::rename(&part, &dest)?;
        Ok(dest)
    }

    /// Install a verified package. See [`kind::apply`].
    pub fn apply(&self, file: &Path) -> Result<kind::ApplyOutcome, UpdateError> {
        kind::apply(&self.kind, file)
    }

    /// Download + verify + apply, except for portable (archive) installs, which are
    /// updated by re-running the one-line installer (nothing to download here).
    pub fn install(&self, update: &AvailableUpdate, dir: &Path, progress: impl FnMut(u64, u64)) -> Result<kind::ApplyOutcome, UpdateError> {
        if let InstallKind::Archive { .. } = self.kind {
            return Ok(kind::ApplyOutcome::Manual { command: kind::installer_command(&self.repo) });
        }
        let file = self.download(update, dir, progress)?;
        self.apply(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn manifest(version: &str, url: &str, sha: &str, size: u64) -> Manifest {
        let mut assets = BTreeMap::new();
        assets.insert(platform_key(&InstallKind::Source), Asset { url: url.into(), sha256: sha.into(), size });
        Manifest { version: version.into(), channel: Channel::Stable, pub_date: "2026-11-04".into(), notes: "notes".into(), assets }
    }

    #[test]
    fn sign_verify_and_tamper() {
        let (sk, pk) = keygen();
        let m = manifest("1.2.0", "https://x/y", "00", 1);
        let s = sign_manifest(&m, &sk).unwrap();
        assert_eq!(verify_manifest(&s, &pk).unwrap(), m);
        let mut t = s.clone();
        t.manifest = t.manifest.replace("1.2.0", "9.9.9");
        assert!(matches!(verify_manifest(&t, &pk), Err(UpdateError::BadSignature)));
        let (_, other) = keygen();
        assert!(matches!(verify_manifest(&s, &other), Err(UpdateError::BadSignature)));
        assert!(public_key_pem(&pk).unwrap().contains("BEGIN PUBLIC KEY"));
    }

    #[test]
    fn version_rules() {
        assert!(is_newer("0.2.0", "0.1.9", Channel::Stable));
        assert!(!is_newer("0.2.0-beta.1", "0.1.0", Channel::Stable));
        assert!(is_newer("0.2.0-beta.1", "0.1.0", Channel::Beta));
        assert!(!is_newer("0.1.0", "0.1.0", Channel::Beta));
        assert!(!is_newer("garbage", "0.1.0", Channel::Beta));
    }

    /// Minimal HTTP server serving fixed files, for end-to-end update tests.
    fn serve(l: TcpListener, files: Vec<(String, Vec<u8>)>) {
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let resp = match files.iter().find(|(p, _)| *p == path) {
                    Some((_, b)) => {
                        [format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()).into_bytes(), b.clone()]
                            .concat()
                    }
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                };
                let _ = s.write_all(&resp);
            }
        });
    }

    #[test]
    fn end_to_end_check_download_verify() {
        let (sk, pk) = keygen();
        let payload = b"pretend installer".to_vec();
        let sha = hex(&Sha256::digest(&payload));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let m = manifest("9.0.0", &format!("{base}/pkg.bin"), &sha, payload.len() as u64);
        let signed = serde_json::to_vec(&sign_manifest(&m, &sk).unwrap()).unwrap();
        let bad_sha = manifest("9.0.0", &format!("{base}/pkg.bin"), &"0".repeat(64), payload.len() as u64);
        let bad_signed = serde_json::to_vec(&sign_manifest(&bad_sha, &sk).unwrap()).unwrap();
        serve(
            listener,
            vec![
                ("/vinci-update.json".to_string(), signed),
                ("/bad.json".to_string(), bad_signed),
                ("/pkg.bin".to_string(), payload.clone()),
            ],
        );

        let mut up = Updater::new("tfem1/vinci", "0.1.0", Channel::Stable);
        up.public_key = Some(pk.clone());
        up.kind = InstallKind::Source;
        up.allow_http = true;
        up.manifest_url = Some(format!("{base}/vinci-update.json"));
        let avail = up.check().unwrap().expect("update available");
        assert_eq!(avail.version, "9.0.0");
        let dir = tempfile::tempdir().unwrap();
        let file = up.download(&avail, dir.path(), |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), payload);

        // Wrong hash → rejected, nothing left behind.
        up.manifest_url = Some(format!("{base}/bad.json"));
        let avail = up.check().unwrap().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        assert!(matches!(up.download(&avail, dir2.path(), |_, _| {}), Err(UpdateError::HashMismatch { .. })));
        assert_eq!(std::fs::read_dir(dir2.path()).unwrap().count(), 0);

        // Wrong key → rejected.
        up.public_key = Some(keygen().1);
        up.manifest_url = Some(format!("{base}/vinci-update.json"));
        assert!(matches!(up.check(), Err(UpdateError::BadSignature)));

        // Plain HTTP is refused unless explicitly allowed.
        up.allow_http = false;
        assert!(matches!(up.check(), Err(UpdateError::InsecureUrl(_))));
        // No key → not configured.
        up.public_key = None;
        assert!(matches!(up.check(), Err(UpdateError::NotConfigured)));
    }
}
