//! Release tooling used by CI (and maintainers):
//!
//! ```text
//! vinci-release-sign keygen
//! vinci-release-sign manifest --version 0.2.0 --channel stable --base-url URL \
//!       --asset windows-x86_64-nsis=dist/VINCI_0.2.0_x64-setup.exe ... --out vinci-update.json
//! vinci-release-sign sign-file --in SHA256SUMS --out SHA256SUMS.sig
//! vinci-release-sign verify --pubkey <b64> --manifest vinci-update.json
//! ```
//! The signing key is read from the `VINCI_UPDATE_SIGNING_KEY` environment variable.

use std::collections::BTreeMap;
use std::path::PathBuf;
use vinci_update::*;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

fn key() -> String {
    std::env::var("VINCI_UPDATE_SIGNING_KEY").unwrap_or_else(|_| die("VINCI_UPDATE_SIGNING_KEY is not set"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("keygen") => {
            let (sk, pk) = keygen();
            println!("VINCI_UPDATE_SIGNING_KEY (GitHub secret, keep private):\n{sk}\n");
            println!("Public key (crates/vinci-update/update-public-key.txt and installers):\n{pk}\n");
            println!("Public key PEM (installers/linux/install.sh):\n{}", public_key_pem(&pk).unwrap());
        }
        Some("pubkey-pem") => {
            let pk = arg(&args, "--pubkey").unwrap_or_else(|| die("--pubkey required"));
            print!("{}", public_key_pem(&pk).unwrap_or_else(|e| die(&e.to_string())));
        }
        Some("manifest") => {
            let version = arg(&args, "--version").unwrap_or_else(|| die("--version required"));
            let channel = match arg(&args, "--channel").as_deref() {
                Some("beta") => Channel::Beta,
                _ => Channel::Stable,
            };
            let base = arg(&args, "--base-url").unwrap_or_else(|| die("--base-url required"));
            let out = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| MANIFEST_ASSET.into()));
            let mut assets = BTreeMap::new();
            for (i, a) in args.iter().enumerate() {
                if a == "--asset" {
                    let spec = args.get(i + 1).unwrap_or_else(|| die("--asset needs key=path"));
                    let (k, p) = spec.split_once('=').unwrap_or_else(|| die("--asset needs key=path"));
                    let path = PathBuf::from(p);
                    let name = path.file_name().unwrap().to_string_lossy().into_owned();
                    let size = std::fs::metadata(&path).unwrap_or_else(|e| die(&format!("{p}: {e}"))).len();
                    let sha256 = sha256_file(&path).unwrap_or_else(|e| die(&e.to_string()));
                    assets.insert(k.to_string(), Asset { url: format!("{}/{name}", base.trim_end_matches('/')), sha256, size });
                }
            }
            let m = Manifest { version, channel, pub_date: arg(&args, "--date").unwrap_or_default(), notes: arg(&args, "--notes").unwrap_or_default(), assets };
            let signed = sign_manifest(&m, &key()).unwrap_or_else(|e| die(&e.to_string()));
            std::fs::write(&out, serde_json::to_vec_pretty(&signed).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
            eprintln!("wrote {} ({} assets)", out.display(), m.assets.len());
        }
        Some("sign-file") => {
            let input = arg(&args, "--in").unwrap_or_else(|| die("--in required"));
            let out = arg(&args, "--out").unwrap_or_else(|| format!("{input}.sig"));
            let data = std::fs::read(&input).unwrap_or_else(|e| die(&e.to_string()));
            let sig = sign_bytes(&data, &key()).unwrap_or_else(|e| die(&e.to_string()));
            std::fs::write(&out, sig).unwrap_or_else(|e| die(&e.to_string()));
            eprintln!("wrote {out} (raw Ed25519, verify with: openssl pkeyutl -verify -pubin -inkey pub.pem -rawin -in {input} -sigfile {out})");
        }
        Some("verify") => {
            let pk = arg(&args, "--pubkey").unwrap_or_else(|| die("--pubkey required"));
            let file = arg(&args, "--manifest").unwrap_or_else(|| die("--manifest required"));
            let s: SignedManifest = serde_json::from_slice(&std::fs::read(&file).unwrap_or_else(|e| die(&e.to_string()))).unwrap_or_else(|e| die(&e.to_string()));
            match verify_manifest(&s, &pk) {
                Ok(m) => println!("OK: VINCI {} ({:?}), {} assets", m.version, m.channel, m.assets.len()),
                Err(e) => die(&e.to_string()),
            }
        }
        _ => {
            eprintln!("usage: vinci-release-sign <keygen|pubkey-pem|manifest|sign-file|verify> [options]");
            std::process::exit(1);
        }
    }
}
